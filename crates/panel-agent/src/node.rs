//! Node.js Runtime operations in panel-agent: list installed versions,
//! download & verify SHA256 tarball, extract to `/opt/panel/runtimes/node/<ver>`,
//! corepack enable (pnpm & yarn), and remove runtime.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_proto::{
    NodeInstalledVersionDto, NodeRuntimeActionResponse, NodeRuntimeCorepackRequest,
    NodeRuntimeInstallRequest, NodeRuntimeListResponse, NodeRuntimeRemoveRequest,
};
use panel_runtime::node::{
    DEFAULT_NODE_BASE_DIR, DEFAULT_NODE_MIRROR, enable_corepack_shims, extract_node_tarball,
    find_shasum256_in_manifest, list_installed_node_versions, normalize_node_version,
    raw_node_version_tag, verify_sha256,
};
use std::path::Path;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub(crate) static NODE_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A disconnected Node installer must stop its download before releasing the
/// installation lock. This runs no blocking download on an async executor thread.
pub async fn install_node_runtime_connected<T>(
    runner: std::sync::Arc<dyn SystemRunner>,
    req: NodeRuntimeInstallRequest,
    rpc: &mut panel_proto::RpcStream<T>,
) -> Result<NodeRuntimeActionResponse, AgentError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    run_node_operation_connected(rpc, move |cancel| {
        install_node_runtime_cancellable(runner.as_ref(), &req, &cancel)
    })
    .await
}

async fn run_node_operation_connected<T, F>(
    rpc: &mut panel_proto::RpcStream<T>,
    operation: F,
) -> Result<NodeRuntimeActionResponse, AgentError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnOnce(CancellationToken) -> Result<NodeRuntimeActionResponse, AgentError> + Send + 'static,
{
    crate::runtime_install::connected(rpc, &NODE_INSTALL_LOCK, operation).await
}

/// Resolves the base installation directory for Node.js runtimes.
/// Priority:
/// 1. Explicit override passed in request
/// 2. `PANEL_NODE_BASE_DIR` environment variable
/// 3. In test runner environments without root permissions, fall back to temporary directory
/// 4. Default: `/opt/panel/runtimes/node`
pub fn get_node_base_dir(override_dir: Option<&str>) -> std::path::PathBuf {
    if let Some(d) = override_dir {
        return std::path::PathBuf::from(d);
    }
    if let Some(env_d) = std::env::var("PANEL_NODE_BASE_DIR")
        .ok()
        .filter(|d| !d.trim().is_empty())
    {
        return std::path::PathBuf::from(env_d);
    }

    let is_test_runner = std::env::var("PANEL_TEST_MODE").is_ok()
        || std::env::var("RUST_TEST_THREADS").is_ok()
        || std::env::current_exe()
            .map(|p| {
                let s = p.to_string_lossy();
                s.contains("/deps/")
                    || s.contains("\\deps\\")
                    || p.file_name()
                        .map(|n| n.to_string_lossy().contains("test"))
                        .unwrap_or(false)
            })
            .unwrap_or(false);

    if is_test_runner {
        return std::env::temp_dir().join("panel_runtimes_node");
    }

    std::path::PathBuf::from(DEFAULT_NODE_BASE_DIR)
}

/// Lists all installed Node.js versions found in the base directory.
pub fn list_node_runtimes(
    runner: &dyn SystemRunner,
    base_dir_override: Option<&str>,
) -> Result<NodeRuntimeListResponse, AgentError> {
    let base_dir = get_node_base_dir(base_dir_override);
    let base_dir_str = base_dir.to_string_lossy().to_string();

    if !runner.path_exists(&base_dir) {
        return Ok(NodeRuntimeListResponse {
            base_dir: base_dir_str,
            versions: Vec::new(),
        });
    }

    let installed_versions = list_installed_node_versions(&base_dir).map_err(|e| {
        AgentError::Internal(format!("Gagal mendata runtime Node.js terpasang: {}", e))
    })?;

    let dtos = installed_versions
        .into_iter()
        .map(|v| NodeInstalledVersionDto {
            version: v.version,
            raw_version: v.raw_version,
            path: v.path,
            node_binary: v.node_binary,
            npm_binary: v.npm_binary,
            npx_binary: v.npx_binary,
            corepack_binary: v.corepack_binary,
            pnpm_binary: v.pnpm_binary,
            yarn_binary: v.yarn_binary,
            corepack_enabled: v.corepack_enabled,
            is_lts: v.is_lts,
            lts_codename: v.lts_codename,
        })
        .collect();

    Ok(NodeRuntimeListResponse {
        base_dir: base_dir_str.to_string(),
        versions: dtos,
    })
}

