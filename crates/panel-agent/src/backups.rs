//! Tenant-scoped site archives. All tenant filesystem commands drop privileges first.
use crate::{AgentError, system::SystemRunner};
use panel_core::backup::{
    BackupDatabaseManifestEntry, BackupFormat, BackupManifest, validate_backup_filename,
};
use panel_proto::{
    BackupCreateRequest, BackupCreateResponse, BackupDatabaseInfo, BackupDeleteRequest,
    BackupDeleteResponse, BackupRestoreRequest, BackupRestoreResponse,
};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn invalid() -> AgentError {
    AgentError::InvalidInput("Identitas atau direktori cadangan tenant tidak valid".into())
}
fn identity(runner: &dyn SystemRunner, name: &str, uid: Option<u32>) -> Result<u32, AgentError> {
    panel_core::Username::parse(name).map_err(|_| invalid())?;
    let out = runner.run_command("getent", &["passwd", name])?;
    let record = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<_> = record.trim_end().split(':').collect();
    if !out.status.success()
        || fields.len() != 7
        || fields[0] != name
        || fields[5] != format!("/home/{name}")
    {
        return Err(invalid());
    }
    let actual: u32 = fields[2].parse().map_err(|_| invalid())?;
    panel_core::tenant::TenantUid::parse(actual).map_err(|_| invalid())?;
    if fields[3] != fields[2] || uid.is_some_and(|v| v != actual) {
        return Err(invalid());
    }
    Ok(actual)
}
fn tenant_path(name: &str, uid: u32, path: &str) -> Result<(), AgentError> {
    crate::node_site::validate_app_directory(name, uid, path)
}
fn tenant(runner: &dyn SystemRunner, name: &str, args: &[&str]) -> Result<(), AgentError> {
    let mut command = vec!["-u", name, "--"];
    command.extend_from_slice(args);
    let out = runner.run_command("runuser", &command)?;
    if !out.status.success() {
        return Err(AgentError::Internal("Operasi cadangan tenant gagal".into()));
    }
    Ok(())
}
fn database(db: &BackupDatabaseInfo) -> Result<(), AgentError> {
    if db.name.is_empty()
        || db.name.len() > 64
        || !db
            .name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || db.name.starts_with(|c: char| !c.is_ascii_alphabetic())
        || !["mysql", "mariadb", "postgres", "postgresql"].contains(&db.engine.as_str())
    {
        return Err(invalid());
    }
    Ok(())
}
fn pg(db: &BackupDatabaseInfo) -> bool {
    ["postgres", "postgresql"].contains(&db.engine.as_str())
}
fn string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn private_dump(runner: &dyn SystemRunner) -> Result<PathBuf, AgentError> {
    let dir = PathBuf::from("/var/lib/qonpanel-migration/legacy-dumps")
        .join(uuid::Uuid::new_v4().to_string());
    runner.create_dir_all(&dir, 0o700, 0, 0)?;
    Ok(dir.join("database.sql"))
}
fn cleanup_dump(runner: &dyn SystemRunner, path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = runner.remove_dir_all(parent);
    }
}
fn digest(path: &Path) -> Result<(u64, String), AgentError> {
    // Tenant-owned output is only read here, never chmod/chown or written as root.
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}
fn scan_dir_stats(path: &Path) -> (u64, u64) {
    let mut total = (0, 0);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.path().symlink_metadata() {
                if meta.is_symlink() {
                    continue;
                }
                if meta.is_file() {
                    total.0 += 1;
                    total.1 += meta.len();
                } else if meta.is_dir() {
                    let sub = scan_dir_stats(&entry.path());
                    total.0 += sub.0;
                    total.1 += sub.1;
                }
            }
        }
    }
    total
}

