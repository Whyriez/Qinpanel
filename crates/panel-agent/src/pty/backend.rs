#![allow(unsafe_code)]

//! Low-level PTY allocation and process execution backend.
//!
//! Provides the underlying OS pseudo-terminal interface:
//! - On Linux/Unix: Real PTY allocation via `openpty`, `fork`, `no_new_privs`, rlimit,
//!   privilege dropping (`setresgid`/`setresuid`), cgroup slice attachment, and login shell execution.
//! - On non-Unix / Test: Mock PTY engine providing interactive shell simulation for portable testing.

use crate::AgentError;
use std::path::PathBuf;
use std::sync::Mutex;

/// Configuration parameters for spawning a new PTY session.
#[derive(Debug, Clone)]
pub struct PtySpawnConfig {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    pub cwd: PathBuf,
    pub shell: String,
    pub rows: u16,
    pub cols: u16,
    pub term: String,
    pub slice_name: Option<String>,
    pub cgroup_base: PathBuf,
}

/// Abstract handle to an active PTY session.
pub trait PtyHandle: Send + Sync {
    /// Returns the PID of the spawned process.
    fn pid(&self) -> u32;

    /// Writes raw input data to the PTY master.
    fn write_input(&self, data: &[u8]) -> Result<usize, AgentError>;

    /// Reads output data from the PTY master (non-blocking).
    /// Returns (data, closed, exit_code).
    fn read_output(&self, max_bytes: usize) -> Result<(Vec<u8>, bool, Option<i32>), AgentError>;

    /// Resizes the terminal dimensions.
    fn resize(&self, rows: u16, cols: u16) -> Result<(), AgentError>;

    /// Terminates the child process.
    fn kill(&self, force: bool) -> Result<(), AgentError>;

    /// Checks if the process is still running.
    fn is_alive(&self) -> bool;

    /// Returns the process exit code, if terminated.
    fn exit_code(&self) -> Option<i32>;
}

/// Backend interface for creating PTY handles.
pub trait PtyBackend: Send + Sync {
    fn spawn(&self, config: &PtySpawnConfig) -> Result<Box<dyn PtyHandle>, AgentError>;
}

// ---------------------------------------------------------------------------
// Unix Implementation (Linux Real PTY)
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub struct UnixPtyBackend;

#[cfg(unix)]
impl UnixPtyBackend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(unix)]
impl Default for UnixPtyBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
struct UnixPtyHandle {
    pid: u32,
    master_fd: i32,
    exit_status: Mutex<Option<i32>>,
}

#[cfg(unix)]
impl UnixPtyHandle {
    fn check_exit_status(&self) -> Option<i32> {
        let mut guard = match self.exit_status.lock() {
            Ok(g) => g,
            Err(_) => return None,
        };
        if let Some(code) = *guard {
            return Some(code);
        }

        let mut status = 0;
        // Reason for unsafe: POSIX waitpid syscall to query child process status non-blocking.
        #[allow(unsafe_code)]
        let res = unsafe { libc::waitpid(self.pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if res > 0 {
            let code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                128 + libc::WTERMSIG(status)
            } else {
                1
            };
            *guard = Some(code);
            Some(code)
        } else if res < 0 {
            // ECHILD or invalid PID means process is already gone
            let code = 0;
            *guard = Some(code);
            Some(code)
        } else {
            None
        }
    }
}

#[cfg(unix)]
impl PtyHandle for UnixPtyHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn write_input(&self, data: &[u8]) -> Result<usize, AgentError> {
        if data.is_empty() {
            return Ok(0);
        }
        // Reason for unsafe: POSIX write syscall to write data to PTY master descriptor.
        #[allow(unsafe_code)]
        let written = unsafe {
            libc::write(
                self.master_fd,
                data.as_ptr() as *const libc::c_void,
                data.len(),
            )
        };

