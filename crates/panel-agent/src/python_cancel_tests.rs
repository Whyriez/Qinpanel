#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::system::{MockAction, MockSystemRunner};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

const TARGET: &str = "/opt/panel/runtimes/python/3.15.0";
fn request() -> PythonRuntimeInstallRequest {
    PythonRuntimeInstallRequest {
        version: "3.15.0".into(),
        base_dir: None,
    }
}

struct PendingPython {
    inner: MockSystemRunner,
    started: AtomicBool,
}
macro_rules! delegate {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $result:ty;)*) => {
        $(fn $name(&self $(, $arg: $ty)*) -> $result { self.inner.$name($($arg),*) })*
    };
}
impl SystemRunner for PendingPython {
    fn operating_system(&self) -> Result<panel_core::platform::LinuxPlatform, AgentError> {
        self.inner.operating_system()
    }
    delegate! {
        fn run_command(&self, binary: &str, args: &[&str]) -> Result<std::process::Output, AgentError>;
        fn create_dir_all(&self, path: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError>;
        fn set_permissions(&self, path: &Path, mode: u32) -> Result<(), AgentError>;
        fn set_ownership(&self, path: &Path, uid: u32, gid: u32) -> Result<(), AgentError>;
        fn set_facl(&self, path: &Path, user: &str, permissions: &str) -> Result<(), AgentError>;
        fn move_path(&self, from: &Path, to: &Path) -> Result<(), AgentError>;
        fn create_symlink(&self, original: &Path, link: &Path) -> Result<(), AgentError>;
        fn read_link(&self, path: &Path) -> Result<PathBuf, AgentError>;
        fn remove_dir_all(&self, path: &Path) -> Result<(), AgentError>;
        fn path_exists(&self, path: &Path) -> bool;
        fn write_file(&self, path: &Path, content: &[u8], mode: u32) -> Result<(), AgentError>;
        fn read_file(&self, path: &Path) -> Result<Vec<u8>, AgentError>;
        fn remove_file(&self, path: &Path) -> Result<(), AgentError>;
        fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, AgentError>;
    }
    fn run_cancellable_python(
        &self,
        args: &[&str],
        cancel: &CancellationToken,
    ) -> Result<std::process::Output, AgentError> {
        self.inner.add_existing_path(TARGET);
        self.started.store(true, Ordering::SeqCst);
        while !cancel.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.inner.run_command(crate::uv::UV_BIN, args)
    }
}

#[tokio::test]
async fn disconnect_stops_python_before_cleaning_only_its_new_target() {
    let runner = Arc::new(PendingPython {
        inner: MockSystemRunner::new(),
        started: AtomicBool::new(false),
    });
    runner.inner.add_existing_path(crate::uv::UV_BIN);
    runner
        .inner
        .add_existing_path("/opt/panel/runtimes/python/3.14.8");
    let copy = Arc::clone(&runner);
    let (client, stream) = tokio::io::duplex(1024);
    let handle = tokio::spawn(async move {
        let mut rpc = panel_proto::RpcStream::new(stream);
        install_python_runtime_connected(copy, request(), &mut rpc).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !runner.started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(client);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), handle)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let removed: Vec<_> = runner
        .inner
        .recorded_actions()
        .into_iter()
        .filter_map(|action| match action {
            MockAction::RemoveDir { path } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(removed, [PathBuf::from(TARGET)]);
    assert!(runner.inner.path_exists(Path::new(crate::uv::UV_BIN)));
    assert!(
        runner
            .inner
            .path_exists(Path::new("/opt/panel/runtimes/python/3.14.8"))
    );
    assert!(PYTHON_INSTALL_LOCK.try_lock().is_ok());
}

#[test]
fn cancelled_or_existing_python_never_starts_or_removes_a_runtime() {
    let runner = MockSystemRunner::new();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(install_python_runtime_cancellable(&runner, &request(), &cancel).is_err());
    assert!(runner.recorded_actions().is_empty());
    runner.add_existing_path(TARGET);
    assert!(
        install_python_runtime_cancellable(&runner, &request(), &CancellationToken::new()).is_err()
    );
    assert!(runner.recorded_actions().is_empty());
    assert!(runner.path_exists(Path::new(TARGET)));
}
