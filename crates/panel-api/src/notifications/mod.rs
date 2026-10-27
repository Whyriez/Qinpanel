//! Notification module for panel-api.

pub mod routes;
pub mod service;

pub use routes::router;
pub use service::{DispatchNotificationEvent, NotificationService};
