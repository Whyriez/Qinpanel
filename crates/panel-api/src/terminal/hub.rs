//! In-memory PTY terminal session coordinator, scrollback buffer, and agent bridge.
//!
//! Complies with PRD Section 5.4 and Task P6-T2:
//! - Multi-client broadcast streaming for terminal output
//! - Ring-buffer bounded scrollback preservation (default 64 KB) for seamless reconnect
//! - Direct backpressure-handled PTY reader task pulling from `panel-agent`
//! - Non-blocking input forwarding, window resizing, and graceful closure

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use panel_proto::{
    PtyCloseRequest, PtyInputRequest, PtyOpenRequest, PtyReadOutputRequest, PtyResizeRequest,
};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, info, warn};

use crate::agent_client::AgentClient;
use crate::terminal::TerminalSessionResponse;

const DEFAULT_SCROLLBACK_CAPACITY_BYTES: usize = 65_536; // 64 KB
const BROADCAST_CHANNEL_CAPACITY: usize = 256;
const PTY_READ_BUFFER_MAX_BYTES: usize = 16_384; // 16 KB chunk per read

#[derive(Debug, Error)]
pub enum TerminalHubError {
    #[error("Terminal session not found: {0}")]
    SessionNotFound(String),
    #[error("Session is already closed")]
    SessionClosed,
    #[error("Agent communication error: {0}")]
    Agent(String),
    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

/// Ring-buffer scrollback storage with a strict upper byte limit.
///
/// When incoming data pushes total size past `capacity`, oldest bytes are discarded
/// to keep memory predictable while guaranteeing recent terminal output is preserved.
#[derive(Debug, Clone)]
pub struct ScrollbackBuffer {
    capacity: usize,
    buffer: Vec<u8>,
}

impl ScrollbackBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            buffer: Vec::with_capacity(capacity.min(4096)),
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data.len() >= self.capacity {
            self.buffer = data[data.len() - self.capacity..].to_vec();
            return;
        }