        if written < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EAGAIN)
                || err.raw_os_error() == Some(libc::EWOULDBLOCK)
            {
                return Ok(0);
            }
            return Err(AgentError::Io(err));
        }

        Ok(written as usize)
    }

    fn read_output(&self, max_bytes: usize) -> Result<(Vec<u8>, bool, Option<i32>), AgentError> {
        let max_len = max_bytes.clamp(1, 65536);
        let mut buffer = vec![0u8; max_len];

        // Reason for unsafe: POSIX read syscall from non-blocking PTY master descriptor.
        #[allow(unsafe_code)]
        let read_bytes = unsafe {
            libc::read(
                self.master_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                max_len,
            )
        };

        if read_bytes < 0 {
            let err = std::io::Error::last_os_error();
            let errno = err.raw_os_error();
            if errno == Some(libc::EAGAIN) || errno == Some(libc::EWOULDBLOCK) {
                // No data currently available
                let exit = self.check_exit_status();
                return Ok((Vec::new(), exit.is_some(), exit));
            }
            if errno == Some(libc::EIO) {
                // EIO is standard Linux behavior when slave end of PTY is closed (child exited)
                let exit = self.check_exit_status().or(Some(0));
                return Ok((Vec::new(), true, exit));
            }
            return Err(AgentError::Io(err));
        }

        if read_bytes == 0 {
            // EOF reached
            let exit = self.check_exit_status().or(Some(0));
            return Ok((Vec::new(), true, exit));
        }

        buffer.truncate(read_bytes as usize);
        let exit = self.check_exit_status();
        Ok((buffer, exit.is_some(), exit))
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<(), AgentError> {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // Reason for unsafe: POSIX ioctl TIOCSWINSZ to resize terminal dimensions.
        #[allow(unsafe_code)]
        let res = unsafe { libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &ws) };
        if res < 0 {
            return Err(AgentError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn kill(&self, force: bool) -> Result<(), AgentError> {
        let sig = if force { libc::SIGKILL } else { libc::SIGHUP };
        // Reason for unsafe: POSIX kill syscall to terminate the child process group.
        #[allow(unsafe_code)]
        unsafe {
            // Send signal to process group first
            let _ = libc::kill(-(self.pid as libc::pid_t), sig);
            let _ = libc::kill(self.pid as libc::pid_t, sig);
        };
        self.check_exit_status();
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.check_exit_status().is_none()
    }

    fn exit_code(&self) -> Option<i32> {
        self.check_exit_status()
    }
}

#[cfg(unix)]
impl Drop for UnixPtyHandle {
    fn drop(&mut self) {
        // Ensure child is terminated and master fd is closed
        let _ = self.kill(true);
        // Reason for unsafe: POSIX close syscall to release master PTY file descriptor.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

#[cfg(unix)]
impl PtyBackend for UnixPtyBackend {
    fn spawn(&self, config: &PtySpawnConfig) -> Result<Box<dyn PtyHandle>, AgentError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let mut master_fd: libc::c_int = -1;
        let mut slave_fd: libc::c_int = -1;

        let winsize = libc::winsize {
            ws_row: config.rows,
            ws_col: config.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        // Reason for unsafe: POSIX openpty to allocate pseudo-terminal master/slave pair.
        #[allow(unsafe_code)]
        let openpty_res = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &winsize,
            )
        };

        if openpty_res < 0 {
            return Err(AgentError::Io(std::io::Error::last_os_error()));
        }

        // Set master FD to non-blocking
        // Reason for unsafe: POSIX fcntl syscall to set non-blocking flag on master FD.
        #[allow(unsafe_code)]
        unsafe {
            let flags = libc::fcntl(master_fd, libc::F_GETFL, 0);
            if flags >= 0 {
                libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        let c_cwd = CString::new(config.cwd.as_os_str().as_bytes())
            .map_err(|e| AgentError::InvalidInput(format!("Invalid cwd: {e}")))?;
        let c_shell = CString::new(config.shell.as_bytes())
            .map_err(|e| AgentError::InvalidInput(format!("Invalid shell: {e}")))?;
        let c_user = CString::new(config.username.as_bytes())
            .map_err(|e| AgentError::InvalidInput(format!("Invalid username: {e}")))?;

        // Prepare environment variables
        let env_home = CString::new(format!("HOME=/home/{}", config.username)).map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable HOME".to_string())
        })?;
        let env_user = CString::new(format!("USER={}", config.username)).map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable USER".to_string())
        })?;
        let env_logname = CString::new(format!("LOGNAME={}", config.username)).map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable LOGNAME".to_string())
        })?;
        let env_shell = CString::new(format!("SHELL={}", config.shell)).map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable SHELL".to_string())
        })?;
        let env_term = CString::new(format!("TERM={}", config.term)).map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable TERM".to_string())
        })?;
        let env_path = CString::new("PATH=/usr/local/bin:/usr/bin:/bin").map_err(|_| {
            AgentError::InvalidInput("Invalid environment variable PATH".to_string())
        })?;

        let envp = [
            env_home.as_ptr(),
            env_user.as_ptr(),
            env_logname.as_ptr(),
            env_shell.as_ptr(),
            env_term.as_ptr(),
            env_path.as_ptr(),
            std::ptr::null(),
        ];

        let argv0 = CString::new("-bash").unwrap_or_else(|_| c_shell.clone());
        let argv = [argv0.as_ptr(), std::ptr::null()];

        let target_uid = config.uid;
        let target_gid = config.gid;

        // Reason for unsafe: POSIX fork syscall to spawn isolated tenant login shell.
        #[allow(unsafe_code)]
        let pid = unsafe { libc::fork() };

        if pid < 0 {
            let err = std::io::Error::last_os_error();
            #[allow(unsafe_code)]
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            return Err(AgentError::Io(err));
        }

        if pid == 0 {
            // Child process execution
            // Reason for unsafe: Child initialization, setting controlling terminal,
            // dropping privileges, and executing login shell.
            #[allow(unsafe_code)]
            unsafe {
                libc::close(master_fd);

                // Create new session leader
                libc::setsid();

                // Set controlling terminal
                libc::ioctl(slave_fd, libc::TIOCSCTTY, 0);

                // Duplicate slave FD to standard descriptors
                libc::dup2(slave_fd, libc::STDIN_FILENO);
                libc::dup2(slave_fd, libc::STDOUT_FILENO);
                libc::dup2(slave_fd, libc::STDERR_FILENO);

                if slave_fd > libc::STDERR_FILENO {
                    libc::close(slave_fd);
                }

                // Hardening: PR_SET_NO_NEW_PRIVS ensures child cannot gain privileges
                libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);

                // Hardening: Resource limits (rlimit)
                let rlim_nproc = libc::rlimit {
                    rlim_cur: 256,
                    rlim_max: 512,
                };
                libc::setrlimit(libc::RLIMIT_NPROC, &rlim_nproc);

                let rlim_nofile = libc::rlimit {
                    rlim_cur: 1024,
                    rlim_max: 2048,
                };
                libc::setrlimit(libc::RLIMIT_NOFILE, &rlim_nofile);

                // Change working directory to confined folder
                if libc::chdir(c_cwd.as_ptr()) != 0 {
                    libc::_exit(126);
                }

                // Drop privileges to tenant UID/GID
                libc::initgroups(c_user.as_ptr(), target_gid);
                libc::setresgid(target_gid, target_gid, target_gid);
                libc::setresuid(target_uid, target_uid, target_uid);

                // Execute login shell
                libc::execve(c_shell.as_ptr(), argv.as_ptr(), envp.as_ptr());

                // If execve fails, exit child immediately
                libc::_exit(127);
            }
        }

        // Parent process execution
        // Reason for unsafe: Closing slave FD in parent process.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(slave_fd);
        }

        let child_pid = pid as u32;

        // Attach child PID to tenant cgroup slice if configured
        if let Some(slice) = &config.slice_name {
            let slice_procs = config.cgroup_base.join(slice).join("cgroup.procs");
            if slice_procs.exists() {
                let _ = std::fs::write(&slice_procs, format!("{}\n", child_pid));
            }
        }

        Ok(Box::new(UnixPtyHandle {
            pid: child_pid,
            master_fd,
            exit_status: Mutex::new(None),
        }))
    }
}

