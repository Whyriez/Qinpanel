//! Central PTY session manager coordinating terminal lifecycles, resource limits, and path confinement.

use super::backend::{PtyBackend, PtySpawnConfig};
use super::session::PtySession;
use crate::AgentError;
use panel_core::Username;
use panel_proto::{
    PtyCloseRequest, PtyCloseResponse, PtyInputRequest, PtyInputResponse, PtyListRequest,
    PtyListResponse, PtyOpenRequest, PtyOpenResponse, PtyReadOutputRequest, PtyReadOutputResponse,
    PtyResizeRequest, PtyResizeResponse,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

pub const DEFAULT_MAX_SESSIONS_PER_TENANT: usize = 5;
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 900; // 15 minutes
pub const DEFAULT_HOME_BASE: &str = "/home";
pub const DEFAULT_SHELL: &str = "/bin/bash";
pub const DEFAULT_TERM: &str = "xterm-256color";

/// Configuration settings for the PTY session manager.
#[derive(Debug, Clone)]
pub struct PtyManagerConfig {
    pub home_base: PathBuf,
    pub default_shell: String,
    pub default_term: String,
    pub max_sessions_per_tenant: usize,
    pub idle_timeout: Duration,
    pub cgroup_base: PathBuf,
}

impl Default for PtyManagerConfig {
    fn default() -> Self {
        Self {
            home_base: PathBuf::from(DEFAULT_HOME_BASE),
            default_shell: DEFAULT_SHELL.to_string(),
            default_term: DEFAULT_TERM.to_string(),
            max_sessions_per_tenant: DEFAULT_MAX_SESSIONS_PER_TENANT,
            idle_timeout: Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS),
            cgroup_base: PathBuf::from("/sys/fs/cgroup"),
        }
    }
}

/// Thread-safe session manager orchestrating pseudo-terminal sessions for tenants.
pub struct PtySessionManager {
    config: PtyManagerConfig,
    backend: Arc<dyn PtyBackend>,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<PtySession>>>>>,
    tenant_sessions: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl PtySessionManager {
    /// Creates a new PTY session manager with the provided backend and config.
    pub fn new(config: PtyManagerConfig, backend: Arc<dyn PtyBackend>) -> Self {
        Self {
            config,
            backend,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            tenant_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawns an asynchronous background reaper that periodically closes idle PTY sessions.
    pub fn start_idle_reaper(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let reaped = mgr.cleanup_idle_sessions().await;
                if reaped > 0 {
                    debug!("PTY idle reaper terminated {} inactive sessions", reaped);
                }
            }
        })
    }

    /// Validates and resolves the target working directory confined within the tenant's home root.
    pub fn resolve_confined_cwd(
        &self,
        username: &str,
        requested_cwd: Option<&str>,
    ) -> Result<PathBuf, AgentError> {
        let home_root = self.config.home_base.join(username);

        let target_path = match requested_cwd {
            None => home_root.clone(),
            Some(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() || trimmed == "/" || trimmed == "." {
                    home_root.clone()
                } else if trimmed.starts_with('/') {
                    // An absolute path must be within tenant's home
                    let p = PathBuf::from(trimmed);
                    if !p.starts_with(&home_root) {
                        return Err(AgentError::SecurityViolation(format!(
                            "Directory path '{trimmed}' escapes tenant home root '{}'",
                            home_root.display()
                        )));
                    }
                    p
                } else {
                    // Relative path from tenant home root
                    // Protect against directory traversal '..'
                    for component in Path::new(trimmed).components() {
                        if matches!(component, std::path::Component::ParentDir) {
                            return Err(AgentError::SecurityViolation(format!(
                                "Path traversal component '..' rejected in cwd '{trimmed}'"
                            )));
                        }
                    }
                    home_root.join(trimmed)
                }
            }
        };

        // If target exists, verify canonical path does not escape home root
        if target_path.exists()
            && let Ok(canonical) = std::fs::canonicalize(&target_path)
            && let Ok(home_canonical) = std::fs::canonicalize(&home_root)
            && !canonical.starts_with(&home_canonical)
        {
            return Err(AgentError::SecurityViolation(format!(
                "Symlink resolution escapes tenant root: {}",
                canonical.display()
            )));
        }

        Ok(target_path)
    }

