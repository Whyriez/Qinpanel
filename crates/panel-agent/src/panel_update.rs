//! Administrator self-update, detached from API/agent lifecycle. Fixed paths only.
use crate::{AgentError, system::SystemRunner};
use panel_core::update::{release_metadata_url, release_version, valid_digest};
use panel_proto::{
    PanelRelease, PanelUpdateRequest, PanelUpdateRun, PanelUpdateStatus, UpdatePhase,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, sync::Mutex};

const ROOT: &str = "/var/lib/panel-updater";
const CATALOG: &str = "/var/lib/panel-updater/catalog.json";
const RUN: &str = "/var/lib/panel-updater/state.json";
const REQUEST: &str = "/var/lib/panel-updater/request.env";
const INSTALLED: &str = "/etc/panel/installed-release";
const WORKER: &str = "/usr/local/libexec/panel-update-worker";
const UNIT: &str = "panel-update.service";
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
struct Catalog {
    checked_at: Option<i64>,
    available: Option<PanelRelease>,
    error: Option<String>,
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn invalid(message: &str) -> AgentError {
    AgentError::InvalidInput(message.into())
}
fn save(runner: &dyn SystemRunner, path: &str, value: &impl Serialize) -> Result<(), AgentError> {
    let temporary = format!("{path}.staging");
    let bytes = serde_json::to_vec(value).map_err(|e| AgentError::Internal(e.to_string()))?;
    runner.write_file(Path::new(&temporary), &bytes, 0o600)?;
    runner.move_path(Path::new(&temporary), Path::new(path))
}
fn read<T: serde::de::DeserializeOwned>(
    runner: &dyn SystemRunner,
    path: &str,
) -> Result<Option<T>, AgentError> {
    if !runner.path_exists(Path::new(path)) {
        return Ok(None);
    }
    let bytes = runner.read_file(Path::new(path))?;
    if bytes.len() > 32_768 {
        return Err(invalid(
            "Status update terlalu besar; periksa layanan updater.",
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| invalid("Status update rusak; periksa layanan updater."))
}
fn status(runner: &dyn SystemRunner) -> Result<PanelUpdateStatus, AgentError> {
    let catalog = read::<Catalog>(runner, CATALOG)?.unwrap_or_default();
    let mut update = read::<PanelUpdateRun>(runner, RUN)?;
    if let Some(run) = update
        .as_mut()
        .filter(|r| busy(&r.phase) && now() - r.started_at > 60)
    {
        let active =
            runner.run_command("systemctl", &["show", UNIT, "--property=ActiveState,Job"])?;
        let properties = String::from_utf8_lossy(&active.stdout);
        let inactive = properties
            .lines()
            .any(|s| matches!(s, "ActiveState=inactive" | "ActiveState=failed"));
        let no_job = properties.lines().any(|s| matches!(s, "Job=" | "Job=0"));
        if inactive && no_job {
            run.phase = UpdatePhase::Interrupted;
            run.finished_at = Some(now());
            save(runner, RUN, run)?;
        }
    }
    let installed_digest = runner
        .read_file(Path::new(INSTALLED))
        .ok()
        .and_then(|bytes| {
            String::from_utf8(bytes).ok().and_then(|s| {
                s.split_whitespace()
                    .nth(1)
                    .filter(|d| valid_digest(d))
                    .map(str::to_owned)
            })
        });
    Ok(PanelUpdateStatus {
        diagnostics: None,
        current_version: format!("v{}", env!("CARGO_PKG_VERSION")),
        installed_digest,
        supported: runner.path_exists(Path::new(WORKER))
            && runner.path_exists(Path::new("/etc/systemd/system/panel-update.service")),
        checked_at: catalog.checked_at,
        available: catalog.available,
        check_error: catalog.error,
        update,
    })
}
fn busy(phase: &UpdatePhase) -> bool {
    matches!(phase, UpdatePhase::Queued | UpdatePhase::Running)
}

pub fn release_from_metadata(value: &Value, arch: &str) -> Result<PanelRelease, AgentError> {
    if value["draft"].as_bool() != Some(false) || value["prerelease"].as_bool() != Some(false) {
        return Err(invalid("Rilis belum stabil atau belum dipublikasikan."));
    }
    let version = value["tag_name"]
        .as_str()
        .ok_or_else(|| invalid("Tag release tidak tersedia."))?;
    release_version(version).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(arch, "x86_64" | "aarch64") {
        return Err(invalid("Arsitektur updater tidak didukung."));
    }
    let filename = format!("qinpanel-{version}-{arch}-unknown-linux-gnu.tar.gz");
    let assets = value["assets"]
        .as_array()
        .ok_or_else(|| invalid("Paket release belum lengkap."))?;
    let previous_filename = format!("qonpanel-{version}-{arch}-unknown-linux-gnu.tar.gz");
    let legacy_filename = format!("nusa-panel-{version}-{arch}-unknown-linux-gnu.tar.gz");
    let asset = [&filename, &previous_filename, &legacy_filename]
        .into_iter()
        .find_map(|name| {
            assets.iter().find(|a| {
                a["name"].as_str() == Some(name) && a["state"].as_str() == Some("uploaded")
            })
        })
        .ok_or_else(|| invalid("Paket arsitektur server belum tersedia."))?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| valid_digest(s))
        .ok_or_else(|| invalid("Digest SHA-256 release belum tersedia."))?;
    if !assets
        .iter()
        .any(|a| a["name"] == "SHA256SUMS" && a["state"] == "uploaded")
    {
        return Err(invalid("Checksum release belum tersedia."));
    }
    if !asset["size"]
        .as_u64()
        .is_some_and(|n| n > 0 && n <= 536_870_912)
    {
        return Err(invalid("Ukuran paket release tidak valid."));
    }
    Ok(PanelRelease {
        version: version.into(),
        digest: digest.into(),
        notes: value["body"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(4000)
            .collect(),
    })
}

pub fn execute(
    runner: &dyn SystemRunner,
    request: PanelUpdateRequest,
) -> Result<PanelUpdateStatus, AgentError> {
    let _guard = LOCK
        .lock()
        .map_err(|_| AgentError::Internal("Kunci updater tidak tersedia.".into()))?;
    let current = status(runner)?;
    match request {
        PanelUpdateRequest::Status => Ok(current),
        PanelUpdateRequest::Logs => {
            let output = runner.run_command(
                "journalctl",
                &[
                    "--unit=panel-update.service",
                    "--no-pager",
                    "--output=cat",
                    "--lines=100",
                ],
            )?;
            let mut current = current;
            current.diagnostics = Some(
                String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .take(12_000)
                    .collect(),
            );
            Ok(current)
        }
        PanelUpdateRequest::Check => {
            if current
                .checked_at
                .is_some_and(|t| (0..30).contains(&(now() - t)))
            {
                return Ok(current);
            }
            let metadata_url = release_metadata_url(option_env!("QONPANEL_RELEASE_BASE_URL"))
                .map_err(|_| {
                    invalid("Sumber distribusi paket tidak valid; perbarui paket lengkap.")
                })?;
            let result = runner
                .run_command(
                    "curl",
                    &[
                        "--fail",
                        "--silent",
                        "--show-error",
                        "--proto",
                        "=https",
                        "--connect-timeout",
                        "5",
                        "--max-time",
                        "15",
                        "--max-filesize",
                        "1048576",
                        "-H",
                        "Accept: application/vnd.github+json",
                        "-H",
                        "User-Agent: Qinpanel-Updater",
                        &metadata_url,
                    ],
                )
                .and_then(|o| {
                    if !o.status.success() || o.stdout.len() > 1_048_576 {
                        return Err(invalid("Metadata release tidak tersedia."));
                    }
                    let value = serde_json::from_slice(&o.stdout)
                        .map_err(|_| invalid("Metadata release tidak valid."))?;
                    release_from_metadata(&value, std::env::consts::ARCH)
                });
            let catalog = match result {
                Ok(release) => Catalog {
                    checked_at: Some(now()),
                    available: Some(release),
                    error: None,
                },
                Err(_) => Catalog {
                    checked_at: Some(now()),
                    available: None,
                    error: Some(
                        "Gagal memeriksa rilis stabil. Periksa koneksi ke server distribusi."
                            .into(),
                    ),
                },
            };
            runner.create_dir_all(Path::new(ROOT), 0o700, 0, 0)?;
            save(runner, CATALOG, &catalog)?;
            status(runner)
        }
        PanelUpdateRequest::Apply {
            id,
            version,
            digest,
        } => apply(runner, current, id, version, digest),
    }
}
fn apply(
    runner: &dyn SystemRunner,
    current: PanelUpdateStatus,
    id: String,
    version: String,
    digest: String,
) -> Result<PanelUpdateStatus, AgentError> {
    if uuid::Uuid::parse_str(&id).is_err()
        || id.len() != 36
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
        || !valid_digest(&digest)
    {
        return Err(invalid("Identitas atau digest update tidak valid."));
    }
    let target = release_version(&version).map_err(|e| invalid(&e.to_string()))?;
    if let Some(run) = &current.update {
        if run.id == id {
            if run.version == version && run.digest == digest {
                return Ok(current);
            }
            return Err(invalid(
                "Identitas update sudah digunakan untuk paket lain.",
            ));
        }
        if busy(&run.phase) {
            return Err(invalid(
                "Update lain masih berjalan. Tunggu status akhirnya.",
            ));
        }
    }
    if !current.supported {
        return Err(invalid(
            "Pasang paket lengkap yang memuat updater sebelum memakai tombol ini.",
        ));
    }
    if runner
        .run_command("systemctl", &["is-active", "--quiet", UNIT])
        .is_ok()
    {
        return Err(invalid(
            "Updater sebelumnya masih menyelesaikan proses. Muat ulang status sebentar lagi.",
        ));
    }
    if target < release_version(&current.current_version).map_err(|e| invalid(&e.to_string()))? {
        return Err(invalid(
            "Downgrade ditolak untuk menjaga kompatibilitas database.",
        ));
    }
    if version == current.current_version && version != "v0.1.0" {
        return Err(invalid(
            "Rilis stabil harus memakai nomor versi baru. Paket versi terpasang tidak diganti melalui panel.",
        ));
    }
    if current.installed_digest.as_deref() == Some(&digest) {
        return Err(invalid("Paket release ini sudah terpasang."));
    }
    if !current
        .checked_at
        .is_some_and(|t| (0..=600).contains(&(now() - t)))
        || !current
            .available
            .as_ref()
            .is_some_and(|r| r.version == version && r.digest == digest)
    {
        return Err(invalid(
            "Periksa ulang rilis; pilihan update sudah kedaluwarsa atau berubah.",
        ));
    }
    let mut run = PanelUpdateRun {
        id,
        version,
        digest,
        phase: UpdatePhase::Queued,
        started_at: now(),
        finished_at: None,
    };
    runner.create_dir_all(Path::new(ROOT), 0o700, 0, 0)?;
    let environment = format!(
        "UPDATE_ID={}\nUPDATE_VERSION={}\nUPDATE_DIGEST={}\n",
        run.id, run.version, run.digest
    );
    runner.write_file(Path::new(REQUEST), environment.as_bytes(), 0o600)?;
    save(runner, RUN, &run)?;
    if runner
        .run_command("systemctl", &["start", "--no-block", UNIT])
        .is_err()
    {
        run.phase = UpdatePhase::Failed;
        run.finished_at = Some(now());
        save(runner, RUN, &run)?;
        return Err(invalid(
            "Updater gagal dijadwalkan. Status tersimpan; periksa layanan update.",
        ));
    }
    status(runner)
}
