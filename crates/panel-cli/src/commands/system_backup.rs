//! `panelctl backup` command handlers for panel database and system configuration.

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use panel_core::system_backup::{
    SystemBackupFileEntry, SystemBackupManifest, SystemBackupSummary,
    SystemBackupVerificationResult, compute_bytes_sha256, generate_system_backup_filename,
    validate_system_backup_filename,
};
use panel_db::pool::DbPool;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tar::{Archive, Builder};
use tracing::info;

/// Subcommands for `panelctl backup`.
#[derive(clap::Subcommand, Debug)]
pub enum SystemBackupCommands {
    /// Create a new snapshot backup of panel database and configuration
    Create {
        /// Optional destination output filename (e.g. panel-backup-custom.tar.gz)
        #[arg(short, long)]
        output: Option<String>,

        /// Include analytics.db SQLite database in the backup archive
        #[arg(long, default_value_t = false)]
        include_analytics: bool,
    },
    /// List all existing panel system backups
    List {
        /// Backups directory path (default: /var/lib/panel/backups or ./backups)
        #[arg(short, long)]
        dir: Option<String>,
    },
    /// Verify integrity and SHA-256 checksums of a backup snapshot archive
    Verify {
        /// Path to the backup archive file (.tar.gz)
        #[arg(short, long)]
        path: String,
    },
    /// Restore panel database and system configurations from a backup snapshot
    Restore {
        /// Path to the backup archive file (.tar.gz)
        #[arg(short, long)]
        path: String,

        /// Bypass safety confirmation
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
}

pub async fn handle_system_backup_command(
    command: SystemBackupCommands,
    pool: &DbPool,
    db_path: &str,
) -> Result<(), Box<dyn Error>> {
    match command {
        SystemBackupCommands::Create {
            output,
            include_analytics,
        } => {
            create_system_backup(pool, db_path, output, include_analytics).await?;
        }
        SystemBackupCommands::List { dir } => {
            list_system_backups(dir)?;
        }
        SystemBackupCommands::Verify { path } => {
            let res = verify_system_backup(&path)?;
            if res.is_valid {
                println!(
                    "\x1b[32m[VALID]\x1b[0m Berkas cadangan '{}' terverifikasi utuh 100%.",
                    path
                );
                if let Some(m) = res.manifest {
                    println!("  Dibuat pada        : {}", m.created_at);
                    println!("  Versi Panel        : {}", m.panel_version);
                    println!("  Jumlah berkas      : {}", m.files.len());
                    println!(
                        "  Ukuran tidak mampat: {} bytes",
                        m.total_uncompressed_bytes
                    );
                }
            } else {
                println!(
                    "\x1b[31m[KORUP]\x1b[0m Berkas cadangan '{}' TIDAK VALID:",
                    path
                );
                for err in res.errors {
                    println!("  - {}", err);
                }
            }
        }
        SystemBackupCommands::Restore { path, force: _ } => {
            restore_system_backup(pool, db_path, &path).await?;
        }
    }
    Ok(())
}

fn determine_backups_dir(override_dir: Option<String>) -> PathBuf {
    if let Some(d) = override_dir {
        PathBuf::from(d)
    } else if Path::new("/var/lib/panel/backups").exists() {
        PathBuf::from("/var/lib/panel/backups")
    } else {
        PathBuf::from("backups")
    }
}

pub async fn create_system_backup(
    pool: &DbPool,
    db_path: &str,
    output: Option<String>,
    include_analytics: bool,
) -> Result<SystemBackupSummary, Box<dyn Error>> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let backups_dir = determine_backups_dir(None);
    std::fs::create_dir_all(&backups_dir)?;

    let filename = output.unwrap_or_else(|| generate_system_backup_filename(timestamp));
    if !validate_system_backup_filename(&filename) {
        return Err(format!("Nama berkas cadangan tidak valid: {}", filename).into());
    }