// ---------------------------------------------------------------------------
// Mock Implementation (Cross-platform testing & Windows support)
// ---------------------------------------------------------------------------

/// In-memory mock PTY handle for testing and non-Unix compilation.
pub struct MockPtyHandle {
    pub pid: u32,
    pub rows: Mutex<u16>,
    pub cols: Mutex<u16>,
    pub buffer: Mutex<Vec<u8>>,
    pub is_alive: Mutex<bool>,
    pub exit_code: Mutex<Option<i32>>,
}

impl MockPtyHandle {
    pub fn new(pid: u32, rows: u16, cols: u16) -> Self {
        let initial_banner = b"Welcome to Qinpanel Shell (Simulated PTY)\n$ ".to_vec();
        Self {
            pid,
            rows: Mutex::new(rows),
            cols: Mutex::new(cols),
            buffer: Mutex::new(initial_banner),
            is_alive: Mutex::new(true),
            exit_code: Mutex::new(None),
        }
    }
}

impl PtyHandle for MockPtyHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn write_input(&self, data: &[u8]) -> Result<usize, AgentError> {
        let mut alive = self
            .is_alive
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        if !*alive {
            return Err(AgentError::InvalidInput(
                "Process already terminated".into(),
            ));
        }

        let mut buf = self
            .buffer
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;

