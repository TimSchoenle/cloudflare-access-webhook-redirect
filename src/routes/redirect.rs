use crate::converter::{ActixToReqwestConverter, ReqwestToActixConverter};
use crate::data::WebHookData;
use actix_web::http::Method;
use actix_web::web::Query;
use actix_web::{HttpRequest, HttpResponse, web};
use reqwest::{Body, Url};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use std::collections::HashMap;

pub fn get_config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("{tail:.*}")
            .route(web::get().to(redirect))
            .route(web::post().to(redirect))
            .route(web::put().to(redirect))
            .route(web::patch().to(redirect))
            .route(web::delete().to(redirect)),
    );
}

async fn redirect(
    mut payload: web::Payload,
    request: HttpRequest,
    path: web::Path<String>,
    web_hook_data: web::Data<WebHookData>,
) -> core::result::Result<HttpResponse, actix_web::Error> {
    // Only allow specific paths
    info!("Received {} request for path: {}", request.method(), path);
    if !web_hook_data.is_allowed_path(&path, request.method()) {
        debug!("Path not allowed: {}", path);
        return Ok(HttpResponse::NotFound().finish());
    }

    // Craft target url
    let target_url = web_hook_data.get_target_url(path.as_str()).map_err(|e| {
        error!("Failed to join URL: {}", e);
        actix_web::error::ErrorBadRequest(e)
    })?;

    // Convert body
    let body = ActixToReqwestConverter::convert_body(&mut payload).await?;

    // Convert headers
    let mut target_headers: reqwest::header::HeaderMap =
        ActixToReqwestConverter::convert_headers(request.headers(), 2);

    // Add Cloudflare Access headers
    target_headers.append("CF-Access-Client-Id", web_hook_data.access_id.clone());
    target_headers.append(
        "CF-Access-Client-Secret",
        web_hook_data.access_secret.clone(),
    );

    // Query params
    let params = Query::<HashMap<String, String>>::from_query(request.query_string())?;

    // Redirect request
    let response = ReqwestBuilder::new(
        &web_hook_data.client,
        target_url,
        body,
        target_headers,
        params.0,
        request.method(),
    )
    .build()
    .map_err(|e| {
        error!("Failed to build request: {}", e);
        actix_web::error::ErrorBadRequest(e)
    })?
    .send()
    .await
    .map_err(|e| {
        error!("Failed to send request: {}", e);
        actix_web::error::ErrorBadRequest(e)
    })?;

    // Parse reqwest response
    let converted_response = ReqwestToActixConverter::convert_response(response).await?;

    debug!("Return response with code {}", converted_response.status());
    Ok(converted_response)
}

struct ReqwestBuilder<'a> {
    client: &'a ClientWithMiddleware,
    url: Url,
    body: Body,
    headers: reqwest::header::HeaderMap,
    params: HashMap<String, String>,

    method: &'a Method,
    include_body: bool,
    include_params: bool,
}

