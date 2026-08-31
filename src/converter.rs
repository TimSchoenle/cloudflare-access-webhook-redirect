//! Translation between actix's HTTP types and reqwest's.
//!
//! Four things, which is everything the proxy moves across: a request body, a request header map,
//! a status code and a whole response. Nothing here decides what is forwarded.

use thiserror::Error;
use tokio_stream::StreamExt;

/// The actix-to-reqwest direction.
///
/// Never constructed; both conversions are associated functions.
pub struct ActixToReqwestConverter {}

/// A [`Result`] carrying the translation's own [`ConverterError`].
pub type ConverterResult<T> = anyhow::Result<T, ConverterError>;

impl ActixToReqwestConverter {
    fn is_valid_header_name(name: &str) -> bool {
        trace!("Checking for valid header name: {}", name);
        !matches!(name, "host")
    }

    /// Reads `payload` to the end and hands it over as a reqwest body.
    ///
    /// The whole body is held in memory before the forwarded request is built, so the largest
    /// request this proxy can carry is bounded by what the process can hold.
    ///
    /// # Errors
    /// Returns [`ConverterError::Payload`] if the client stops sending or the connection drops
    /// before the body is complete.
    pub async fn convert_body(
        payload: &mut actix_web::web::Payload,
    ) -> ConverterResult<reqwest::Body> {
        let mut bytes = actix_web::web::BytesMut::new();
        while let Some(item) = payload.next().await {
            let item = item?;
            bytes.extend_from_slice(&item);
        }

        let body = reqwest::Body::from(bytes.freeze());
        Ok(body)
    }

    /// Copies `headers` across, dropping `host` and leaving room for `additional_headers` more.
    ///
    /// `host` is dropped because reqwest sets it from the target URL, and forwarding the proxy's
    /// own would hand the upstream a name it does not answer to. A header whose name or value
    /// reqwest refuses is dropped without a word, so a forwarded request can arrive with fewer
    /// headers than it left with.
    #[must_use]
    pub fn convert_headers(
        headers: &actix_web::http::header::HeaderMap,
        additional_headers: usize,
    ) -> reqwest::header::HeaderMap {
        let mut target_headers: reqwest::header::HeaderMap =
            reqwest::header::HeaderMap::with_capacity(headers.capacity() + additional_headers);
        headers
            .iter()
            .filter(|(key, _)| ActixToReqwestConverter::is_valid_header_name(key.as_str()))
            .for_each(|(key, value)| {
                if let Ok(value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
                    && let Ok(key) =
                        reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes())
                {
                    target_headers.append(key, value);
                }
            });

        target_headers
    }
}

/// The reqwest-to-actix direction.
pub struct ReqwestToActixConverter {}

impl ReqwestToActixConverter {
    /// Maps a reqwest status onto actix's.
    ///
    /// # Errors
    /// Returns [`ConverterError::InvalidStatusCode`] outside 100 to 999. Both crates hold their
    /// status types to that range on construction, so a code reqwest actually parsed cannot land
    /// here.
    pub fn convert_status_code(
        status_code: reqwest::StatusCode,
    ) -> ConverterResult<actix_web::http::StatusCode> {
        actix_web::http::StatusCode::from_u16(status_code.as_u16())
            .map_err(|_| ConverterError::invalid_status_code(status_code))
    }

    /// Reads the response body to the end and rebuilds it under the same status code.
    ///
    /// The upstream's headers are not carried over. A client of this proxy sees the status and the
    /// body, and nothing the protected service set alongside them.
    ///
    /// # Errors
    /// Returns [`ConverterError::ReqwestError`] if the upstream connection drops before the body
    /// is complete, and [`ConverterError::InvalidStatusCode`] for a status actix will not take,
    /// which [`convert_status_code`](ReqwestToActixConverter::convert_status_code) cannot in
    /// practice produce.
    pub async fn convert_response(
        response: reqwest::Response,
    ) -> ConverterResult<actix_web::HttpResponse> {
        let status_code = ReqwestToActixConverter::convert_status_code(response.status())?;
        let body = response.bytes().await?;

        let response = actix_web::HttpResponse::build(status_code).body(body);
        Ok(response)
    }
}