    /// Opens a new PTY login shell for a tenant.
    pub async fn open_session(&self, req: PtyOpenRequest) -> Result<PtyOpenResponse, AgentError> {
        let username =
            Username::parse(&req.username).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let username_str = username.as_str();

        panel_core::tenant::TenantUid::parse(req.uid)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        if req.gid != req.uid {
            return Err(AgentError::SecurityViolation(
                "GID terminal harus sama dengan UID tenant".into(),
            ));
        }

        let max_sessions = req
            .max_sessions
            .unwrap_or(self.config.max_sessions_per_tenant);

        // Check concurrent session limits for this tenant
        {
            let tenant_guard = self.tenant_sessions.read().await;
            if let Some(active_set) = tenant_guard.get(username_str)
                && active_set.len() >= max_sessions
            {
                return Err(AgentError::InvalidInput(format!(
                    "Maximum concurrent PTY sessions ({max_sessions}) reached for tenant '{username_str}'"
                )));
            }
        }

        // Validate confined cwd
        let cwd = self.resolve_confined_cwd(username_str, req.cwd.as_deref())?;

        let rows = if req.rows == 0 { 24 } else { req.rows };
        let cols = if req.cols == 0 { 80 } else { req.cols };
        let term = req.term.unwrap_or_else(|| self.config.default_term.clone());
        let slice_name = format!("panel-u{}.slice", req.uid);

        let spawn_cfg = PtySpawnConfig {
            username: username_str.to_string(),
            uid: req.uid,
            gid: req.gid,
            cwd: cwd.clone(),
            shell: self.config.default_shell.clone(),
            rows,
            cols,
            term,
            slice_name: Some(slice_name),
            cgroup_base: self.config.cgroup_base.clone(),
        };

        let handle = self.backend.spawn(&spawn_cfg)?;
        let pid = handle.pid();
        let session_id = Uuid::new_v4().to_string();

        let session = PtySession::new(
            session_id.clone(),
            username_str.to_string(),
            req.uid,
            req.gid,
            cwd.clone(),
            rows,
            cols,
            handle,
        );

        let created_at = session.created_at;

        // Register session in manager state
        {
            let mut sessions_guard = self.sessions.write().await;
            sessions_guard.insert(session_id.clone(), Arc::new(Mutex::new(session)));
        }

        {
            let mut tenant_guard = self.tenant_sessions.write().await;
            tenant_guard
                .entry(username_str.to_string())
                .or_default()
                .insert(session_id.clone());
        }

        info!(
            "Opened PTY session {} for tenant '{}' (PID: {}, cwd: {})",
            session_id,
            username_str,
            pid,
            cwd.display()
        );

        Ok(PtyOpenResponse {
            session_id,
            username: username_str.to_string(),
            pid,
            cwd: cwd.display().to_string(),
            rows,
            cols,
            created_at,
        })
    }

    /// Writes input to an active PTY session.
    pub async fn write_input(&self, req: PtyInputRequest) -> Result<PtyInputResponse, AgentError> {
        let session_arc = {
            let guard = self.sessions.read().await;
            guard.get(&req.session_id).cloned().ok_or_else(|| {
                AgentError::InvalidInput(format!("PTY session '{}' not found", req.session_id))
            })?
        };

        let session = session_arc.lock().await;
        let bytes_written = session.write_input(&req.data)?;

        Ok(PtyInputResponse {
            session_id: req.session_id,
            bytes_written,
        })
    }