/// Installs a Node.js runtime version:
/// - downloads official tarball from mirror or reads from offline path
/// - validates SHA256 checksum against official `SHASUMS256.txt` manifest
/// - extracts archive safely to `/opt/panel/runtimes/node/<ver>`
/// - enables corepack shims (`pnpm`, `yarn`)
pub fn install_node_runtime(
    runner: &dyn SystemRunner,
    req: &NodeRuntimeInstallRequest,
) -> Result<NodeRuntimeActionResponse, AgentError> {
    install_node_runtime_cancellable(runner, req, &CancellationToken::new())
}

fn install_node_runtime_cancellable(
    runner: &dyn SystemRunner,
    req: &NodeRuntimeInstallRequest,
    cancel: &CancellationToken,
) -> Result<NodeRuntimeActionResponse, AgentError> {
    ensure_node_install_active(cancel)?;
    let base_dir = get_node_base_dir(req.base_dir.as_deref());

    let norm_ver = normalize_node_version(&req.version);
    if norm_ver.is_empty()
        || norm_ver.contains("..")
        || norm_ver.contains('/')
        || norm_ver.contains('\\')
    {
        return Err(AgentError::InvalidInput(format!(
            "Format versi Node.js '{}' tidak valid",
            req.version
        )));
    }

    let target_dir = base_dir.join(&norm_ver);
    let node_bin = target_dir.join("bin").join("node");
    let node_bin_win = target_dir.join("bin").join("node.exe");

    // Idempotency: if already installed and binary exists, ensure corepack and return ok
    if runner.path_exists(&node_bin) || runner.path_exists(&node_bin_win) {
        if req.enable_corepack {
            let _ = enable_corepack_shims(&target_dir.join("bin"));
        }
        return Ok(NodeRuntimeActionResponse {
            version: norm_ver,
            success: true,
            path: target_dir.to_string_lossy().to_string(),
            message: "Versi Node.js sudah terpasang".to_string(),
        });
    }

    info!(
        "Beginning installation of Node.js v{} into {}",
        norm_ver,
        target_dir.display()
    );

    // Acquire tarball data and verify SHA256
    let tarball_bytes = fetch_tarball_payload(runner, req, &norm_ver, cancel)?;
    ensure_node_install_active(cancel)?;

    // Ensure parent directory exists
    runner.create_dir_all(&base_dir, 0o755, 0, 0)?;

    // Extract archive safely
    extract_node_tarball(&tarball_bytes, &target_dir).map_err(|e| {
        let _ = runner.remove_dir_all(&target_dir);
        AgentError::Internal(format!(
            "Gagal mengekstrak binary Node.js ke {}: {}",
            target_dir.display(),
            e
        ))
    })?;

    if let Err(error) = ensure_node_install_active(cancel) {
        let _ = runner.remove_dir_all(&target_dir);
        return Err(error);
    }

    // Enable corepack shims if requested
    if req.enable_corepack {
        let bin_dir = target_dir.join("bin");
        if let Err(e) = enable_corepack_shims(&bin_dir) {
            warn!("Corepack enablement warning: {}", e);
        }
    }

    info!(
        "Node.js v{} successfully installed at {}",
        norm_ver,
        target_dir.display()
    );

    Ok(NodeRuntimeActionResponse {
        version: norm_ver.clone(),
        success: true,
        path: target_dir.to_string_lossy().to_string(),
        message: format!(
            "Node.js v{} berhasil dipasang",
            raw_node_version_tag(&norm_ver)
        ),
    })
}

