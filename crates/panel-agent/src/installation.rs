//! Opt-in typed installation logs. A disconnected observer never kills APT.

use crate::{AgentError, IpcStream, system::SystemRunner};
use panel_proto::{InstallOperation, InstallationProgress, ResponsePayload, RpcStream};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
};

static PACKAGE_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Both pipes are drained concurrently, with bounded capture for command diagnostics.
pub(crate) fn run_streamed_command(
    mut command: Command,
    binary: &str,
    on_output: &(dyn Fn(&str) + Send + Sync),
) -> Result<Output, AgentError> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::scope(|scope| {
        let read = |pipe: Option<Box<dyn Read + Send>>| -> std::io::Result<Vec<u8>> {
            let mut captured = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut chunk = [0; 2048];
                loop {
                    let count = pipe.read(&mut chunk)?;
                    if count == 0 {
                        break;
                    }
                    on_output(&String::from_utf8_lossy(&chunk[..count]));
                    captured.extend_from_slice(&chunk[..count]);
                    if captured.len() > 1024 * 1024 {
                        captured.drain(..captured.len() - 1024 * 1024);
                    }
                }
            }
            Ok(captured)
        };
        let out = scope.spawn(move || read(stdout.map(|p| Box::new(p) as Box<dyn Read + Send>)));
        let err = scope.spawn(move || read(stderr.map(|p| Box::new(p) as Box<dyn Read + Send>)));
        let status = child.wait()?;
        let stdout = out
            .join()
            .map_err(|_| AgentError::Internal("stdout reader failed".into()))??;
        let stderr = err
            .join()
            .map_err(|_| AgentError::Internal("stderr reader failed".into()))??;
        if !status.success() {
            return Err(AgentError::CommandFailed {
                binary: binary.into(),
                code: status.code(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            });
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    })
}

struct ProgressRunner {
    inner: Arc<dyn SystemRunner>,
    tx: tokio::sync::mpsc::Sender<String>,
}

impl ProgressRunner {
    fn emit(&self, text: &str) {
        for chunk in text.as_bytes().chunks(4096) {
            let text = String::from_utf8_lossy(chunk).replace('\r', "\n");
            if !text.trim().is_empty() {
                let _ = self.tx.blocking_send(text);
            }
        }
    }
}

impl SystemRunner for ProgressRunner {
    fn operating_system(&self) -> Result<panel_core::platform::LinuxPlatform, AgentError> {
        self.inner.operating_system()
    }
    fn run_command_with_input(
        &self,
        binary: &str,
        args: &[&str],
        input: &[u8],
    ) -> Result<Output, AgentError> {
        self.inner.run_command_with_input(binary, args, input)
    }
    fn run_command(&self, binary: &str, args: &[&str]) -> Result<Output, AgentError> {
        // Only package-manager/service output is exposed, never SQL or credentials.
        if [
            "dnf",
            "rpm",
            "rpmkeys",
            "apt-get",
            "apt-cache",
            "add-apt-repository",
            "systemctl",
            "dpkg-query",
        ]
        .contains(&binary)
        {
            self.emit(&format!("$ {binary} {}", args.join(" ")));
            let result = self
                .inner
                .run_command_streamed(binary, args, &|text| self.emit(text));
            self.emit(if result.is_ok() {
                "Perintah selesai."
            } else {
                "Perintah gagal; lihat keluaran di atas."
            });
            result
        } else {
            self.inner.run_command(binary, args)
        }
    }
    fn create_dir_all(&self, p: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError> {
        self.inner.create_dir_all(p, mode, uid, gid)
    }
    fn set_permissions(&self, p: &Path, mode: u32) -> Result<(), AgentError> {
        self.inner.set_permissions(p, mode)
    }
    fn set_ownership(&self, p: &Path, uid: u32, gid: u32) -> Result<(), AgentError> {
        self.inner.set_ownership(p, uid, gid)
    }
    fn set_facl(&self, p: &Path, user: &str, permissions: &str) -> Result<(), AgentError> {
        self.inner.set_facl(p, user, permissions)
    }
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), AgentError> {
        self.inner.move_path(from, to)
    }
    fn create_symlink(&self, original: &Path, link: &Path) -> Result<(), AgentError> {
        self.inner.create_symlink(original, link)
    }
    fn read_link(&self, p: &Path) -> Result<PathBuf, AgentError> {
        self.inner.read_link(p)
    }
    fn remove_dir_all(&self, p: &Path) -> Result<(), AgentError> {
        self.inner.remove_dir_all(p)
    }
    fn path_exists(&self, p: &Path) -> bool {
        self.inner.path_exists(p)
    }
    fn write_file(&self, p: &Path, content: &[u8], mode: u32) -> Result<(), AgentError> {
        self.inner.write_file(p, content, mode)
    }
    fn read_file(&self, p: &Path) -> Result<Vec<u8>, AgentError> {
        self.inner.read_file(p)
    }
    fn remove_file(&self, p: &Path) -> Result<(), AgentError> {
        self.inner.remove_file(p)
    }
    fn read_dir(&self, p: &Path) -> Result<Vec<PathBuf>, AgentError> {
        self.inner.read_dir(p)
    }
}