    let final_archive_path = backups_dir.join(&filename);
    let staging_dir = backups_dir.join(format!("staging_{}_{}", timestamp, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir)?;

    let mut manifest_files = Vec::new();

    // Snapshot each lock domain through SQLite, including committed WAL frames.
    let core_path = panel_db::operations::database_path(pool)
        .await?
        .unwrap_or_else(|| PathBuf::from(db_path));
    let core_snapshot = staging_dir.join("panel.db");
    panel_db::snapshot::from_pool(pool, &core_snapshot).await?;
    let content = std::fs::read(&core_snapshot)?;
    manifest_files.push(SystemBackupFileEntry {
        relative_path: "database/panel.db".to_string(),
        file_type: "sqlite_db".to_string(),
        size_bytes: content.len() as u64,
        sha256: compute_bytes_sha256(&content),
    });
    let operations_src = panel_db::operations::sidecar_path(&core_path);
    let analytics_src = std::env::var_os("PANEL_ANALYTICS_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| core_path.with_file_name("analytics.db"));
    for (source, name, include) in [
        (operations_src, "operations.db", true),
        (analytics_src, "analytics.db", include_analytics),
    ] {
        if include && source.exists() {
            let dest = staging_dir.join(name);
            panel_db::snapshot::from_file(&source, &dest).await?;
            let content = std::fs::read(&dest)?;
            manifest_files.push(SystemBackupFileEntry {
                relative_path: format!("database/{name}"),
                file_type: "sqlite_db".to_string(),
                size_bytes: content.len() as u64,
                sha256: compute_bytes_sha256(&content),
            });
        }
    }

    // 3. Copy panel.toml
    let toml_src = Path::new("panel.toml");
    if toml_src.exists() {
        let content = std::fs::read(toml_src)?;
        let size = content.len() as u64;
        let hash = compute_bytes_sha256(&content);
        let dest = staging_dir.join("panel.toml");
        std::fs::write(&dest, &content)?;
        manifest_files.push(SystemBackupFileEntry {
            relative_path: "config/panel.toml".to_string(),
            file_type: "config_toml".to_string(),
            size_bytes: size,
            sha256: hash,
        });
    }

    // 4. Create manifest.json
    let manifest = SystemBackupManifest::new(timestamp, env!("CARGO_PKG_VERSION"), manifest_files);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(staging_dir.join("manifest.json"), &manifest_bytes)?;

    // 5. Pack tar.gz archive
    let tar_gz_file = File::create(&final_archive_path)?;
    let enc = GzEncoder::new(tar_gz_file, Compression::default());
    let mut builder = Builder::new(enc);
    builder.append_dir_all(".", &staging_dir)?;
    let enc_res = builder.into_inner()?;
    enc_res.finish()?;

    // Cleanup staging
    let _ = std::fs::remove_dir_all(&staging_dir);

    let archive_bytes = std::fs::read(&final_archive_path)?;
    let checksum = compute_bytes_sha256(&archive_bytes);
    let size_bytes = archive_bytes.len() as u64;

    info!(
        "Cadangan panel berhasil dibuat di {} (ukuran: {} bytes, SHA-256: {})",
        final_archive_path.display(),
        size_bytes,
        checksum
    );

    println!(
        "\x1b[32m[SUKSES]\x1b[0m Cadangan sistem berhasil dibuat di: {}",
        final_archive_path.display()
    );
    println!("  Ukuran berkas : {} bytes", size_bytes);
    println!("  Checksum      : {}", checksum);
    println!("  Jumlah berkas : {}", manifest.files.len());

    Ok(SystemBackupSummary {
        filename,
        filepath: final_archive_path.to_string_lossy().to_string(),
        created_at: timestamp,
        size_bytes,
        checksum_sha256: checksum,
        files_count: manifest.files.len(),
        verified: true,
    })
}

pub fn list_system_backups(
    dir_override: Option<String>,
) -> Result<Vec<SystemBackupSummary>, Box<dyn Error>> {
    let dir = determine_backups_dir(dir_override);
    let mut list = Vec::new();

    if dir.exists() {
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !validate_system_backup_filename(&fname) {
                continue;
            }
            if let Ok(meta) = path.metadata() {
                let size = meta.len();
                let created_at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let checksum = if let Ok(bytes) = std::fs::read(&path) {
                    compute_bytes_sha256(&bytes)
                } else {
                    String::new()
                };

                list.push(SystemBackupSummary {
                    filename: fname,
                    filepath: path.to_string_lossy().to_string(),
                    created_at,
                    size_bytes: size,
                    checksum_sha256: checksum,
                    files_count: 0,
                    verified: true,
                });
            }
        }
    }

    list.sort_by_key(|a| std::cmp::Reverse(a.created_at));

    println!("================================================================================");
    println!("                     DAFTAR CADANGAN SISTEM PANEL                               ");
    println!("================================================================================");
    if list.is_empty() {
        println!(
            "Tidak ada berkas cadangan ditemukan di direktori {}.",
            dir.display()
        );
    } else {
        println!(
            "{:<35} {:<15} {:<20}",
            "NAMA BERKAS", "UKURAN", "DIBUAT PADA"
        );
        println!(
            "--------------------------------------------------------------------------------"
        );
        for item in &list {
            println!(
                "{:<35} {:<15} {:<20}",
                item.filename,
                format!("{} bytes", item.size_bytes),
                item.created_at
            );
        }
    }
    println!("================================================================================");

    Ok(list)
}

pub fn verify_system_backup(
    path_str: &str,
) -> Result<SystemBackupVerificationResult, Box<dyn Error>> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Ok(SystemBackupVerificationResult {
            is_valid: false,
            manifest: None,
            errors: vec![format!("Berkas tidak ditemukan: {}", path_str)],
        });
    }

    let file = File::open(path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);

    let mut manifest: Option<SystemBackupManifest> = None;
    let mut extracted_files = std::collections::HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_string_lossy().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content)?;

        if entry_path.ends_with("manifest.json") {
            if let Ok(m) = serde_json::from_slice::<SystemBackupManifest>(&content) {
                manifest = Some(m);
            }
        } else {
            let hash = compute_bytes_sha256(&content);
            extracted_files.insert(entry_path, (content.len() as u64, hash));
        }
    }

    let Some(manifest) = manifest else {
        return Ok(SystemBackupVerificationResult {
            is_valid: false,
            manifest: None,
            errors: vec!["Berkas manifest.json tidak ditemukan di dalam arsip".to_string()],
        });
    };

    let mut errors = Vec::new();
    if let Err(e) = manifest.validate() {
        errors.push(e);
    }

    for expected in &manifest.files {
        let archive_name = Path::new(&expected.relative_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let actual = extracted_files.iter().find(|(name, _)| {
            Path::new(name.as_str())
                .file_name()
                .and_then(|name| name.to_str())
                == Some(archive_name)
        });
        if !matches!(actual, Some((_, (size, hash))) if *size == expected.size_bytes && *hash == expected.sha256)
        {
            errors.push(format!(
                "Checksum atau ukuran tidak cocok: {}",
                expected.relative_path
            ));
        }
    }

    Ok(SystemBackupVerificationResult {
        is_valid: errors.is_empty(),
        manifest: Some(manifest),
        errors,
    })
}

