//! Audit log module for Qinpanel.
//!
//! Provides `AuditWriter` service for recording actions, and HTTP API endpoints
//! for listing, filtering, and retrieving audit logs with RBAC policy enforcement.

pub mod handlers;
pub mod writer;

pub use handlers::{
    AuditLogItem, AuditLogListResponse, AuditLogQuery, get_audit_log_handler,
    list_audit_logs_handler,
};
pub use writer::AuditWriter;

/// Helper function to extract the client IP address from request headers.
pub fn extract_client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        })
}
