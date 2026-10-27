//! HTTP security headers middleware enforcing OWASP ASVS L2 requirements.
//!
//! Injects strict Content Security Policy (CSP), anti-clickjacking, MIME-sniffing
//! protection, referrer policy, and strict transport security headers into all responses.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

pub const CSP_VALUE: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' ws: wss:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";
pub const X_CONTENT_TYPE_OPTIONS_VALUE: &str = "nosniff";
pub const X_FRAME_OPTIONS_VALUE: &str = "DENY";
pub const X_XSS_PROTECTION_VALUE: &str = "1; mode=block";
pub const REFERRER_POLICY_VALUE: &str = "strict-origin-when-cross-origin";
pub const PERMISSIONS_POLICY_VALUE: &str = "geolocation=(), camera=(), microphone=(), payment=()";
pub const HSTS_VALUE: &str = "max-age=31536000; includeSubDomains; preload";

/// Injects comprehensive OWASP ASVS L2 security headers into outgoing HTTP responses.
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let database_tool = req
        .uri()
        .path()
        .strip_prefix("/api/v1/databases/")
        .is_some_and(|path| {
            path.split_once('/').is_some_and(|(id, rest)| {
                id.parse::<i64>().is_ok_and(|id| id > 0) && rest.starts_with("web/")
            })
        });
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    if !database_tool || !headers.contains_key("Content-Security-Policy") {
        headers.insert(
            "Content-Security-Policy",
            HeaderValue::from_static(CSP_VALUE),
        );
    }
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static(X_CONTENT_TYPE_OPTIONS_VALUE),
    );
    headers.insert(
        "X-Frame-Options",
        HeaderValue::from_static(X_FRAME_OPTIONS_VALUE),
    );
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static(X_XSS_PROTECTION_VALUE),
    );
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static(if database_tool {
            "no-referrer"
        } else {
            REFERRER_POLICY_VALUE
        }),
    );
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static(PERMISSIONS_POLICY_VALUE),
    );
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static(HSTS_VALUE),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn;
    use axum::routing::get;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_security_headers_injected_in_response() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(from_fn(security_headers_middleware));

        let req = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .expect("request");

        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let headers = response.headers();
        assert_eq!(
            headers
                .get("Content-Security-Policy")
                .and_then(|v| v.to_str().ok()),
            Some(CSP_VALUE)
        );
        assert_eq!(
            headers
                .get("X-Content-Type-Options")
                .and_then(|v| v.to_str().ok()),
            Some(X_CONTENT_TYPE_OPTIONS_VALUE)
        );
        assert_eq!(
            headers.get("X-Frame-Options").and_then(|v| v.to_str().ok()),
            Some(X_FRAME_OPTIONS_VALUE)
        );
        assert_eq!(
            headers.get("Referrer-Policy").and_then(|v| v.to_str().ok()),
            Some(REFERRER_POLICY_VALUE)
        );
        assert_eq!(
            headers
                .get("Strict-Transport-Security")
                .and_then(|v| v.to_str().ok()),
            Some(HSTS_VALUE)
        );
    }
}
