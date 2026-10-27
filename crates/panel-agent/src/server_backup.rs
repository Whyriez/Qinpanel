//! Typed root handoff. Archives and host paths are never provided by the API.
use crate::{AgentError, system::SystemRunner};
use panel_proto::{ServerBackupJob, ServerBackupOperation, ServerBackupRequest};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

const ROOT: &str = "/var/lib/qonpanel-migration";
const UNIT: &str = "qonpanel-server-backup.service";
static LOCK: Mutex<()> = Mutex::new(());
fn invalid(message: &str) -> AgentError {
    AgentError::InvalidInput(message.into())
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}
fn read(r: &dyn SystemRunner, name: &str, fallback: Value) -> Result<Value, AgentError> {
    let path = format!("{ROOT}/{name}.json");
    if !r.path_exists(Path::new(&path)) {
        return Ok(fallback);
    }
    let bytes = r.read_file(Path::new(&path))?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("Status backup terlalu besar."));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Metadata backup tidak valid."))
}
fn save(r: &dyn SystemRunner, name: &str, value: &Value) -> Result<(), AgentError> {
    let path = format!("{ROOT}/{name}.json");
    let temp = format!("{path}.agent");
    r.write_file(
        Path::new(&temp),
        &serde_json::to_vec(value).map_err(|_| invalid("Status backup tidak dapat disimpan."))?,
        0o600,
    )?;
    r.move_path(Path::new(&temp), Path::new(&path))
}
fn status(r: &dyn SystemRunner) -> Result<Value, AgentError> {
    let mut state = read(r, "state", Value::Null)?;
    if matches!(state["phase"].as_str(), Some("queued" | "running"))
        && chrono::Utc::now()
            .timestamp()
            .saturating_sub(state["started_at"].as_i64().unwrap_or(0))
            >= 5
        && let Ok(output) = r.run_command(
            "systemctl",
            &["show", UNIT, "--property=ActiveState,Job", "--no-pager"],
        )
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text
            .lines()
            .any(|line| matches!(line, "ActiveState=inactive" | "ActiveState=failed"))
            && text.lines().any(|line| matches!(line, "Job=" | "Job=0"))
        {
            state["phase"] = json!("interrupted");
            state["message"] = json!(
                "Pekerjaan terputus. Periksa layanan; bila ada transaksi keselamatan, jalankan helper --recover melalui SSH."
            );
            save(r, "state", &state)?;
            if !r.path_exists(Path::new("/var/lib/qonpanel-migration/journal.json"))
                && r.path_exists(Path::new("/var/lib/qonpanel-migration/maintenance"))
            {
                r.remove_file(Path::new("/var/lib/qonpanel-migration/maintenance"))?;
            }
            if read(r, "request", Value::Null)?["id"] == state["id"] {
                r.remove_file(Path::new("/var/lib/qonpanel-migration/request.json"))?;
            }
        }
    }
    let supported = r.operating_system().is_ok_and(|os| {
        !os.is_rpm()
            || [
                "/usr/local/libexec/qonpanel-selinux",
                "/usr/local/libexec/qonpanel-mail-auth",
            ]
            .iter()
            .all(|path| r.path_exists(Path::new(path)))
    }) && r.path_exists(Path::new("/usr/local/libexec/qonpanel-server-backup"))
        && r.path_exists(Path::new(
            "/etc/systemd/system/qonpanel-server-backup.service",
        ));
    Ok(json!({"supported": supported,
        "job":state,"backups":read(r,"catalog",json!([]))?,"inspection":read(r,"inspection",Value::Null)?,"approval":read(r,"approval",Value::Null)?}))
}
pub fn validate(job: &ServerBackupJob) -> Result<(), AgentError> {
    if !valid_id(&job.id)
        || !(16..=256).contains(&job.passphrase.len())
        || job
            .passphrase
            .chars()
            .any(|c| matches!(c, '\r' | '\n' | '\0'))
    {
        return Err(invalid(
            "Gunakan identitas valid dan passphrase 16–256 byte tanpa baris baru.",
        ));
    }
    if !matches!(job.operation, ServerBackupOperation::Create)
        && !job.archive_id.as_deref().is_some_and(valid_id)
    {
        return Err(invalid("Pilih unggahan backup yang valid."));
    }
    if matches!(job.operation, ServerBackupOperation::Restore)
        && !job
            .digest
            .as_deref()
            .is_some_and(panel_core::update::valid_digest)
    {
        return Err(invalid("Periksa arsip sebelum restore."));
    }
    if let Some(address) = job.new_ipv4.as_deref().filter(|s| !s.is_empty()) {
        let address = address
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| invalid("IPv4 baru tidak valid."))?;
        if address.is_unspecified() || address.is_multicast() {
            return Err(invalid("IPv4 baru tidak valid."));
        }
    }
    Ok(())
}
pub fn execute(r: &dyn SystemRunner, request: ServerBackupRequest) -> Result<Value, AgentError> {
    let _guard = LOCK
        .lock()
        .map_err(|_| invalid("Kunci backup tidak tersedia."))?;
    let current = status(r)?;
    match request {
        ServerBackupRequest::Status => Ok(current),
        ServerBackupRequest::Start(job) => {
            validate(&job)?;
            if current["job"]["id"] == job.id {
                if current["job"]["operation"]
                    != serde_json::to_value(job.operation)
                        .map_err(|_| invalid("Operasi tidak valid."))?
                {
                    return Err(invalid(
                        "Identitas pekerjaan sudah digunakan untuk operasi lain.",
                    ));
                }
                for (key, expected) in [
                    ("archive_id", serde_json::to_value(&job.archive_id)),
                    ("digest", serde_json::to_value(&job.digest)),
                    ("new_ipv4", serde_json::to_value(&job.new_ipv4)),
                ] {
                    if current["job"][key]
                        != expected.map_err(|_| invalid("Permintaan tidak valid."))?
                    {
                        return Err(invalid(
                            "Identitas pekerjaan sudah digunakan untuk isian lain.",
                        ));
                    }
                }
                return Ok(current);
            }
            if matches!(job.operation, ServerBackupOperation::Restore)
                && (current["approval"]["digest"].as_str() != job.digest.as_deref()
                    || current["approval"]["expires_at"].as_i64().unwrap_or(0)
                        < chrono::Utc::now().timestamp())
            {
                return Err(invalid(
                    "Pemilik server perlu mengesahkan SHA-256 melalui SSH/installer sebelum restore konfigurasi root.",
                ));
            }
            if current["supported"] != true {
                return Err(invalid(
                    "Upgrade paket lengkap Qinpanel untuk mengaktifkan backup migrasi.",
                ));
            }
            if matches!(current["job"]["phase"].as_str(), Some("running" | "queued"))
                || r.run_command("systemctl", &["is-active", "--quiet", UNIT])
                    .is_ok()
            {
                return Err(invalid(
                    "Pekerjaan sebelumnya belum selesai. Tunggu status akhir atau lakukan pemulihan melalui SSH.",
                ));
            }
            if r.path_exists(Path::new("/var/lib/qonpanel-migration/journal.json")) {
                return Err(invalid(
                    "Transaksi lama terputus. Jalankan helper --recover melalui SSH sebelum mencoba lagi.",
                ));
            }
            // Serialize with the existing updater through the same deployment lock in the worker.
            if r.run_command(
                "systemctl",
                &["is-active", "--quiet", "panel-update.service"],
            )
            .is_ok()
            {
                return Err(invalid(
                    "Tunggu pembaruan versi selesai sebelum backup/restore.",
                ));
            }
            r.create_dir_all(Path::new(ROOT), 0o750, 0, 0)?;
            if matches!(job.operation, ServerBackupOperation::Create) {
                crate::crypto::get_or_create_master_key(r, None)?;
            }
            save(
                r,
                "request",
                &serde_json::to_value(&job).map_err(|_| invalid("Permintaan tidak valid."))?,
            )?;
            save(
                r,
                "state",
                &json!({"id":job.id,"operation":job.operation,"phase":"queued","progress":0,"started_at":chrono::Utc::now().timestamp(),"archive_id":job.archive_id,"digest":job.digest,"new_ipv4":job.new_ipv4,"message":"Pekerjaan masuk antrean."}),
            )?;
            if r.run_command("systemctl", &["start", "--no-block", UNIT])
                .is_err()
            {
                let _ = r.remove_file(Path::new("/var/lib/qonpanel-migration/request.json"));
                save(
                    r,
                    "state",
                    &json!({"id":job.id,"operation":job.operation,"phase":"failed","message":"Layanan backup tidak dapat dimulai."}),
                )?;
                return Err(invalid("Layanan backup tidak dapat dimulai."));
            }
            status(r)
        }
        ServerBackupRequest::Delete { id } => {
            if !valid_id(&id) {
                return Err(invalid("Identitas backup tidak valid."));
            }
            if matches!(current["job"]["phase"].as_str(), Some("queued" | "running")) {
                return Err(invalid(
                    "Tunggu pekerjaan selesai sebelum menghapus backup.",
                ));
            }
            let entries = current["backups"]
                .as_array()
                .ok_or_else(|| invalid("Katalog tidak valid."))?;
            let retained: Vec<_> = entries
                .iter()
                .filter(|entry| entry["id"] != id)
                .cloned()
                .collect();
            if retained.len() == entries.len() {
                return Err(invalid("Backup tidak ditemukan."));
            }
            r.remove_file(Path::new(&format!("{ROOT}/exports/{id}.qpb")))?;
            save(r, "catalog", &json!(retained))?;
            status(r)
        }
        ServerBackupRequest::DeleteUpload { id } => {
            if !valid_id(&id) {
                return Err(invalid("Identitas transfer tidak valid."));
            }
            if matches!(current["job"]["phase"].as_str(), Some("queued" | "running")) {
                return Err(invalid(
                    "Tunggu pekerjaan selesai sebelum menghapus transfer.",
                ));
            }
            let path = format!("{ROOT}/imports/{id}.qpb");
            if r.path_exists(Path::new(&path)) {
                r.remove_file(Path::new(&path))?;
            }
            if current["inspection"]["archive_id"] == id {
                save(r, "inspection", &Value::Null)?;
            }
            status(r)
        }
    }
}