pub fn create_backup(
    runner: &dyn SystemRunner,
    req: &BackupCreateRequest,
) -> Result<BackupCreateResponse, AgentError> {
    let uid = identity(runner, &req.tenant_username, Some(req.tenant_uid))?;
    tenant_path(&req.tenant_username, uid, &req.site_dir)?;
    tenant_path(&req.tenant_username, uid, &req.backup_dir)?;
    validate_backup_filename(&req.filename).map_err(|_| invalid())?;
    for db in &req.databases {
        database(db)?;
    }
    let name = &req.tenant_username;
    let backup_dir = Path::new(&req.backup_dir);
    let staging = backup_dir.join(format!(".staging_{}", uuid::Uuid::new_v4()));
    let stage = string(&staging);
    tenant(runner, name, &["mkdir", "-p", "-m", "0700", "--", &stage])?;
    let result = (|| {
        let mut manifest = BackupManifest {
            site_id: req.site_id,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            includes_files: req.include_files,
            includes_databases: req.include_databases,
            archive_format: req.format.clone(),
            ..Default::default()
        };
        if req.include_files {
            let dest = string(&staging.join("files"));
            tenant(runner, name, &["mkdir", "-p", "-m", "0700", "--", &dest])?;
            tenant(
                runner,
                name,
                &[
                    "cp",
                    "-a",
                    "--no-dereference",
                    "--",
                    &format!("{}/.", req.site_dir),
                    &dest,
                ],
            )?;
            let (count, bytes) = scan_dir_stats(&staging.join("files"));
            manifest.files_count = count;
            manifest.uncompressed_bytes = bytes;
        }
        if req.include_databases {
            let dest = string(&staging.join("databases"));
            tenant(runner, name, &["mkdir", "-p", "-m", "0700", "--", &dest])?;
            for db in &req.databases {
                let private = private_dump(runner)?;
                let rel = format!("databases/{}.sql", db.name);
                let target = string(&staging.join(&rel));
                let export = (|| {
                    if pg(db) {
                        runner.backup_file_stream(
                            "runuser",
                            &[
                                "-u",
                                "postgres",
                                "--",
                                "pg_dump",
                                "--no-owner",
                                "--no-acl",
                                "--dbname",
                                &db.name,
                            ],
                            &private,
                            false,
                        )?;
                    } else {
                        // Root can write only an agent-generated private file, never a tenant symlink.
                        runner.backup_file_stream(
                            "mariadb-dump",
                            &[
                                "--single-transaction",
                                "--quick",
                                "--routines",
                                "--triggers",
                                "--no-tablespaces",
                                &db.name,
                            ],
                            &private,
                            false,
                        )?;
                    }
                    runner.backup_file_stream(
                        "runuser",
                        &["-u", name, "--", "tee", "--", &target],
                        &private,
                        true,
                    )?;
                    Ok::<(), AgentError>(())
                })();
                cleanup_dump(runner, &private);
                export?;
                manifest.databases.push(BackupDatabaseManifestEntry {
                    engine: db.engine.clone(),
                    name: db.name.clone(),
                    username: db.username.clone(),
                    dump_file: rel,
                    size_bytes: std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0),
                });
            }
        }
        // The manifest also crosses into the tenant tree with a dropped UID.
        let private = private_dump(runner)?;
        let manifest_result = (|| {
            runner.write_file(
                &private,
                &serde_json::to_vec_pretty(&manifest)
                    .map_err(|e| AgentError::Internal(e.to_string()))?,
                0o600,
            )?;
            runner.backup_file_stream(
                "runuser",
                &[
                    "-u",
                    name,
                    "--",
                    "tee",
                    "--",
                    &string(&staging.join("manifest.json")),
                ],
                &private,
                true,
            )?;
            Ok::<(), AgentError>(())
        })();
        cleanup_dump(runner, &private);
        manifest_result?;
        let archive = backup_dir.join(&req.filename);
        let archive_str = string(&archive);
        if matches!(
            BackupFormat::from_str_loose(&req.format),
            BackupFormat::TarGz
        ) {
            tenant(
                runner,
                name,
                &["tar", "-czf", &archive_str, "-C", &stage, "."],
            )?;
        } else {
            tenant(
                runner,
                name,
                &["tar", "--zstd", "-cf", &archive_str, "-C", &stage, "."],
            )?;
        }
        let (size_bytes, checksum_sha256) = digest(&archive)?;
        Ok(BackupCreateResponse {
            success: true,
            size_bytes,
            checksum_sha256,
            filepath: archive_str,
            error: None,
        })
    })();
    let _ = tenant(runner, name, &["rm", "-rf", "--", &stage]);
    result
}

pub fn restore_backup(
    runner: &dyn SystemRunner,
    req: &BackupRestoreRequest,
) -> Result<BackupRestoreResponse, AgentError> {
    let uid = identity(runner, &req.tenant_username, Some(req.tenant_uid))?;
    tenant_path(&req.tenant_username, uid, &req.backup_filepath)?;
    tenant_path(&req.tenant_username, uid, &req.target_site_dir)?;
    for db in &req.databases {
        database(db)?;
    }
    let name = &req.tenant_username;
    let staging = PathBuf::from(format!("/home/{name}/.restore_{}", uuid::Uuid::new_v4()));
    let stage = string(&staging);
    tenant(runner, name, &["mkdir", "-p", "-m", "0700", "--", &stage])?;
    let result = (|| {
        let compression = if req.backup_filepath.ends_with(".tar.gz") {
            "-xzf"
        } else {
            "-xf"
        };
        let mut args = vec!["tar", "--no-same-owner", "--no-same-permissions"];
        if compression == "-xf" {
            args.push("--zstd");
        }
        args.extend([compression, &req.backup_filepath, "-C", &stage]);
        // GNU tar rejects traversal; even a malicious link only has the tenant's privileges.
        tenant(runner, name, &args)?;
        let mut count = 0;
        if req.restore_files {
            tenant(runner, name, &["mkdir", "-p", "--", &req.target_site_dir])?;
            tenant(
                runner,
                name,
                &[
                    "cp",
                    "-a",
                    "--no-dereference",
                    "--",
                    &format!("{stage}/files/."),
                    &req.target_site_dir,
                ],
            )?;
            count = scan_dir_stats(Path::new(&req.target_site_dir)).0;
        }
        let mut databases = 0;
        if req.restore_databases {
            for db in &req.databases {
                // Import only the explicitly registered database names, never archive-supplied names.
                let input = string(&staging.join(format!("databases/{}.sql", db.name)));
                let private = private_dump(runner)?;
                let import = (|| {
                    runner.backup_file_stream(
                        "runuser",
                        &["-u", name, "--", "cat", "--", &input],
                        &private,
                        false,
                    )?;
                    if pg(db) {
                        runner.backup_file_stream(
                            "runuser",
                            &[
                                "-u",
                                "postgres",
                                "--",
                                "psql",
                                "--no-psqlrc",
                                "--set",
                                "ON_ERROR_STOP=1",
                                "--dbname",
                                &db.name,
                            ],
                            &private,
                            true,
                        )?;
                    } else {
                        // Disable client commands such as system/source and local file loading.
                        runner.backup_file_stream(
                            "mariadb",
                            &[
                                "--binary-mode",
                                "--batch",
                                "--local-infile=0",
                                "--database",
                                &db.name,
                            ],
                            &private,
                            true,
                        )?;
                    }
                    Ok::<(), AgentError>(())
                })();
                cleanup_dump(runner, &private);
                import?;
                databases += 1;
            }
        }
        Ok(BackupRestoreResponse {
            success: true,
            restored_files_count: count,
            restored_databases_count: databases,
            error: None,
        })
    })();
    let _ = tenant(runner, name, &["rm", "-rf", "--", &stage]);
    result
}

