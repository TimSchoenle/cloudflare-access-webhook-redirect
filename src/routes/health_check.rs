//! The liveness endpoint the container's health check calls.

use actix_web::{HttpResponse, web};

/// Registers `GET /health`, which answers `200` as long as the process is serving.
///
/// It reaches nothing upstream, so a `200` says the listener is up and says nothing about whether
/// the protected service is.
pub fn get_config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/health").route(web::get().to(HttpResponse::Ok)));
}

#[cfg(test)]
mod tests {
    use crate::routes::health_check::get_config;
    use actix_web::{App, test};

    #[actix_web::test]
    async fn test_handle_web_hook() {
        let app = test::init_service(App::new().configure(get_config)).await;

        let req = test::TestRequest::get().uri("/health").to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }
}