        let total_len = self.buffer.len() + data.len();
        if total_len > self.capacity {
            let overflow = total_len - self.capacity;
            self.buffer.drain(..overflow);
        }
        self.buffer.extend_from_slice(data);
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.buffer.clone()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Event broadcast to active WebSocket subscriber connections for a session.
#[derive(Debug, Clone)]
pub enum TerminalBroadcastEvent {
    Output(Arc<Vec<u8>>),
    Closed(Option<i32>),
}

/// In-memory representation of an active or recently terminated PTY session.
pub struct ActiveTerminalSession {
    pub session_id: String,
    pub user_id: i64,
    pub username: String,
    pub pid: u32,
    pub cwd: String,
    pub cols: Arc<AtomicU16>,
    pub rows: Arc<AtomicU16>,
    pub created_at: i64,
    pub last_active_at: Arc<AtomicI64>,
    pub scrollback: Arc<RwLock<ScrollbackBuffer>>,
    pub is_closed: Arc<AtomicBool>,
    pub exit_code: Arc<RwLock<Option<i32>>>,
    pub tx: broadcast::Sender<TerminalBroadcastEvent>,
    pub linux_username: String,
    pub client_ip: Arc<RwLock<Option<String>>>,
    pub closed_audit_emitted: Arc<AtomicBool>,
}

impl ActiveTerminalSession {
    pub fn to_response(&self) -> TerminalSessionResponse {
        TerminalSessionResponse {
            session_id: self.session_id.clone(),
            username: self.linux_username.clone(),
            pid: self.pid,
            cwd: self.cwd.clone(),
            cols: self.cols.load(Ordering::Relaxed),
            rows: self.rows.load(Ordering::Relaxed),
            created_at: self.created_at,
            ws_url: format!("/ws/terminal/{}", self.session_id),
            closed: self.is_closed.load(Ordering::Relaxed),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalBroadcastEvent> {
        self.tx.subscribe()
    }

    pub fn update_activity(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.last_active_at.store(now, Ordering::Relaxed);
    }
}

/// Hub managing terminal sessions, coordinating background agent reader tasks,
/// and streaming input/output across WebSockets.
pub struct TerminalHub {
    sessions: Arc<RwLock<HashMap<String, Arc<ActiveTerminalSession>>>>,
    agent_client: Arc<AgentClient>,
    audit_writer: Option<Arc<crate::audit::AuditWriter>>,
}

impl TerminalHub {
    pub fn new(agent_client: Arc<AgentClient>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            agent_client,
            audit_writer: None,
        }
    }

    pub fn with_audit_writer(mut self, audit_writer: Arc<crate::audit::AuditWriter>) -> Self {
        self.audit_writer = Some(audit_writer);
        self
    }

    /// Spawn a new PTY session on the agent and register it in the in-memory hub.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &self,
        user_id: i64,
        username: String,
        linux_username: String,
        uid: u32,
        gid: u32,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
        max_sessions: Option<usize>,
        idle_timeout_secs: Option<u64>,
        client_ip: Option<String>,
    ) -> Result<Arc<ActiveTerminalSession>, TerminalHubError> {
        let req = PtyOpenRequest {
            username: linux_username.clone(),
            uid,
            gid,
            cwd: cwd.clone(),
            cols,
            rows,
            term: Some("xterm-256color".to_string()),
            max_sessions,
            idle_timeout_secs,
        };

        let open_res = self
            .agent_client
            .pty_open(req)
            .await
            .map_err(|e| TerminalHubError::Agent(e.to_string()))?;

        let (tx, _rx) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let scrollback = Arc::new(RwLock::new(ScrollbackBuffer::new(
            DEFAULT_SCROLLBACK_CAPACITY_BYTES,
        )));
        let cols_atomic = Arc::new(AtomicU16::new(open_res.cols));
        let rows_atomic = Arc::new(AtomicU16::new(open_res.rows));
        let is_closed = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(RwLock::new(None));
        let last_active = Arc::new(AtomicI64::new(now));
        let client_ip_arc = Arc::new(RwLock::new(client_ip));
        let closed_audit_emitted = Arc::new(AtomicBool::new(false));

        let session = Arc::new(ActiveTerminalSession {
            session_id: open_res.session_id.clone(),
            user_id,
            username: username.clone(),
            pid: open_res.pid,
            cwd: open_res.cwd.clone(),
            cols: cols_atomic,
            rows: rows_atomic,
            created_at: open_res.created_at,
            last_active_at: last_active,
            scrollback: Arc::clone(&scrollback),
            is_closed: Arc::clone(&is_closed),
            exit_code: Arc::clone(&exit_code),
            tx: tx.clone(),
            linux_username: open_res.username,
            client_ip: Arc::clone(&client_ip_arc),
            closed_audit_emitted: Arc::clone(&closed_audit_emitted),
        });

        {
            let mut lock = self.sessions.write().await;
            lock.insert(open_res.session_id.clone(), Arc::clone(&session));
        }

        // Spawn async reader loop pulling PTY output from host agent
        let reader_agent = Arc::clone(&self.agent_client);
        let reader_sid = open_res.session_id.clone();
        let reader_closed = Arc::clone(&is_closed);
        let reader_exit = Arc::clone(&exit_code);
        let audit_writer_opt = self.audit_writer.clone();
        let reader_user_id = user_id;
        let reader_username = username.clone();
        let reader_linux_user = linux_username.clone();
        let reader_cwd = open_res.cwd.clone();
        let reader_created_at = open_res.created_at;
        let reader_ip_lock = Arc::clone(&client_ip_arc);
        let reader_audit_emitted = Arc::clone(&closed_audit_emitted);

        tokio::spawn(async move {
            run_pty_reader_loop(
                reader_sid,
                reader_agent,
                scrollback,
                reader_closed,
                reader_exit,
                tx,
                audit_writer_opt,
                reader_user_id,
                reader_username,
                reader_linux_user,
                reader_cwd,
                reader_created_at,
                reader_ip_lock,
                reader_audit_emitted,
            )
            .await;
        });

        info!(
            "Registered terminal session {} (pid {}) for user {} (uid {})",
            open_res.session_id, open_res.pid, username, user_id
        );

        Ok(session)
    }

    /// Retrieve an active terminal session by ID.
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<ActiveTerminalSession>> {
        let lock = self.sessions.read().await;
        lock.get(session_id).cloned()
    }