/// Removes an installed Node.js runtime version directory.
/// Note: Caller must verify that the version is NOT in use by any site before calling!
pub fn remove_node_runtime(
    runner: &dyn SystemRunner,
    req: &NodeRuntimeRemoveRequest,
) -> Result<NodeRuntimeActionResponse, AgentError> {
    let base_dir = get_node_base_dir(req.base_dir.as_deref());

    let norm_ver = normalize_node_version(&req.version);
    if norm_ver.is_empty()
        || norm_ver.contains("..")
        || norm_ver.contains('/')
        || norm_ver.contains('\\')
    {
        return Err(AgentError::InvalidInput(format!(
            "Format versi Node.js '{}' tidak valid",
            req.version
        )));
    }

    let target_dir = base_dir.join(&norm_ver);

    if runner.path_exists(&target_dir) {
        runner.remove_dir_all(&target_dir)?;
        info!("Removed Node.js runtime at {}", target_dir.display());
    } else {
        info!(
            "Node.js runtime at {} already nonexistent",
            target_dir.display()
        );
    }

    Ok(NodeRuntimeActionResponse {
        version: norm_ver.clone(),
        success: true,
        path: target_dir.to_string_lossy().to_string(),
        message: format!("Node.js v{} berhasil dihapus dari sistem", norm_ver),
    })
}