pub(crate) async fn run(
    inner: Arc<dyn SystemRunner>,
    operation: InstallOperation,
    rpc: &mut RpcStream<IpcStream>,
    request_id: &str,
) -> Result<ResponsePayload, AgentError> {
    let _guard = PACKAGE_INSTALL_LOCK.lock().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::task::spawn_blocking(move || {
        let runner = ProgressRunner { inner, tx };
        match operation {
            InstallOperation::EmailServer(config) => {
                crate::email::install(&runner, config).map(ResponsePayload::Email)
            }
            InstallOperation::CloudflareConnector => {
                crate::cloudflare::install_connector(&runner).map(ResponsePayload::Cloudflare)
            }
            InstallOperation::PhpRepository => {
                crate::php::setup_php_repository(&runner).map(ResponsePayload::PhpRepoSetup)
            }
            InstallOperation::PhpRuntime(req) => {
                crate::php::install_php_runtime(&runner, &req.version)
                    .map(ResponsePayload::PhpRuntimeInstalled)
            }
            InstallOperation::DatabaseServer(req) => {
                crate::databases::install_database_server(&runner, &req)
                    .map(ResponsePayload::DatabaseServerInstalled)
            }
            InstallOperation::DatabaseWebTools => {
                crate::database_web::install(&runner).map(ResponsePayload::DatabaseWebStatus)
            }
        }
    });
    let mut connected = true;
    while let Some(message) = rx.recv().await {
        if connected {
            let response = panel_proto::RpcResponse::ok(
                request_id,
                ResponsePayload::InstallationProgress(InstallationProgress { message }),
            );
            connected = rpc.send_response(&response).await.is_ok();
        }
    }
    // Drain the stream and let the package manager finish even if the client left.
    task.await
        .map_err(|e| AgentError::Internal(format!("Installation task failed: {e}")))?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    #[test]
    fn child_output_fixture() {
        let Some(release) = std::env::var_os("VPANEL_LOG_TEST_RELEASE") else {
            return;
        };
        println!("OUTPUT_BEFORE_COMPLETION");
        std::io::stdout().flush().unwrap();
        eprintln!("STDERR_DIAGNOSTIC");
        std::io::stderr().flush().unwrap();
        for _ in 0..500 {
            if Path::new(&release).exists() {
                if std::env::var_os("VPANEL_LOG_TEST_FAIL").is_some() {
                    std::process::exit(7);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("reader did not receive output before child completion");
    }

    #[test]
    fn streams_both_pipes_before_exit_and_preserves_failure() {
        for fail in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let release = temp.path().join("release");
            let output = Mutex::new(String::new());
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "installation::tests::child_output_fixture",
                    "--nocapture",
                ])
                .env("VPANEL_LOG_TEST_RELEASE", &release);
            if fail {
                command.env("VPANEL_LOG_TEST_FAIL", "1");
            }
            let result = run_streamed_command(command, "test-child", &|text| {
                output.lock().unwrap().push_str(text);
                if text.contains("OUTPUT_BEFORE_COMPLETION") {
                    std::fs::write(&release, b"go").unwrap();
                }
            });
            assert!(
                release.exists(),
                "stream must be consumed while child is still running"
            );
            assert!(output.lock().unwrap().contains("STDERR_DIAGNOSTIC"));
            if fail {
                assert!(matches!(
                    result,
                    Err(AgentError::CommandFailed { code: Some(7), .. })
                ));
            } else {
                assert!(result.unwrap().status.success());
            }
        }
    }
}