pub fn delete_backup(
    runner: &dyn SystemRunner,
    req: &BackupDeleteRequest,
) -> Result<BackupDeleteResponse, AgentError> {
    let uid = identity(runner, &req.tenant_username, None)?;
    tenant_path(&req.tenant_username, uid, &req.filepath)?;
    tenant(
        runner,
        &req.tenant_username,
        &["rm", "-f", "--", &req.filepath],
    )?;
    Ok(BackupDeleteResponse {
        success: true,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    fn runner() -> MockSystemRunner {
        let runner = MockSystemRunner::new();
        runner.set_response(
            "getent passwd p_alice",
            0,
            b"p_alice:x:10000:10000::/home/p_alice:/usr/sbin/nologin\n",
            b"",
        );
        runner
    }
    #[test]
    fn test_delete_backup_mock() {
        let runner = runner();
        assert!(
            delete_backup(
                &runner,
                &BackupDeleteRequest {
                    filepath: "/home/p_alice/backups/test.tar.gz".into(),
                    tenant_username: "p_alice".into()
                }
            )
            .unwrap()
            .success
        );
        assert!(
            runner
                .get_commands()
                .iter()
                .any(|c| c == "runuser -u p_alice -- rm -f -- /home/p_alice/backups/test.tar.gz")
        );
    }
    #[test]
    fn backup_cannot_write_root_approval_or_escape_tenant() {
        for target in [
            "/var/lib/qonpanel-migration/approval.json",
            "/home/p_alice/../../etc/systemd/system/evil.service",
        ] {
            let runner = runner();
            assert!(
                restore_backup(
                    &runner,
                    &BackupRestoreRequest {
                        site_id: 1,
                        tenant_username: "p_alice".into(),
                        tenant_uid: 10000,
                        backup_filepath: "/home/p_alice/backups/test.tar.gz".into(),
                        target_site_dir: target.into(),
                        restore_files: true,
                        restore_databases: false,
                        databases: vec![]
                    }
                )
                .is_err()
            );
            assert!(
                runner
                    .get_commands()
                    .iter()
                    .all(|c| c.starts_with("getent "))
            );
        }
        let runner = runner();
        assert!(
            delete_backup(
                &runner,
                &BackupDeleteRequest {
                    filepath: "/var/lib/qonpanel-migration/approval.json".into(),
                    tenant_username: "p_alice".into()
                }
            )
            .is_err()
        );
        assert!(identity(&runner, "p_alice", Some(0)).is_err());
        assert!(identity(&runner, "root", Some(0)).is_err());
    }
    #[test]
    fn restore_drops_uid_and_disables_database_client_shell_commands() {
        let runner = runner();
        let result = restore_backup(
            &runner,
            &BackupRestoreRequest {
                site_id: 1,
                tenant_username: "p_alice".into(),
                tenant_uid: 10000,
                backup_filepath: "/home/p_alice/backups/test.tar.gz".into(),
                target_site_dir: "/home/p_alice/sites/test".into(),
                restore_files: true,
                restore_databases: true,
                databases: vec![BackupDatabaseInfo {
                    name: "alice_db".into(),
                    engine: "mariadb".into(),
                    username: "alice".into(),
                    password: None,
                }],
            },
        )
        .unwrap();
        assert_eq!(result.restored_databases_count, 1);
        let commands = runner.get_commands();
        assert!(commands.iter().any(|c| {
            c.contains("mariadb --binary-mode --batch --local-infile=0 --database alice_db")
        }));
        assert!(
            !commands.iter().any(|c| c.starts_with("tar ")
                || c.starts_with("psql ")
                || c.contains(" -e source"))
        );
        assert!(
            commands
                .iter()
                .any(|c| c.starts_with("runuser -u p_alice -- tar "))
        );
    }
}
