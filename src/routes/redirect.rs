//! The forwarding handler, and the one gate in front of the Cloudflare Access credentials.

use crate::converter::{ActixToReqwestConverter, ReqwestToActixConverter};
use crate::data::WebHookData;
use actix_web::http::Method;
use actix_web::web::Query;
use actix_web::{HttpRequest, HttpResponse, web};
use reqwest::{Body, Client, RequestBuilder, Url};
use std::collections::HashMap;

/// Registers the five methods that can be forwarded against every path.
///
/// The pattern is `{tail:.*}`, so this claims whatever `health_check` did not; register it last.
/// A method outside these five is refused by actix before [`redirect`] sees it.
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

/// Forwards one request to `webhook.target_base` with the Cloudflare Access token attached.
///
/// Answers `404` when the allow list does not admit the path and method together, and `400` for
/// every failure after that: a body that cannot be read, a query string that cannot be parsed, a
/// URL that will not join, an upstream that refuses, a response that cannot be relayed. The
/// upstream's own status is passed through untouched when it does answer.
async fn redirect(
    mut payload: web::Payload,
    request: HttpRequest,
    path: web::Path<String>,
    web_hook_data: web::Data<WebHookData>,
) -> core::result::Result<HttpResponse, actix_web::Error> {
    let method = request.method();

    // The allow list is the only gate in front of the Cloudflare Access credentials, so both
    // verdicts are logged at the same level: at the default level every request leaves exactly
    // one line saying whether it was forwarded or turned away. Passing the allow list is not by
    // itself proof that the request reached the target, so the failures below say so as well.
    if !web_hook_data.is_allowed_path(&path, method) {
        info!(
            "Rejected {} request for path '{}': no allowed path matches this method, responding 404",
            method, path
        );
        return Ok(HttpResponse::NotFound().finish());
    }

    // Craft target url
    let target_url = web_hook_data.get_target_url(path.as_str()).map_err(|e| {
        error!(
            "Not forwarding allowed {} request for path '{}': failed to join the target URL: {}",
            method, path, e
        );
        actix_web::error::ErrorBadRequest(e)
    })?;

    // Convert body
    let body = ActixToReqwestConverter::convert_body(&mut payload)
        .await
        .map_err(|e| {
            error!(
                "Not forwarding allowed {} request for path '{}': failed to read the body: {}",
                method, path, e
            );
            e
        })?;

    // Convert headers
    let mut target_headers: reqwest::header::HeaderMap =
        ActixToReqwestConverter::convert_headers(request.headers(), 3);

    // Add Cloudflare Access headers
    target_headers.append("CF-Access-Client-Id", web_hook_data.access_id().clone());
    target_headers.append(
        "CF-Access-Client-Secret",
        web_hook_data.access_secret().clone(),
    );

    // Continue this hop's trace at the target, so one webhook delivery reads as a single trace
    // rather than as two unrelated ones. Empty, and so a no-op, unless Sentry is on.
    propagate_trace(&mut target_headers);

    // Query params
    let params =
        Query::<HashMap<String, String>>::from_query(request.query_string()).map_err(|e| {
            error!(
                "Not forwarding allowed {} request for path '{}': failed to parse the query string: {}",
                method, path, e
            );
            e
        })?;

    // Redirect request
    info!(
        "Forwarding {} request for path '{}' to {}",
        method, path, target_url
    );
    let response = ReqwestBuilder::new(
        web_hook_data.client(),
        target_url,
        body,
        target_headers,
        params.0,
        method,
    )
    .build()
    .map_err(|e| {
        error!(
            "Failed to build the forwarded {} request for path '{}': {}",
            method, path, e
        );
        actix_web::error::ErrorBadRequest(e)
    })?
    .send()
    .await
    .map_err(|e| {
        error!(
            "Failed to send the forwarded {} request for path '{}' to the target: {}",
            method, path, e
        );
        actix_web::error::ErrorBadRequest(e)
    })?;

    // Parse reqwest response
    let converted_response = ReqwestToActixConverter::convert_response(response)
        .await
        .map_err(|e| {
            error!(
                "Forwarded {} request for path '{}', but the target response could not be relayed: {}",
                method, path, e
            );
            e
        })?;

    debug!(
        "Forwarded {} request for path '{}', target responded {}",
        method,
        path,
        converted_response.status()
    );
    Ok(converted_response)
}

/// Overwrite the trace-continuation headers of the forwarded request with this hop's.
///
/// Overwrite rather than append: the inbound value is the *caller's* claim about the trace, and
/// the span the target should continue is the one the middleware opened here. Appending would
/// leave the target a header with two values, which it reads as malformed.
fn propagate_trace(headers: &mut reqwest::header::HeaderMap) {
    for (name, value) in crate::telemetry::trace_headers() {
        let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(&value),
        ) else {
            continue;
        };
        headers.insert(name, value);
    }
}

/// One forwarded request, assembled from the inbound one.
///
/// The method decides what comes with it. All five carry the headers and the query string; only
/// `POST`, `PUT` and `PATCH` carry the body, so a `GET` or `DELETE` that arrived with one has it
/// dropped here.
struct ReqwestBuilder<'a> {
    client: &'a Client,
    url: Url,
    body: Body,
    headers: reqwest::header::HeaderMap,
    params: HashMap<String, String>,

    method: &'a Method,
    include_body: bool,
    include_params: bool,
}

impl<'a> ReqwestBuilder<'a> {
    /// Collects the parts. [`build`](ReqwestBuilder::build) decides which of them the method
    /// actually carries.
    pub fn new(
        client: &'a Client,
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

    /// Assembles the request, attaching the body and the query string the method calls for.
    ///
    /// # Errors
    /// Returns [`Error::InvalidRoute`](crate::error::Error::InvalidRoute) for a method outside the
    /// five [`get_config`] registers, which no routed request can reach.
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
    use reqwest::Client;
    use secrecy::SecretString;
    use std::collections::HashSet;
    use wiremock::{Mock, ResponseTemplate};

    const RETURN_STRING: &str = "Success!";

    #[derive(Getters)]
    #[getset(get = "pub")]
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

            let client = Client::new();
            let web_hook_data = WebHookData::new(
                client,
                target,
                allowed_paths,
                &SecretString::new(Box::from("access-id")),
                &SecretString::new(Box::from("access-secret")),
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
                .app_data(test_app.web_hook_data().clone())
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
                .app_data(test_app.web_hook_data().clone())
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
                .app_data(test_app.web_hook_data().clone())
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
