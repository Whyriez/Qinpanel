//! Global uv prerequisite. Fixed upstream artifacts; never executes an installer script.
use crate::{AgentError, system::SystemRunner};
use flate2::read::GzDecoder;
use panel_runtime::node::verify_sha256;
use std::{io::Read, path::Path, sync::Mutex};
use tokio_util::sync::CancellationToken;

pub(crate) const UV_BIN: &str = "/usr/local/bin/uv";
const UV_VERSION: &str = "0.13.1";
const DOWNLOAD_DIR: &str = "/var/lib/panel/uv-bootstrap";
const MAX_BYTES: u64 = 64 * 1024 * 1024;
static LOCK: Mutex<()> = Mutex::new(());

fn artifact(arch: &str) -> Result<(&'static str, &'static str), AgentError> {
    match arch {
        "x86_64" => Ok((
            "uv-x86_64-unknown-linux-gnu",
            "eadd9d27c134ab5ac1ffd4473b97a741407f3ea238288f03121fc4deddf90220",
        )),
        "aarch64" => Ok((
            "uv-aarch64-unknown-linux-gnu",
            "9041c73c99fdc5892cf128140beb354eca9231d502068cc36e6cb53a8e59ad14",
        )),
        _ => Err(AgentError::InvalidInput(
            "Arsitektur server belum didukung untuk pemasangan uv".into(),
        )),
    }
}

fn verify_binary(runner: &dyn SystemRunner, path: &str) -> Result<(), AgentError> {
    let result = runner.run_command(path, &["--version"]).map_err(|error| {
        AgentError::Internal(format!(
            "Prasyarat uv tidak dapat dijalankan dari {path}: {error}"
        ))
    })?;
    if !result.status.success() {
        return Err(AgentError::Internal(
            "Pemeriksaan prasyarat uv gagal".into(),
        ));
    }
    Ok(())
}

pub(crate) fn ensure(
    runner: &dyn SystemRunner,
    cancel: &CancellationToken,
) -> Result<&'static str, AgentError> {
    let _guard = loop {
        if cancel.is_cancelled() {
            return Err(AgentError::Internal("Persiapan uv dibatalkan".into()));
        }
        match LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(_) => {
                return Err(AgentError::Internal(
                    "Kunci persiapan uv tidak tersedia".into(),
                ));
            }
        }
    };
    if runner.path_exists(Path::new(UV_BIN)) {
        verify_binary(runner, UV_BIN)?;
        return Ok(UV_BIN);
    }
    // An OS-managed installation is usable by tenants; never depend on root's home/PATH.
    if runner.path_exists(Path::new("/usr/bin/uv")) {
        verify_binary(runner, "/usr/bin/uv")?;
        runner.create_symlink(Path::new("/usr/bin/uv"), Path::new(UV_BIN))?;
        return Ok(UV_BIN);
    }
    let (name, checksum) = artifact(std::env::consts::ARCH)?;
    bootstrap(runner, name, checksum, cancel)?;
    Ok(UV_BIN)
}

fn bootstrap(
    runner: &dyn SystemRunner,
    name: &str,
    checksum: &str,
    cancel: &CancellationToken,
) -> Result<(), AgentError> {
    let archive_path = Path::new(DOWNLOAD_DIR).join("uv.tar.gz");
    runner.create_dir_all(Path::new(DOWNLOAD_DIR), 0o700, 0, 0)?;
    let url =
        format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{name}.tar.gz");
    let result = (|| {
        let output = runner
            .run_cancellable_download(
                &[
                    "--disable",
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--connect-timeout",
                    "10",
                    "--max-time",
                    "180",
                    "--max-filesize",
                    "67108864",
                    "--output",
                    &archive_path.to_string_lossy(),
                    &url,
                ],
                cancel,
            )
            .map_err(|error| {
                AgentError::Internal(format!("Gagal mengunduh prasyarat uv: {error}"))
            })?;
        if !output.status.success() {
            return Err(AgentError::Internal("Unduhan prasyarat uv gagal".into()));
        }
        let archive = runner.read_file(&archive_path)?;
        if archive.len() as u64 > MAX_BYTES {
            return Err(AgentError::InvalidInput(
                "Arsip uv melebihi batas ukuran".into(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(AgentError::Internal("Persiapan uv dibatalkan".into()));
        }
        install_verified(runner, &archive, name, checksum)?;
        verify_binary(runner, UV_BIN)
    })();
    let _ = runner.remove_file(&archive_path);
    result
}

fn install_verified(
    runner: &dyn SystemRunner,
    bytes: &[u8],
    name: &str,
    checksum: &str,
) -> Result<(), AgentError> {
    verify_sha256(bytes, checksum)
        .map_err(|_| AgentError::InvalidInput("Checksum SHA256 prasyarat uv tidak cocok".into()))?;
    let expected = format!("{name}/uv");
    let decoder = GzDecoder::new(bytes).take(MAX_BYTES * 2);
    let mut archive = tar::Archive::new(decoder);
    let mut binary = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() != Path::new(&expected) {
            continue;
        }
        if !entry.header().entry_type().is_file() || binary.is_some() {
            return Err(AgentError::InvalidInput(
                "Entri executable uv tidak valid".into(),
            ));
        }
        let mut content = Vec::new();
        entry
            .by_ref()
            .take(MAX_BYTES + 1)
            .read_to_end(&mut content)?;
        if content.is_empty() || content.len() as u64 > MAX_BYTES {
            return Err(AgentError::InvalidInput(
                "Ukuran executable uv tidak valid".into(),
            ));
        }
        binary = Some(content);
    }
    let binary = binary.ok_or_else(|| {
        AgentError::InvalidInput("Executable uv tidak ditemukan di arsip resmi".into())
    })?;
    runner.create_dir_all(Path::new("/usr/local/bin"), 0o755, 0, 0)?;
    // Publish only the single verified file, atomically, without unpacking archive paths.
    let staged = Path::new("/usr/local/bin/.panel-uv-new");
    runner.write_file(staged, &binary, 0o755)?;
    runner.set_ownership(staged, 0, 0)?;
    let result = runner.move_path(staged, Path::new(UV_BIN));
    if result.is_err() {
        let _ = runner.remove_file(staged);
    }
    result
}

#[cfg(test)]
#[path = "uv_tests.rs"]
mod tests;
