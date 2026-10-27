//! Drain panel writes before a consistent managed-server snapshot.
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::atomic::{AtomicUsize, Ordering};
pub const FLAG: &str = "/var/lib/qonpanel-migration/maintenance";
static WRITES: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct WriteGuard;
impl Drop for WriteGuard {
    fn drop(&mut self) {
        WRITES.fetch_sub(1, Ordering::SeqCst);
    }
}
pub fn active() -> usize {
    WRITES.load(Ordering::SeqCst)
}
/// Background observers participate in the same drain barrier as HTTP writes.
pub(crate) fn background_write() -> Option<WriteGuard> {
    WRITES.fetch_add(1, Ordering::SeqCst);
    let guard = WriteGuard;
    if std::path::Path::new(FLAG).exists() {
        None
    } else {
        Some(guard)
    }
}
pub fn blocked(method: &str, path: &str, maintenance: bool) -> bool {
    maintenance
        && matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
        && !matches!(path, "/api/v1/auth/login" | "/api/v1/auth/logout")
}
pub async fn middleware(request: Request, next: Next) -> Response {
    let write = matches!(
        request.method().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    );
    let _guard = if write {
        WRITES.fetch_add(1, Ordering::SeqCst);
        Some(WriteGuard)
    } else {
        None
    };
    if blocked(
        request.method().as_str(),
        request.uri().path(),
        std::path::Path::new(FLAG).exists(),
    ) {
        return (StatusCode::SERVICE_UNAVAILABLE,axum::Json(serde_json::json!({"detail":"Panel sedang membuat snapshot/restore. Tunggu layanan kembali sebelum mengubah data."}))).into_response();
    }
    next.run(request).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintenance_blocks_writes_but_keeps_status_and_auth_available() {
        assert!(blocked("POST", "/api/v1/sites", true));
        assert!(blocked("PUT", "/api/v1/sites/1", true));
        assert!(!blocked("GET", "/api/v1/system/server-backup", true));
        assert!(!blocked("POST", "/api/v1/auth/login", true));
        assert!(!blocked("DELETE", "/api/v1/sites/1", false));
    }
}