    /// List active sessions filtered by tenant user (or all if admin).
    pub async fn list_sessions_for_user(
        &self,
        user_id: i64,
        is_admin: bool,
    ) -> Vec<Arc<ActiveTerminalSession>> {
        let lock = self.sessions.read().await;
        lock.values()
            .filter(|s| is_admin || s.user_id == user_id)
            .cloned()
            .collect()
    }

    /// Count active unclosed sessions for a specific tenant user.
    pub async fn count_active_user_sessions(&self, user_id: i64) -> usize {
        let lock = self.sessions.read().await;
        lock.values()
            .filter(|s| s.user_id == user_id && !s.is_closed.load(Ordering::Relaxed))
            .count()
    }

    /// Forward input data (keystrokes / paste) to PTY stdin on host agent.
    pub async fn write_input(
        &self,
        session_id: &str,
        data: Vec<u8>,
    ) -> Result<usize, TerminalHubError> {
        let session = self
            .get_session(session_id)
            .await
            .ok_or_else(|| TerminalHubError::SessionNotFound(session_id.to_string()))?;

        if session.is_closed.load(Ordering::Relaxed) {
            return Err(TerminalHubError::SessionClosed);
        }

        session.update_activity();

        let req = PtyInputRequest {
            session_id: session_id.to_string(),
            data,
        };

        let res = self
            .agent_client
            .pty_input(req)
            .await
            .map_err(|e| TerminalHubError::Agent(e.to_string()))?;

        Ok(res.bytes_written)
    }

    /// Resize terminal dimensions across agent PTY and in-memory session.
    pub async fn resize_session(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalHubError> {
        let session = self
            .get_session(session_id)
            .await
            .ok_or_else(|| TerminalHubError::SessionNotFound(session_id.to_string()))?;

        if session.is_closed.load(Ordering::Relaxed) {
            return Err(TerminalHubError::SessionClosed);
        }

        session.update_activity();

        let req = PtyResizeRequest {
            session_id: session_id.to_string(),
            cols,
            rows,
        };

        self.agent_client
            .pty_resize(req)
            .await
            .map_err(|e| TerminalHubError::Agent(e.to_string()))?;

        session.cols.store(cols, Ordering::Relaxed);
        session.rows.store(rows, Ordering::Relaxed);

        debug!(
            "Resized terminal session {} to {}x{}",
            session_id, cols, rows
        );
        Ok(())
    }

    /// Close and terminate a PTY session.
    pub async fn close_session(
        &self,
        session_id: &str,
        force: bool,
    ) -> Result<bool, TerminalHubError> {
        let session = self
            .get_session(session_id)
            .await
            .ok_or_else(|| TerminalHubError::SessionNotFound(session_id.to_string()))?;

        session.is_closed.store(true, Ordering::SeqCst);

        let req = PtyCloseRequest {
            session_id: session_id.to_string(),
            force,
        };

        let res = match self.agent_client.pty_close(req).await {
            Ok(r) => {
                let mut ec = session.exit_code.write().await;
                *ec = r.exit_code;
                r.closed
            }
            Err(e) => {
                warn!(
                    "Agent pty_close returned error for session {}: {:?}",
                    session_id, e
                );
                true
            }
        };

        let _ = session.tx.send(TerminalBroadcastEvent::Closed(None));

        // Emit audit log for terminal session end (P6-T4: catat akhir sesi)
        let client_ip = session.client_ip.read().await.clone();
        Self::emit_session_end_audit(
            &self.audit_writer,
            session_id,
            session.user_id,
            &session.username,
            &session.linux_username,
            &session.cwd,
            session.created_at,
            client_ip,
            *session.exit_code.read().await,
            "manual_close",
            &session.closed_audit_emitted,
        );

        // Note: We retain the session in in-memory map temporarily so clients can inspect exit status,
        // or prune if explicitly requested.
        info!("Closed terminal session {}", session_id);
        Ok(res)
    }

    /// Helper to record session end audit log entry safely without double emissions.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_session_end_audit(
        audit_writer: &Option<Arc<crate::audit::AuditWriter>>,
        session_id: &str,
        user_id: i64,
        username: &str,
        linux_username: &str,
        cwd: &str,
        created_at: i64,
        client_ip: Option<String>,
        exit_code: Option<i32>,
        reason: &str,
        closed_audit_emitted: &AtomicBool,
    ) {
        if !closed_audit_emitted.swap(true, Ordering::SeqCst)
            && let Some(writer) = audit_writer
        {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let duration_secs = (now - created_at).max(0);

            let details = serde_json::json!({
                "session_id": session_id,
                "tenant_user_id": user_id,
                "tenant": linux_username,
                "linux_username": linux_username,
                "folder": cwd,
                "duration_secs": duration_secs,
                "exit_code": exit_code,
                "reason": reason,
            });

            writer.log_async(
                Some(user_id),
                username,
                "terminal.session_end",
                session_id,
                "success",
                client_ip,
                Some(details.to_string()),
            );
        }
    }