/// A request or response that could not be moved between the two HTTP stacks.
#[derive(Error, Debug)]
pub enum ConverterError {
    /// The inbound body ended early, or the client's connection dropped while it was being read.
    #[error("Payload Error")]
    Payload(#[from] actix_web::error::PayloadError),
    /// The upstream answered with a status code outside the range actix accepts.
    #[error("Invalid Status Code")]
    InvalidStatusCode(String),
    /// The forwarded request failed, or its response body did not arrive complete.
    #[error("Reqwest Error")]
    ReqwestError(#[from] reqwest::Error),
}

impl ConverterError {
    fn invalid_status_code(status_code: reqwest::StatusCode) -> Self {
        ConverterError::InvalidStatusCode(format!("Invalid status code: {}", status_code.as_u16()))
    }
}

/// Every translation failure becomes a `400`, the upstream's included.
impl From<ConverterError> for actix_web::Error {
    fn from(e: ConverterError) -> Self {
        actix_web::error::ErrorBadRequest(e)
    }
}

#[cfg(test)]
mod tests_actix_to_reqwest_converter {
    use actix_web::FromRequest;
    use actix_web::http::header::{HeaderName, HeaderValue};
    use std::collections::HashMap;

    fn convert_headers(values: HashMap<String, String>) -> actix_web::http::header::HeaderMap {
        let mut header_map = actix_web::http::header::HeaderMap::new();

        for (key, value) in values {
            let key = HeaderName::from_bytes(key.as_bytes()).unwrap();
            let value = HeaderValue::from_bytes(value.as_bytes()).unwrap();

            header_map.append(key, value);
        }

        header_map
    }

    async fn payload(body: &'static str) -> actix_web::web::Payload {
        let (request, mut payload) = actix_web::test::TestRequest::default()
            .set_payload(body)
            .to_http_parts();

        actix_web::web::Payload::from_request(&request, &mut payload)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_convert_body() {
        let mut payload = payload("foo").await;

        let body = super::ActixToReqwestConverter::convert_body(&mut payload)
            .await
            .unwrap();

        assert_eq!(body.as_bytes(), Some(&b"foo"[..]));
    }

    #[tokio::test]
    async fn test_convert_body_empty() {
        let mut payload = payload("").await;

        let body = super::ActixToReqwestConverter::convert_body(&mut payload)
            .await
            .unwrap();

        assert_eq!(body.as_bytes(), Some(&b""[..]));
    }

    #[test]
    fn test_convert_headers_invalid_header() {
        let mut header_values = HashMap::new();
        header_values.insert("Host".to_string(), "localhost".to_string());

        let headers = convert_headers(header_values);

        let converted_headers = super::ActixToReqwestConverter::convert_headers(&headers, 0);

        assert!(converted_headers.is_empty());
    }

    #[test]
    fn test_convert_headers() {
        let mut header_values = HashMap::new();
        // Valid headers
        header_values.insert("test".to_string(), "value".to_string());

        // Invalid headers
        header_values.insert("host".to_string(), "localhost".to_string());

        let headers = convert_headers(header_values);

        let converted_headers = super::ActixToReqwestConverter::convert_headers(&headers, 0);

        assert_eq!(converted_headers.len(), 1);
        assert_eq!(converted_headers.get("test").unwrap(), "value");
    }
}

#[cfg(test)]
mod tests_reqwest_to_actix_converter {
    use http::response::Builder;
    use reqwest::{Response, ResponseBuilderExt, Url};

    #[test]
    fn test_convert_status_code() {
        let status_code = reqwest::StatusCode::OK;
        let actix_status_code =
            super::ReqwestToActixConverter::convert_status_code(status_code).unwrap();

        assert_eq!(actix_status_code, actix_web::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_convert_response() {
        let url = Url::parse("https://example.com").unwrap();
        let response = Builder::new()
            .status(200)
            .url(url.clone())
            .body("foo")
            .unwrap();

        let response = Response::from(response);
        let actix_response = super::ReqwestToActixConverter::convert_response(response)
            .await
            .unwrap();

        assert_eq!(actix_response.status(), actix_web::http::StatusCode::OK);

        let body = actix_web::body::to_bytes(actix_response.into_body())
            .await
            .unwrap();
        assert_eq!(body, actix_web::web::Bytes::from_static(b"foo"));
    }
}
