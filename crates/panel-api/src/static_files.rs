//! Static file serving with embedded single-page application (SPA).

use axum::{
    body::Body,
    http::{HeaderValue, Response, StatusCode, Uri, header},
    response::IntoResponse,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
pub struct WebAssets;

pub async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    // Unknown service/asset routes must keep their 404 semantics.
    if path == "api" || path.starts_with("api/") || path == "ws" || path.starts_with("ws/") {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }

    // If path is empty, serve index.html
    let asset_path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = WebAssets::get(asset_path) {
        let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            )
            .body(Body::from(file.data))
            .unwrap_or_else(|_| {
                (StatusCode::INTERNAL_SERVER_ERROR, "Response error").into_response()
            });
    }

    // SPA fallback: If file not found and doesn't look like a direct asset file, serve index.html
    if !path.contains('.') && !path.starts_with("assets/") && !path.starts_with("fonts/") {
        return match WebAssets::get("index.html") {
            Some(file) => Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )
                .body(Body::from(file.data))
                .unwrap_or_else(|_| {
                    (StatusCode::INTERNAL_SERVER_ERROR, "Response error").into_response()
                }),
            None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        };
    }

    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