fn unpack_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), Box<dyn Error>> {
    let file = File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);

    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.unpack_in(dest_dir)? {
            return Err("Path arsip berada di luar direktori pemulihan".into());
        }
    }
    Ok(())
}

fn restore_database_file(
    source: &Path,
    target: &Path,
    remove_if_missing: bool,
) -> Result<(), Box<dyn Error>> {
    if !source.exists() && !remove_if_missing {
        return Ok(());
    }
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = target.as_os_str().to_os_string();
        sidecar.push(suffix);
        match std::fs::remove_file(PathBuf::from(sidecar)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if source.exists() {
        std::fs::copy(source, target)?;
    } else if target.exists() {
        // A pre-split backup is imported again from its restored core DB.
        std::fs::remove_file(target)?;
    }
    Ok(())
}

pub async fn restore_system_backup(
    pool: &DbPool,
    db_path: &str,
    path_str: &str,
) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(format!("Berkas cadangan tidak ditemukan: {}", path_str).into());
    }

    let core_path = panel_db::operations::database_path(pool)
        .await?
        .ok_or("Pemulihan memerlukan database berbasis file")?;
    #[cfg(unix)]
    if core_path == Path::new(panel_core::production::DEFAULT_DATABASE_PATH) {
        let status = panel_core::production::system_command("systemctl")
            .args(["is-active", "panel-api.service"])
            .output()?;
        let state = String::from_utf8_lossy(&status.stdout);
        if !matches!(state.trim(), "inactive" | "failed") {
            return Err("Hentikan panel-api sebelum pemulihan database offline".into());
        }
    }
    let verification = verify_system_backup(path_str)?;
    if !verification.is_valid {
        return Err(format!("Cadangan tidak valid: {:?}", verification.errors).into());
    }

    // Safety pre-restore snapshot
    info!("Membuat cadangan keselamatan pra-pemulihan...");
    let pre_backup = create_system_backup(pool, db_path, None, false).await?;
    info!("Cadangan keselamatan dibuat di: {}", pre_backup.filepath);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let restore_temp = determine_backups_dir(None).join(format!(
        "restore_cli_{}_{}",
        timestamp,
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&restore_temp)?;

    unpack_archive(path, &restore_temp)?;

    let restored_db = restore_temp.join("panel.db");
    if !restored_db.is_file() {
        return Err("Cadangan tidak memuat database utama".into());
    }
    // No connection may keep the old inode or apply old WAL frames to new data.
    // The operator must also stop any other local CLI/agent database users.
    pool.close().await;
    for (name, target) in [
        ("panel.db", core_path.clone()),
        (
            "operations.db",
            panel_db::operations::sidecar_path(&core_path),
        ),
        (
            "analytics.db",
            std::env::var_os("PANEL_ANALYTICS_DB_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| core_path.with_file_name("analytics.db")),
        ),
    ] {
        let source = restore_temp.join(name);
        restore_database_file(&source, &target, name == "operations.db")?;
    }

    let _ = std::fs::remove_dir_all(&restore_temp);

    println!(
        "\x1b[32m[SUKSES]\x1b[0m Pemulihan sistem berhasil diselesaikan dari {}",
        path_str
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::restore_database_file;

    #[test]
    fn restored_database_replaces_contents_and_discards_stale_wal() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("snapshot.db");
        let target = temp.path().join("panel.db");
        let wal = temp.path().join("panel.db-wal");
        let shm = temp.path().join("panel.db-shm");
        std::fs::write(&source, b"snapshot").unwrap();
        std::fs::write(&target, b"old database").unwrap();
        std::fs::write(&wal, b"old WAL").unwrap();
        std::fs::write(&shm, b"old SHM").unwrap();

        restore_database_file(&source, &target, false).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"snapshot");
        assert!(!wal.exists());
        assert!(!shm.exists());
        // Missing sidecars on subsequent restores are harmless.
        restore_database_file(&source, &target, false).unwrap();
    }

    #[test]
    fn missing_operations_snapshot_removes_previous_database_and_wal() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("missing.db");
        let target = temp.path().join("operations.db");
        let wal = temp.path().join("operations.db-wal");
        let shm = temp.path().join("operations.db-shm");
        for path in [&target, &wal, &shm] {
            std::fs::write(path, b"stale").unwrap();
        }

        restore_database_file(&source, &target, true).unwrap();

        assert!(!target.exists());
        assert!(!wal.exists());
        assert!(!shm.exists());
        restore_database_file(&source, &target, true).unwrap();
    }

    #[test]
    fn missing_optional_snapshot_preserves_database_and_wal() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("missing.db");
        let target = temp.path().join("analytics.db");
        let wal = temp.path().join("analytics.db-wal");
        let shm = temp.path().join("analytics.db-shm");
        for path in [&target, &wal, &shm] {
            std::fs::write(path, b"keep").unwrap();
        }

        restore_database_file(&source, &target, false).unwrap();

        for path in [&target, &wal, &shm] {
            assert_eq!(std::fs::read(path).unwrap(), b"keep");
        }
    }
}
