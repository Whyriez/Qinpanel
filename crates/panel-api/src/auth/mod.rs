//! Authentication, session management, CSRF protection, and lockout policies.

pub mod csrf;
pub mod handlers;
pub mod lockout;
pub mod middleware;
pub mod session;

pub use csrf::{CSRF_HEADER_NAME, generate_csrf_token, verify_csrf_token};
pub use handlers::*;
pub use lockout::LockoutManager;
pub use middleware::{
    AuthenticatedUser, RequireAdmin, RequireAdminCsrf, RequireAuth, RequireCsrf, RequireUser,
    RequireUserCsrf, validate_csrf_header,
};
pub use session::{
    SESSION_COOKIE_NAME, build_clear_session_cookie, build_session_cookie, extract_session_token,
};