impl<'a> ReqwestBuilder<'a> {
    pub fn new(
        client: &'a ClientWithMiddleware,
        url: Url,
        body: Body,
        headers: reqwest::header::HeaderMap,
        params: HashMap<String, String>,
        method: &'a Method,
    ) -> ReqwestBuilder<'a> {
        ReqwestBuilder {
            client,
            url,
            body,
            headers,
            method,
            params,
            include_body: false,
            include_params: false,
        }
    }

    fn include_body(&mut self) {
        self.include_body = true;
    }

    fn include_params(&mut self) {
        self.include_params = true;
    }

    pub fn build(mut self) -> crate::Result<RequestBuilder> {
        let mut request = match *self.method {
            Method::GET => {
                self.include_params();
                Ok(self.client.get(self.url))
            }
            Method::POST => {
                self.include_body();
                self.include_params();
                Ok(self.client.post(self.url))
            }
            Method::PUT => {
                self.include_body();
                self.include_params();
                Ok(self.client.put(self.url))
            }
            Method::PATCH => {
                self.include_body();
                self.include_params();
                Ok(self.client.patch(self.url))
            }
            Method::DELETE => {
                self.include_params();
                Ok(self.client.delete(self.url))
            }
            _ => Err(crate::Error::invalid_route(self.method)),
        }?;

        // Headers are always required for Cloudflare Access
        request = request.headers(self.headers);

        if self.include_body {
            request = request.body(self.body);
        }

        if self.include_params {
            request = request.query(&self.params);
        }

        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AllowedMethod;
    use actix_web::{App, test};
    use reqwest_middleware::ClientBuilder;
    use secrecy::SecretString;
    use std::collections::HashSet;
    use wiremock::{Mock, ResponseTemplate};

    const RETURN_STRING: &str = "Success!";

    pub struct TestApp {
        _mock_server: wiremock::MockServer,
        web_hook_data: web::Data<WebHookData>,
    }

    impl TestApp {
        pub async fn new(
            mock_method: &str,
            mock_path: &str,
            allowed_method: &str,
            allowed_path: &str,
        ) -> Self {
            let mock_server = wiremock::MockServer::start().await;
            Mock::given(wiremock::matchers::method(mock_method))
                .and(wiremock::matchers::path(mock_path))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(RETURN_STRING)
                        .insert_header("Test", "123"),
                )
                .expect(1)
                .mount(&mock_server)
                .await;

            let target = Url::parse(mock_server.uri().as_str()).unwrap();

            let mut paths = HashMap::new();

            let mut methods: HashSet<AllowedMethod> = HashSet::new();
            methods.insert((&allowed_method.to_string()).try_into().unwrap());
            paths.insert(allowed_path.to_string(), methods);

            let allowed_paths = paths.try_into().unwrap();

            let client = ClientBuilder::new(reqwest::Client::new()).build();
            let web_hook_data = WebHookData::new(
                client,
                target,
                allowed_paths,
                SecretString::new(Box::from("access-id")),
                SecretString::new(Box::from("access-secret")),
            )
            .unwrap();

            let web_hook_data = web::Data::new(web_hook_data);
            Self {
                _mock_server: mock_server,
                web_hook_data,
            }
        }
    }

    #[actix_web::test]
    async fn test_redirect_get() {
        let test_app = TestApp::new("GET", "test", "GET", "test").await;
        let app = test::init_service(
            App::new()
                .app_data(test_app.web_hook_data.clone())
                .configure(get_config),
        )
        .await;

        // Valid request
        let req = test::TestRequest::get().uri("/test").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = resp.into_body();
        let bytes = actix_web::body::to_bytes(body).await;
        assert_eq!(
            bytes.unwrap(),
            web::Bytes::from_static(RETURN_STRING.as_ref())
        );

        // Invalid request
        let req = test::TestRequest::get().uri("/test/d").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_redirect_all() {
        let test_app = TestApp::new("PUT", "test", "ALL", "test").await;
        let app = test::init_service(
            App::new()
                .app_data(test_app.web_hook_data.clone())
                .configure(get_config),
        )
        .await;

        // Valid request
        let req = test::TestRequest::put().uri("/test").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = resp.into_body();
        let bytes = actix_web::body::to_bytes(body).await;
        assert_eq!(
            bytes.unwrap(),
            web::Bytes::from_static(RETURN_STRING.as_ref())
        );

        // Invalid request - Allowed request, but not supported by the mock server
        let req = test::TestRequest::get().uri("/test").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(!resp.status().is_success());
    }

    #[actix_web::test]
    async fn test_redirect_regex() {
        let test_app = TestApp::new("PUT", "test/10090", "ALL", r"test/\d*").await;
        let app = test::init_service(
            App::new()
                .app_data(test_app.web_hook_data.clone())
                .configure(get_config),
        )
        .await;

        // Valid request
        let req = test::TestRequest::put().uri("/test/10090").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = resp.into_body();
        let bytes = actix_web::body::to_bytes(body).await;
        assert_eq!(
            bytes.unwrap(),
            web::Bytes::from_static(RETURN_STRING.as_ref())
        );

        // Invalid request -- Allowed request, but not supported by the mock server
        let req = test::TestRequest::put().uri("/test/9").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(!resp.status().is_success());

        // Invalid request
        let req = test::TestRequest::get().uri("/test/d").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);

        let req = test::TestRequest::get().uri("/test/90d").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }
}