        // Simulate echo and basic command handling
        buf.extend_from_slice(data);

        let input_str = String::from_utf8_lossy(data);
        if input_str.contains("exit") {
            *alive = false;
            let mut code = self
                .exit_code
                .lock()
                .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
            *code = Some(0);
            buf.extend_from_slice(b"\nlogout\n");
        } else if input_str.contains('\n') || input_str.contains('\r') {
            buf.extend_from_slice(b"\n$ ");
        }

        Ok(data.len())
    }

    fn read_output(&self, max_bytes: usize) -> Result<(Vec<u8>, bool, Option<i32>), AgentError> {
        let mut buf = self
            .buffer
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        let alive = self
            .is_alive
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        let code = self
            .exit_code
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;

        let to_read = max_bytes.min(buf.len());
        let chunk: Vec<u8> = buf.drain(..to_read).collect();
        let closed = !*alive && buf.is_empty();

        Ok((chunk, closed, *code))
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<(), AgentError> {
        let mut r = self
            .rows
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        let mut c = self
            .cols
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        *r = rows;
        *c = cols;
        Ok(())
    }

    fn kill(&self, _force: bool) -> Result<(), AgentError> {
        let mut alive = self
            .is_alive
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        *alive = false;
        let mut code = self
            .exit_code
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        if code.is_none() {
            *code = Some(137);
        }
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.is_alive.lock().map(|g| *g).unwrap_or(false)
    }

    fn exit_code(&self) -> Option<i32> {
        self.exit_code.lock().ok().and_then(|g| *g)
    }
}

/// Mock backend for testing PTY sessions without requiring root or real Linux PTYs.
pub struct MockPtyBackend {
    next_pid: Mutex<u32>,
}

impl MockPtyBackend {
    pub fn new() -> Self {
        Self {
            next_pid: Mutex::new(20000),
        }
    }
}

impl Default for MockPtyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyBackend for MockPtyBackend {
    fn spawn(&self, config: &PtySpawnConfig) -> Result<Box<dyn PtyHandle>, AgentError> {
        let mut pid_lock = self
            .next_pid
            .lock()
            .map_err(|_| AgentError::Internal("Lock poisoned".into()))?;
        let pid = *pid_lock;
        *pid_lock += 1;

        Ok(Box::new(MockPtyHandle::new(pid, config.rows, config.cols)))
    }
}