    /// Reads output from an active PTY session.
    pub async fn read_output(
        &self,
        req: PtyReadOutputRequest,
    ) -> Result<PtyReadOutputResponse, AgentError> {
        let session_arc = {
            let guard = self.sessions.read().await;
            guard.get(&req.session_id).cloned().ok_or_else(|| {
                AgentError::InvalidInput(format!("PTY session '{}' not found", req.session_id))
            })?
        };

        let session = session_arc.lock().await;
        let (data, closed, exit_code) = session.read_output(req.max_bytes)?;

        // If session closed and child exited, clean up session
        if closed {
            let session_id = req.session_id.clone();
            let username = session.username.clone();
            drop(session);
            self.remove_session(&session_id, &username).await;
        }

        Ok(PtyReadOutputResponse {
            session_id: req.session_id,
            data,
            closed,
            exit_code,
        })
    }

    /// Resizes dimensions of an active PTY session.
    pub async fn resize(&self, req: PtyResizeRequest) -> Result<PtyResizeResponse, AgentError> {
        let session_arc = {
            let guard = self.sessions.read().await;
            guard.get(&req.session_id).cloned().ok_or_else(|| {
                AgentError::InvalidInput(format!("PTY session '{}' not found", req.session_id))
            })?
        };

        let mut session = session_arc.lock().await;
        session.resize(req.rows, req.cols)?;

        Ok(PtyResizeResponse {
            session_id: req.session_id,
            rows: req.rows,
            cols: req.cols,
        })
    }

    /// Terminates and closes an active PTY session.
    pub async fn close(&self, req: PtyCloseRequest) -> Result<PtyCloseResponse, AgentError> {
        let session_arc = {
            let guard = self.sessions.read().await;
            guard.get(&req.session_id).cloned().ok_or_else(|| {
                AgentError::InvalidInput(format!("PTY session '{}' not found", req.session_id))
            })?
        };

        let session = session_arc.lock().await;
        let username = session.username.clone();
        let _ = session.kill(req.force);
        let exit_code = session.handle.exit_code();
        drop(session);

        self.remove_session(&req.session_id, &username).await;

        info!(
            "Closed PTY session {} for tenant '{}'",
            req.session_id, username
        );

        Ok(PtyCloseResponse {
            session_id: req.session_id,
            closed: true,
            exit_code,
        })
    }

    /// Lists active PTY sessions, optionally filtered by username.
    pub async fn list_sessions(&self, req: PtyListRequest) -> Result<PtyListResponse, AgentError> {
        let guard = self.sessions.read().await;
        let mut result = Vec::new();

        for session_arc in guard.values() {
            let session = session_arc.lock().await;
            if let Some(ref target_user) = req.username
                && &session.username != target_user
            {
                continue;
            }
            result.push(session.to_dto());
        }

        Ok(PtyListResponse { sessions: result })
    }

    /// Reaps inactive or dead sessions that exceeded the idle timeout.
    pub async fn cleanup_idle_sessions(&self) -> usize {
        let timeout = self.config.idle_timeout;
        let mut to_remove = Vec::new();

        {
            let guard = self.sessions.read().await;
            for (id, session_arc) in guard.iter() {
                let session = session_arc.lock().await;
                if !session.is_alive() || session.is_idle(timeout) {
                    to_remove.push((id.clone(), session.username.clone(), session.handle.pid()));
                }
            }
        }

        let count = to_remove.len();
        for (id, user, pid) in to_remove {
            info!(
                "Reaping idle/dead PTY session {} for user '{}' (PID: {})",
                id, user, pid
            );
            if let Some(session_arc) = self.sessions.read().await.get(&id).cloned() {
                let session = session_arc.lock().await;
                let _ = session.kill(true);
            }
            self.remove_session(&id, &user).await;
        }

        count
    }

    /// Internal helper removing session from state maps.
    async fn remove_session(&self, session_id: &str, username: &str) {
        {
            let mut sessions_guard = self.sessions.write().await;
            sessions_guard.remove(session_id);
        }

        {
            let mut tenant_guard = self.tenant_sessions.write().await;
            if let Some(set) = tenant_guard.get_mut(username) {
                set.remove(session_id);
                if set.is_empty() {
                    tenant_guard.remove(username);
                }
            }
        }
    }
}
