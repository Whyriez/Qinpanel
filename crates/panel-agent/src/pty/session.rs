//! Active PTY session instance tracking process state, dimensions, and activity.

use super::backend::PtyHandle;
use crate::AgentError;
use panel_proto::PtySessionInfoDto;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An active pseudo-terminal session assigned to a tenant.
pub struct PtySession {
    pub id: String,
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    pub created_at: i64,
    pub last_active_at: Arc<AtomicI64>,
    pub handle: Box<dyn PtyHandle>,
}

impl PtySession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        username: String,
        uid: u32,
        gid: u32,
        cwd: PathBuf,
        rows: u16,
        cols: u16,
        handle: Box<dyn PtyHandle>,
    ) -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let now_secs = now_ms / 1000;

        Self {
            id,
            username,
            uid,
            gid,
            cwd,
            rows,
            cols,
            created_at: now_secs,
            last_active_at: Arc::new(AtomicI64::new(now_ms)),
            handle,
        }
    }

    /// Updates the last active timestamp to current time.
    fn touch(&self) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.last_active_at.store(now_ms, Ordering::Relaxed);
    }

    /// Writes raw input data to the PTY and updates activity.
    pub fn write_input(&self, data: &[u8]) -> Result<usize, AgentError> {
        let bytes_written = self.handle.write_input(data)?;
        if bytes_written > 0 {
            self.touch();
        }
        Ok(bytes_written)
    }

    /// Reads output from the PTY and updates activity if bytes were read.
    pub fn read_output(
        &self,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, bool, Option<i32>), AgentError> {
        let (data, closed, exit_code) = self.handle.read_output(max_bytes)?;
        if !data.is_empty() {
            self.touch();
        }
        Ok((data, closed, exit_code))
    }

    /// Resizes terminal dimensions.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), AgentError> {
        self.handle.resize(rows, cols)?;
        self.rows = rows;
        self.cols = cols;
        self.touch();
        Ok(())
    }

    /// Terminates child process.
    pub fn kill(&self, force: bool) -> Result<(), AgentError> {
        self.handle.kill(force)
    }

    /// Checks if child process is still running.
    pub fn is_alive(&self) -> bool {
        self.handle.is_alive()
    }

    /// Checks if session has been inactive longer than the given idle duration.
    pub fn is_idle(&self, timeout: Duration) -> bool {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let last_ms = self.last_active_at.load(Ordering::Relaxed);
        let elapsed = Duration::from_millis(now_ms.saturating_sub(last_ms) as u64);
        elapsed >= timeout
    }

    /// Returns session information DTO.
    pub fn to_dto(&self) -> PtySessionInfoDto {
        PtySessionInfoDto {
            session_id: self.id.clone(),
            username: self.username.clone(),
            uid: self.uid,
            pid: self.handle.pid(),
            cwd: self.cwd.display().to_string(),
            rows: self.rows,
            cols: self.cols,
            created_at: self.created_at,
            last_active_at: self.last_active_at.load(Ordering::Relaxed) / 1000,
            is_alive: self.is_alive(),
        }
    }
}
