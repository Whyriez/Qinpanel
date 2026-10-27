//! Web terminal and PTY session management module.
//!
//! Complies with PRD Section 5.4 and Tasks P6-T1 / P6-T2:
//! - PTY session creation, listing, resizing, and teardown
//! - Bounded scrollback reconnect streaming
//! - Bidirectional WebSocket terminal protocol
//! - Strict tenant isolation & CSWSH prevention

pub mod handlers;
pub mod hub;
pub mod ws;

pub use handlers::*;
pub use hub::*;
pub use ws::*;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Request payload to initiate a new PTY session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CreateTerminalSessionRequest {
    /// Initial working directory. If omitted, defaults to tenant home directory.
    pub cwd: Option<String>,
    /// Initial terminal columns (default: 80, range 20..500)
    pub cols: Option<u16>,
    /// Initial terminal rows (default: 24, range 5..200)
    pub rows: Option<u16>,
    /// Target tenant username (administrator only)
    pub tenant: Option<String>,
}

/// Details of a running or terminated terminal session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TerminalSessionResponse {
    /// Unique session identifier UUID
    pub session_id: String,
    /// Linux system username the PTY is executing under
    pub username: String,
    /// Host OS Process ID
    pub pid: u32,
    /// Current confined working directory
    pub cwd: String,
    /// Terminal columns
    pub cols: u16,
    /// Terminal rows
    pub rows: u16,
    /// Unix timestamp of creation (seconds)
    pub created_at: i64,
    /// WebSocket streaming endpoint relative path
    pub ws_url: String,
    /// Whether the process has exited
    pub closed: bool,
}

/// List of terminal sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TerminalSessionListResponse {
    pub sessions: Vec<TerminalSessionResponse>,
}

/// Response returned when closing a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TerminalCloseApiResponse {
    pub session_id: String,
    pub closed: bool,
    pub message: String,
}

/// Request payload to resize a terminal session window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TerminalResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

/// Response payload after resizing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TerminalResizeResponse {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// Query parameters for listing sessions.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TerminalListQuery {
    pub tenant: Option<String>,
}

/// Inbound control/data message received from WebSocket client.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    /// Terminal window resize event
    Resize { cols: u16, rows: u16 },
    /// Heartbeat ping
    Ping,
    /// Raw input text / keystroke data
    Input { data: String },
}

/// Outbound control message sent from server to WebSocket client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    /// Initial connection confirmation & metadata
    Connected {
        session_id: String,
        cols: u16,
        rows: u16,
        cwd: String,
        username: String,
    },
    /// Heartbeat pong response
    Pong,
    /// Notification that the underlying process has exited
    Closed { exit_code: Option<i32> },
    /// Error notification
    Error { message: String },
}

/// Query parameters for retrieving terminal audit entries.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TerminalAuditQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub tenant: Option<String>,
}

/// Structured terminal audit item with duration, folder, and client IP.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TerminalAuditItem {
    pub id: i64,
    pub session_id: String,
    pub actor_username: String,
    pub action: String,
    pub tenant: Option<String>,
    pub folder: Option<String>,
    pub duration_secs: Option<i64>,
    pub ip_address: Option<String>,
    pub status: String,
    pub keystroke_recording: bool,
    pub created_at: i64,
}

/// Paginated list response of terminal audit events.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TerminalAuditListResponse {
    pub items: Vec<TerminalAuditItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
}