fn fetch_tarball_payload(
    runner: &dyn SystemRunner,
    req: &NodeRuntimeInstallRequest,
    norm_ver: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, AgentError> {
    if let Some(offline_path) = &req.offline_tarball_path {
        read_offline_tarball(runner, offline_path, req.offline_checksum.as_deref())
    } else {
        let mirror_base = req
            .mirror_url
            .as_deref()
            .unwrap_or(DEFAULT_NODE_MIRROR)
            .trim_end_matches('/');
        let raw_tag = raw_node_version_tag(norm_ver);
        download_mirror_tarball(runner, mirror_base, &raw_tag, cancel)
    }
}

fn read_offline_tarball(
    runner: &dyn SystemRunner,
    offline_path: &str,
    expected_checksum: Option<&str>,
) -> Result<Vec<u8>, AgentError> {
    let p = Path::new(offline_path);
    let bytes = runner.read_file(p).map_err(|e| {
        AgentError::Internal(format!(
            "Gagal membaca arsip offline {}: {}",
            p.display(),
            e
        ))
    })?;

    if let Some(expected) = expected_checksum {
        verify_sha256(&bytes, expected).map_err(|e| {
            AgentError::Internal(format!("Verifikasi checksum arsip offline gagal: {}", e))
        })?;
    }

    Ok(bytes)
}

fn download_mirror_tarball(
    runner: &dyn SystemRunner,
    mirror_base: &str,
    raw_tag: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, AgentError> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let tarball_filename = format!("node-{}-linux-{}.tar.gz", raw_tag, arch);
    let manifest_url = format!("{}/{}/SHASUMS256.txt", mirror_base, raw_tag);
    let tarball_url = format!("{}/{}/{}", mirror_base, raw_tag, tarball_filename);

    let tmp_dir = std::env::temp_dir().join(format!("node-install-{}", uuid::Uuid::new_v4()));
    runner.create_dir_all(&tmp_dir, 0o700, 0, 0)?;

    let manifest_path = tmp_dir.join("SHASUMS256.txt");
    let download_tarball_path = tmp_dir.join(&tarball_filename);

    info!(
        "Downloading Node.js checksum manifest from {}",
        manifest_url
    );
    if let Err(e) = runner.run_cancellable_download(
        &[
            "-fsSL",
            "--max-time",
            "120",
            "-o",
            &manifest_path.to_string_lossy(),
            &manifest_url,
        ],
        cancel,
    ) {
        let _ = runner.remove_dir_all(&tmp_dir);
        return Err(AgentError::Internal(format!(
            "Gagal mengunduh SHASUMS256.txt dari {}: {}",
            manifest_url, e
        )));
    }

    let manifest_bytes = runner.read_file(&manifest_path).map_err(|e| {
        let _ = runner.remove_dir_all(&tmp_dir);
        AgentError::Internal(format!("Gagal membaca manifest checksum: {}", e))
    })?;
    let manifest_str = String::from_utf8_lossy(&manifest_bytes);

    let expected_sha256 =
        find_shasum256_in_manifest(&manifest_str, &tarball_filename).ok_or_else(|| {
            let _ = runner.remove_dir_all(&tmp_dir);
            AgentError::Internal(format!(
                "Filename {} tidak ditemukan di dalam {}",
                tarball_filename, manifest_url
            ))
        })?;

    info!("Downloading Node.js binary tarball from {}", tarball_url);
    if let Err(e) = runner.run_cancellable_download(
        &[
            "-fsSL",
            "--max-time",
            "600",
            "-o",
            &download_tarball_path.to_string_lossy(),
            &tarball_url,
        ],
        cancel,
    ) {
        let _ = runner.remove_dir_all(&tmp_dir);
        return Err(AgentError::Internal(format!(
            "Gagal mengunduh binary Node.js dari {}: {}",
            tarball_url, e
        )));
    }

    let bytes = runner.read_file(&download_tarball_path).map_err(|e| {
        let _ = runner.remove_dir_all(&tmp_dir);
        AgentError::Internal(format!("Gagal membaca binary Node.js hasil unduhan: {}", e))
    })?;

    let _ = runner.remove_dir_all(&tmp_dir);

    info!("Verifying SHA256 checksum for {}", tarball_filename);
    verify_sha256(&bytes, &expected_sha256).map_err(|e| {
        AgentError::Internal(format!(
            "Verifikasi integritas SHA256 gagal untuk {}: {}",
            tarball_filename, e
        ))
    })?;

    Ok(bytes)
}

fn ensure_node_install_active(cancel: &CancellationToken) -> Result<(), AgentError> {
    if cancel.is_cancelled() {
        return Err(AgentError::Internal("Instalasi Node.js dibatalkan".into()));
    }
    Ok(())
}

/// Re-enables corepack shims (`pnpm`, `yarn`) for a given installed Node.js runtime.
pub fn enable_node_corepack(
    runner: &dyn SystemRunner,
    req: &NodeRuntimeCorepackRequest,
) -> Result<NodeRuntimeActionResponse, AgentError> {
    let base_dir = get_node_base_dir(req.base_dir.as_deref());

    let norm_ver = normalize_node_version(&req.version);
    let target_dir = base_dir.join(&norm_ver);
    let bin_dir = target_dir.join("bin");

    if !runner.path_exists(&bin_dir) {
        return Err(AgentError::InvalidInput(format!(
            "Direktori runtime Node.js v{} tidak ditemukan di {}",
            norm_ver,
            target_dir.display()
        )));
    }

    enable_corepack_shims(&bin_dir)
        .map_err(|e| AgentError::Internal(format!("Gagal mengaktifkan corepack: {}", e)))?;

    Ok(NodeRuntimeActionResponse {
        version: norm_ver,
        success: true,
        path: target_dir.to_string_lossy().to_string(),
        message: "Corepack (pnpm & yarn) berhasil diaktifkan".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use panel_runtime::node::compute_sha256;
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn disconnected_installer_waits_for_cleanup_and_releases_install_lock() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let cleaned = Arc::new(AtomicBool::new(false));
        let worker_cleaned = Arc::clone(&cleaned);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (client, stream) = tokio::io::duplex(1024);
        let handle = tokio::spawn(async move {
            let mut rpc = panel_proto::RpcStream::new(stream);
            run_node_operation_connected(&mut rpc, move |cancel| {
                let _ = started_tx.send(());
                while !cancel.is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                worker_cleaned.store(true, Ordering::SeqCst);
                Err(AgentError::Internal("cancelled fixture".into()))
            })
            .await
        });
        started_rx.await.unwrap();
        drop(client);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), handle)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(cleaned.load(Ordering::SeqCst));
        let (_client, stream) = tokio::io::duplex(1024);
        let mut rpc = panel_proto::RpcStream::new(stream);
        let result = run_node_operation_connected(&mut rpc, |_| {
            Ok(NodeRuntimeActionResponse {
                version: "24.22.0".into(),
                success: true,
                path: "fixture".into(),
                message: "done".into(),
            })
        })
        .await
        .unwrap();
        assert!(result.success);
    }

    #[test]
    fn cancelled_download_does_not_start_a_command() {
        let runner = MockSystemRunner::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            runner
                .run_cancellable_download(&["--version"], &cancel)
                .is_err()
        );
        assert!(runner.actions.lock().unwrap().is_empty());
    }

    fn build_mock_node_tarball(ver: &str) -> Vec<u8> {
        let mut tar_builder = tar::Builder::new(Vec::new());

        let node_script = format!("#!/usr/bin/env node\nconsole.log('{}');\n", ver);
        let mut header_node = tar::Header::new_gnu();
        header_node
            .set_path(format!("node-v{}-linux-x64/bin/node", ver))
            .unwrap();
        header_node.set_size(node_script.len() as u64);
        header_node.set_mode(0o755);
        header_node.set_cksum();
        tar_builder
            .append(&header_node, node_script.as_bytes())
            .unwrap();

        let cp_script = b"#!/usr/bin/env node\n// corepack\n";
        let mut header_cp = tar::Header::new_gnu();
        header_cp
            .set_path(format!("node-v{}-linux-x64/bin/corepack", ver))
            .unwrap();
        header_cp.set_size(cp_script.len() as u64);
        header_cp.set_mode(0o755);
        header_cp.set_cksum();
        tar_builder.append(&header_cp, &cp_script[..]).unwrap();

        let tar_data = tar_builder.into_inner().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_install_offline_tarball_and_list_remove() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join("runtimes").join("node");

        let tarball_bytes = build_mock_node_tarball("22.14.0");
        let checksum = compute_sha256(&tarball_bytes);

        let offline_tar_file = tmp.path().join("node-v22.14.0-linux-x64.tar.gz");
        std::fs::write(&offline_tar_file, &tarball_bytes).unwrap();

        let runner = MockSystemRunner::new();

        // 1. Install Node.js from offline file with checksum verification
        let install_req = NodeRuntimeInstallRequest {
            version: "22.14.0".to_string(),
            mirror_url: None,
            offline_tarball_path: Some(offline_tar_file.to_string_lossy().to_string()),
            offline_checksum: Some(checksum.clone()),
            enable_corepack: true,
            base_dir: Some(base_dir.to_string_lossy().to_string()),
        };

        let res_install = install_node_runtime(&runner, &install_req).expect("install failed");
        assert!(res_install.success);
        assert_eq!(res_install.version, "22.14.0");

        // 2. List runtimes
        let list_res =
            list_node_runtimes(&runner, Some(&base_dir.to_string_lossy())).expect("list failed");
        assert_eq!(list_res.versions.len(), 1);
        assert_eq!(list_res.versions[0].version, "22.14.0");
        assert_eq!(list_res.versions[0].raw_version, "v22.14.0");
        assert!(list_res.versions[0].corepack_enabled);

        // 3. Re-enable corepack
        let cp_req = NodeRuntimeCorepackRequest {
            version: "22.14.0".to_string(),
            base_dir: Some(base_dir.to_string_lossy().to_string()),
        };
        let res_cp = enable_node_corepack(&runner, &cp_req).expect("corepack enable failed");
        assert!(res_cp.success);

        // 4. Remove runtime
        let rem_req = NodeRuntimeRemoveRequest {
            version: "22.14.0".to_string(),
            base_dir: Some(base_dir.to_string_lossy().to_string()),
        };
        let res_rem = remove_node_runtime(&runner, &rem_req).expect("remove failed");
        assert!(res_rem.success);

        // Verify it was removed
        let list_after =
            list_node_runtimes(&runner, Some(&base_dir.to_string_lossy())).expect("list failed");
        assert_eq!(list_after.versions.len(), 0);
    }

    #[test]
    fn test_install_offline_tarball_checksum_mismatch() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join("runtimes").join("node");

        let tarball_bytes = build_mock_node_tarball("20.18.3");
        let offline_tar_file = tmp.path().join("node-v20.18.3-linux-x64.tar.gz");
        std::fs::write(&offline_tar_file, &tarball_bytes).unwrap();

        let runner = MockSystemRunner::new();

        let install_req = NodeRuntimeInstallRequest {
            version: "20.18.3".to_string(),
            mirror_url: None,
            offline_tarball_path: Some(offline_tar_file.to_string_lossy().to_string()),
            offline_checksum: Some(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
            ),
            enable_corepack: true,
            base_dir: Some(base_dir.to_string_lossy().to_string()),
        };

        let res = install_node_runtime(&runner, &install_req);
        assert!(res.is_err());
    }
}