    /// Remove a terminated session entirely from hub memory.
    pub async fn remove_session(&self, session_id: &str) {
        let mut lock = self.sessions.write().await;
        lock.remove(session_id);
    }
}

/// Background loop running continuously per session, pulling non-blocking output
/// from the host agent and streaming to broadcast channels + scrollback buffer.
#[allow(clippy::cognitive_complexity, clippy::too_many_arguments)]
async fn run_pty_reader_loop(
    session_id: String,
    agent_client: Arc<AgentClient>,
    scrollback: Arc<RwLock<ScrollbackBuffer>>,
    is_closed: Arc<AtomicBool>,
    exit_code_store: Arc<RwLock<Option<i32>>>,
    tx: broadcast::Sender<TerminalBroadcastEvent>,
    audit_writer: Option<Arc<crate::audit::AuditWriter>>,
    user_id: i64,
    username: String,
    linux_username: String,
    cwd: String,
    created_at: i64,
    client_ip_lock: Arc<RwLock<Option<String>>>,
    closed_audit_emitted: Arc<AtomicBool>,
) {
    let mut consecutive_empty_reads = 0;

    while !is_closed.load(Ordering::Relaxed) {
        let req = PtyReadOutputRequest {
            session_id: session_id.clone(),
            max_bytes: PTY_READ_BUFFER_MAX_BYTES,
        };

        match agent_client.pty_read_output(req).await {
            Ok(res) => {
                if !res.data.is_empty() {
                    consecutive_empty_reads = 0;

                    // 1. Append to ring-buffered scrollback
                    {
                        let mut sb = scrollback.write().await;
                        sb.append(&res.data);
                    }

                    // 2. Broadcast output frame to any listening WebSockets
                    let _ = tx.send(TerminalBroadcastEvent::Output(Arc::new(res.data)));
                } else {
                    consecutive_empty_reads += 1;
                }

                if res.closed {
                    is_closed.store(true, Ordering::SeqCst);
                    {
                        let mut ec = exit_code_store.write().await;
                        *ec = res.exit_code;
                    }
                    let _ = tx.send(TerminalBroadcastEvent::Closed(res.exit_code));
                    info!(
                        "PTY process for session {} exited with code {:?}",
                        session_id, res.exit_code
                    );

                    let client_ip = client_ip_lock.read().await.clone();
                    TerminalHub::emit_session_end_audit(
                        &audit_writer,
                        &session_id,
                        user_id,
                        &username,
                        &linux_username,
                        &cwd,
                        created_at,
                        client_ip,
                        res.exit_code,
                        "process_exit",
                        &closed_audit_emitted,
                    );
                    break;
                }

                // Adaptive backoff: tight 5ms yield when active, graduating to 20ms and 50ms when idle
                if consecutive_empty_reads > 10 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                } else if consecutive_empty_reads > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
                } else {
                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                }
            }
            Err(e) => {
                warn!("PTY reader poll failed for session {}: {:?}", session_id, e);
                is_closed.store(true, Ordering::SeqCst);
                let _ = tx.send(TerminalBroadcastEvent::Closed(None));

                let client_ip = client_ip_lock.read().await.clone();
                TerminalHub::emit_session_end_audit(
                    &audit_writer,
                    &session_id,
                    user_id,
                    &username,
                    &linux_username,
                    &cwd,
                    created_at,
                    client_ip,
                    None,
                    "agent_read_error",
                    &closed_audit_emitted,
                );
                break;
            }
        }
    }
}
