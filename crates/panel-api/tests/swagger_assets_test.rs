use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[tokio::test]
async fn swagger_assets_and_panel_schema_are_served_after_zip_security_backport() {
    // In-process requests only: no API daemon, listener, database or agent.
    let app: Router = SwaggerUi::new("/swagger-ui")
        .url("/api/v1/openapi.json", panel_api::routes::ApiDoc::openapi())
        .into();
    for (path, expected) in [
        ("/swagger-ui/", "Swagger UI"),
        ("/swagger-ui/swagger-ui.css", ".swagger-ui"),
        ("/swagger-ui/swagger-ui-bundle.js", "SwaggerUIBundle"),
        ("/swagger-ui/swagger-initializer.js", "/api/v1/openapi.json"),
        ("/api/v1/openapi.json", "Qinpanel API"),
    ] {
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("request");
        let response = app.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let text = std::str::from_utf8(&bytes).expect("text asset");
        assert!(text.contains(expected), "unexpected content for {path}");
    }
}
