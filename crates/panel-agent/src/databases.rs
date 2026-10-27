//! Privileged database provisioning and service management operations for MariaDB and PostgreSQL.

use crate::AgentError;
use crate::system::SystemRunner;
use panel_core::{
    DatabaseEngine, validate_allowed_hosts, validate_database_password, validate_dump_path,
};
use panel_proto::{
    DatabaseCreateRequest, DatabaseCreateResponse, DatabaseDeleteRequest, DatabaseDeleteResponse,
    DatabaseExportRequest, DatabaseExportResponse, DatabaseImportRequest, DatabaseImportResponse,
    DatabaseRotatePasswordRequest, DatabaseRotatePasswordResponse, DatabaseServerActionRequest,
    DatabaseServerActionResponse, DatabaseServerInstallRequest, DatabaseServerInstallResponse,
    DatabaseServerStatusItem, DatabaseServerStatusRequest, DatabaseServerStatusResponse,
    DatabaseSizeCheckRequest, DatabaseSizeCheckResponse, DatabaseUpdateRemoteAccessRequest,
    DatabaseUpdateRemoteAccessResponse,
};
use std::path::Path;

/// Inspects current installation and running status of database engines.
pub fn get_database_server_status(
    runner: &dyn SystemRunner,
    req: &DatabaseServerStatusRequest,
) -> Result<DatabaseServerStatusResponse, AgentError> {
    let mut servers = Vec::new();

    for engine_str in &req.engines {
        let engine = match DatabaseEngine::parse(engine_str) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let binary_name = match engine {
            DatabaseEngine::MariaDb => "mariadb",
            DatabaseEngine::PostgreSql => "psql",
        };

        let service_name = engine.systemd_service_name();
        let port = engine.default_port();
        let socket_path = Some(engine.default_socket_path().to_string());

        // 1. Check if binary exists or version output works
        let (is_installed, version) = match runner.run_command(binary_name, &["--version"]) {
            Ok(output) if output.status.success() => {
                let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let v_str = if !stdout_str.is_empty() {
                    Some(stdout_str)
                } else if !stderr_str.is_empty() {
                    Some(stderr_str)
                } else {
                    None
                };
                if let Some(v) = v_str {
                    (true, Some(v))
                } else {
                    (false, None)
                }
            }
            _ => (false, None),
        };

        // 2. Check systemd service status
        let is_running = if is_installed {
            match runner.run_command("systemctl", &["is-active", service_name]) {
                Ok(output) => {
                    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    s == "active"
                }
                _ => false,
            }
        } else {
            false
        };

        servers.push(DatabaseServerStatusItem {
            engine: engine.as_str().to_string(),
            is_installed,
            is_running,
            version,
            port,
            socket_path,
        });
    }

    Ok(DatabaseServerStatusResponse { servers })
}

/// Installs a database engine server on demand via apt-get.
pub fn install_database_server(
    runner: &dyn SystemRunner,
    req: &DatabaseServerInstallRequest,
) -> Result<DatabaseServerInstallResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    let packages: &[&str] = match engine {
        DatabaseEngine::MariaDb => &["install", "-y", "mariadb-server", "mariadb-client"],
        DatabaseEngine::PostgreSql => &["install", "-y", "postgresql", "postgresql-contrib"],
    };

    let service_name = engine.systemd_service_name();

    // 1. Run apt-get install
    crate::platform::install(runner, &packages[2..])?;
    if runner.operating_system()?.is_rpm()
        && engine == DatabaseEngine::PostgreSql
        && !runner.path_exists(std::path::Path::new("/var/lib/pgsql/data/PG_VERSION"))
    {
        runner.run_command("postgresql-setup", &["--initdb"])?;
        // RHEL initializes TCP loopback with ident. Panel credentials are SCRAM
        // passwords; adjust only this newly created cluster, never an existing one.
        let path = std::path::Path::new("/var/lib/pgsql/data/pg_hba.conf");
        let bytes = runner.read_file(path)?;
        let content = std::str::from_utf8(&bytes)
            .map_err(|_| AgentError::InvalidInput("pg_hba.conf bukan UTF-8".into()))?;
        let updated = content
            .lines()
            .map(|line| {
                let columns = line.split_whitespace().collect::<Vec<_>>();
                if columns.len() == 5
                    && columns[0] == "host"
                    && columns[1] == "all"
                    && columns[2] == "all"
                    && matches!(columns[3], "127.0.0.1/32" | "::1/128")
                    && columns[4] == "ident"
                {
                    format!("host all all {} scram-sha-256", columns[3])
                } else {
                    line.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        runner.write_file(
            path.with_extension("conf.qonpanel-before").as_path(),
            &bytes,
            0o600,
        )?;
        runner.write_file(path, updated.as_bytes(), 0o600)?;
        // PostgreSQL 13 (AlmaLinux 9 default) otherwise creates MD5 role hashes,
        // which cannot authenticate against the SCRAM loopback rule above.
        let config = std::path::Path::new("/var/lib/pgsql/data/postgresql.conf");
        let original = runner.read_file(config)?;
        let mut settings = original.clone();
        settings.extend_from_slice(b"\n# Managed by QonPanel: newly initialized cluster.\npassword_encryption = 'scram-sha-256'\n");
        runner.write_file(
            config.with_extension("conf.qonpanel-before").as_path(),
            &original,
            0o600,
        )?;
        runner.write_file(config, &settings, 0o600)?;
    }

    // 2. Enable and start systemd service
    runner.run_command("systemctl", &["enable", "--now", service_name])?;

    Ok(DatabaseServerInstallResponse {
        engine: engine.as_str().to_string(),
        success: true,
        message: format!(
            "Engine {} berhasil dipasang dan diaktifkan",
            engine.as_str()
        ),
    })
}

/// Manages systemd lifecycle for a database service (start, stop, restart, status).
pub fn action_database_server(
    runner: &dyn SystemRunner,
    req: &DatabaseServerActionRequest,
) -> Result<DatabaseServerActionResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    let action = req.action.trim().to_ascii_lowercase();
    if !["start", "stop", "restart", "reload", "status"].contains(&action.as_str()) {
        return Err(AgentError::InvalidInput(format!(
            "Aksi service '{}' tidak valid. Pilih start, stop, restart, atau reload.",
            action
        )));
    }

    let service_name = engine.systemd_service_name();
    runner.run_command("systemctl", &[&action, service_name])?;

    Ok(DatabaseServerActionResponse {
        engine: engine.as_str().to_string(),
        action,
        success: true,
        message: format!(
            "Aksi '{}' pada service {} berhasil dieksekusi",
            req.action, service_name
        ),
    })
}

/// Provisions a new database and dedicated user credentials.
pub fn create_database(
    runner: &dyn SystemRunner,
    req: &DatabaseCreateRequest,
) -> Result<DatabaseCreateResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    // Basic domain parameter validations
    validate_database_password(&req.password)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    validate_allowed_hosts(&req.allowed_hosts, req.remote_access)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    match engine {
        DatabaseEngine::MariaDb => {
            let host_spec = if req.remote_access {
                if req.allowed_hosts.trim() == "%" || req.allowed_hosts.trim().is_empty() {
                    "%"
                } else {
                    req.allowed_hosts.split(',').next().unwrap_or("%").trim()
                }
            } else {
                "localhost"
            };

            let create_db_sql = format!(
                "CREATE DATABASE IF NOT EXISTS `{}` CHARACTER SET {} COLLATE {};",
                req.name, req.character_set, req.collation
            );
            let create_user_sql = format!(
                "CREATE USER IF NOT EXISTS '{}'@'{}' IDENTIFIED BY '{}';",
                req.db_user, host_spec, req.password
            );
            let grant_sql = format!(
                "GRANT ALL PRIVILEGES ON `{}`.* TO '{}'@'{}'; FLUSH PRIVILEGES;",
                req.name, req.db_user, host_spec
            );

            let combined_sql = format!("{create_db_sql} {create_user_sql} {grant_sql}");
            runner.run_command("mariadb", &["-e", &combined_sql])?;
        }
        DatabaseEngine::PostgreSql => {
            let create_user_sql = format!(
                "DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '{}') THEN CREATE USER \"{}\" WITH PASSWORD '{}'; ELSE ALTER USER \"{}\" WITH PASSWORD '{}'; END IF; END $$;",
                req.db_user, req.db_user, req.password, req.db_user, req.password
            );
            runner.run_command("psql", &["-U", "postgres", "-c", &create_user_sql])?;

            let create_db_sql = format!(
                "SELECT 'CREATE DATABASE \"{}\" OWNER \"{}\" ENCODING ''{}''' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '{}')\\gexec",
                req.name, req.db_user, req.character_set, req.name
            );
            runner.run_command("psql", &["-U", "postgres", "-c", &create_db_sql])?;

            let grant_sql = format!(
                "GRANT ALL PRIVILEGES ON DATABASE \"{}\" TO \"{}\";",
                req.name, req.db_user
            );
            runner.run_command("psql", &["-U", "postgres", "-c", &grant_sql])?;
        }
    }

    Ok(DatabaseCreateResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        db_user: req.db_user.clone(),
        success: true,
        message: format!(
            "Database '{}' dan pengguna '{}' berhasil dibuat",
            req.name, req.db_user
        ),
    })
}

/// Deletes a database and drops the corresponding user.
pub fn delete_database(
    runner: &dyn SystemRunner,
    req: &DatabaseDeleteRequest,
) -> Result<DatabaseDeleteResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    match engine {
        DatabaseEngine::MariaDb => {
            let sql = format!(
                "DROP DATABASE IF EXISTS `{}`; DROP USER IF EXISTS '{}'@'localhost'; DROP USER IF EXISTS '{}'@'%'; FLUSH PRIVILEGES;",
                req.name, req.db_user, req.db_user
            );
            runner.run_command("mariadb", &["-e", &sql])?;
            crate::database_web::remove_account(runner, engine, &req.name)?;
        }
        DatabaseEngine::PostgreSql => {
            let drop_db_sql = format!("DROP DATABASE IF EXISTS \"{}\";", req.name);
            runner.run_command("psql", &["-U", "postgres", "-c", &drop_db_sql])?;
            crate::database_web::remove_account(runner, engine, &req.name)?;

            let drop_user_sql = format!("DROP USER IF EXISTS \"{}\";", req.db_user);
            let _ = runner.run_command("psql", &["-U", "postgres", "-c", &drop_user_sql]);
        }
    }

    Ok(DatabaseDeleteResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        success: true,
        message: format!("Database '{}' berhasil dihapus", req.name),
    })
}

/// Rotates database user password.
pub fn rotate_database_password(
    runner: &dyn SystemRunner,
    req: &DatabaseRotatePasswordRequest,
) -> Result<DatabaseRotatePasswordResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    validate_database_password(&req.new_password)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    match engine {
        DatabaseEngine::MariaDb => {
            let host_spec = if req.allowed_hosts.is_empty() {
                "localhost"
            } else {
                req.allowed_hosts
                    .split(',')
                    .next()
                    .unwrap_or("localhost")
                    .trim()
            };

            let sql = format!(
                "ALTER USER '{}'@'{}' IDENTIFIED BY '{}'; FLUSH PRIVILEGES;",
                req.db_user, host_spec, req.new_password
            );
            runner.run_command("mariadb", &["-e", &sql])?;
        }
        DatabaseEngine::PostgreSql => {
            let sql = format!(
                "ALTER USER \"{}\" WITH PASSWORD '{}';",
                req.db_user, req.new_password
            );
            runner.run_command("psql", &["-U", "postgres", "-c", &sql])?;
        }
    }

    Ok(DatabaseRotatePasswordResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        db_user: req.db_user.clone(),
        success: true,
        message: format!("Kata sandi pengguna '{}' berhasil dirotasi", req.db_user),
    })
}

/// Updates remote host access privileges.
pub fn update_remote_access(
    runner: &dyn SystemRunner,
    req: &DatabaseUpdateRemoteAccessRequest,
) -> Result<DatabaseUpdateRemoteAccessResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    validate_allowed_hosts(&req.allowed_hosts, req.remote_access)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    if engine == DatabaseEngine::MariaDb {
        if req.remote_access {
            let host_spec =
                if req.allowed_hosts.trim() == "%" || req.allowed_hosts.trim().is_empty() {
                    "%"
                } else {
                    req.allowed_hosts.split(',').next().unwrap_or("%").trim()
                };
            let sql = format!(
                "GRANT ALL PRIVILEGES ON `{}`.* TO '{}'@'{}'; FLUSH PRIVILEGES;",
                req.name, req.db_user, host_spec
            );
            runner.run_command("mariadb", &["-e", &sql])?;
        } else {
            let sql = format!(
                "REVOKE ALL PRIVILEGES ON `{}`.* FROM '{}'@'%'; FLUSH PRIVILEGES;",
                req.name, req.db_user
            );
            let _ = runner.run_command("mariadb", &["-e", &sql]);
        }
    }

    Ok(DatabaseUpdateRemoteAccessResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        db_user: req.db_user.clone(),
        success: true,
        message: "Konfigurasi akses remote database berhasil diperbarui".to_string(),
    })
}

/// Exports a database to a dump file inside tenant home directory.
pub fn export_database(
    runner: &dyn SystemRunner,
    req: &DatabaseExportRequest,
) -> Result<DatabaseExportResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    validate_dump_path(&req.export_file_path, &req.tenant_username)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    let dump_path = Path::new(&req.export_file_path);
    if let Some(parent) = dump_path.parent() {
        runner.create_dir_all(parent, 0o750, req.tenant_uid, req.tenant_uid)?;
    }

    match engine {
        DatabaseEngine::MariaDb => {
            let result_flag = format!("--result-file={}", req.export_file_path);
            runner.run_command(
                "mariadb-dump",
                &["--single-transaction", "--quick", &result_flag, &req.name],
            )?;
        }
        DatabaseEngine::PostgreSql => {
            runner.run_command(
                "pg_dump",
                &["-U", "postgres", "-f", &req.export_file_path, &req.name],
            )?;
        }
    }

    // Set secure permission 0600 and ownership to tenant
    let _ = runner.set_permissions(dump_path, 0o600);
    let _ = runner.set_ownership(dump_path, req.tenant_uid, req.tenant_uid);

    let size_bytes = match runner.read_file(dump_path) {
        Ok(bytes) => bytes.len() as u64,
        Err(_) => 0,
    };

    Ok(DatabaseExportResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        file_path: req.export_file_path.clone(),
        size_bytes,
        success: true,
        message: format!(
            "Ekspor database '{}' berhasil disimpan ke {}",
            req.name, req.export_file_path
        ),
    })
}

/// Imports a dump file into target database.
pub fn import_database(
    runner: &dyn SystemRunner,
    req: &DatabaseImportRequest,
) -> Result<DatabaseImportResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    validate_dump_path(&req.import_file_path, &req.tenant_username)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    let import_path = Path::new(&req.import_file_path);
    if !runner.path_exists(import_path) {
        return Err(AgentError::InvalidInput(format!(
            "Berkas impor tidak ditemukan: '{}'",
            req.import_file_path
        )));
    }

    match engine {
        DatabaseEngine::MariaDb => {
            let source_cmd = format!("source {}", req.import_file_path);
            runner.run_command("mariadb", &["-e", &source_cmd, &req.name])?;
        }
        DatabaseEngine::PostgreSql => {
            runner.run_command(
                "psql",
                &[
                    "-U",
                    "postgres",
                    "-d",
                    &req.name,
                    "-f",
                    &req.import_file_path,
                ],
            )?;
        }
    }

    Ok(DatabaseImportResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        file_path: req.import_file_path.clone(),
        success: true,
        message: format!(
            "Impor berkas '{}' ke database '{}' berhasil diselesaikan",
            req.import_file_path, req.name
        ),
    })
}

/// Queries current database disk size.
pub fn check_database_size(
    runner: &dyn SystemRunner,
    req: &DatabaseSizeCheckRequest,
) -> Result<DatabaseSizeCheckResponse, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;

    let size_bytes = match engine {
        DatabaseEngine::MariaDb => {
            let sql = format!(
                "SELECT IFNULL(SUM(data_length + index_length), 0) FROM information_schema.tables WHERE table_schema = '{}';",
                req.name
            );
            match runner.run_command("mariadb", &["-s", "-N", "-e", &sql]) {
                Ok(output) => {
                    let out = String::from_utf8_lossy(&output.stdout);
                    out.trim().parse::<u64>().unwrap_or(0)
                }
                Err(_) => 0,
            }
        }
        DatabaseEngine::PostgreSql => {
            let sql = format!("SELECT pg_database_size('{}');", req.name);
            match runner.run_command("psql", &["-U", "postgres", "-t", "-c", &sql]) {
                Ok(output) => {
                    let out = String::from_utf8_lossy(&output.stdout);
                    out.trim().parse::<u64>().unwrap_or(0)
                }
                Err(_) => 0,
            }
        }
    };

    Ok(DatabaseSizeCheckResponse {
        engine: engine.as_str().to_string(),
        name: req.name.clone(),
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;

    #[test]
    fn test_database_server_status_and_actions() {
        let mock = MockSystemRunner::new();
        // Simulate mariadb version & systemctl active
        mock.set_response("mariadb --version", 0, b"mariadb from 11.4.2", b"");
        mock.set_response("systemctl is-active mariadb", 0, b"active", b"");
        mock.set_response("psql --version", 1, b"", b"psql: not found");

        let req = DatabaseServerStatusRequest {
            engines: vec!["mariadb".to_string(), "postgresql".to_string()],
        };

        let res = get_database_server_status(&mock, &req).expect("get status");
        assert_eq!(res.servers.len(), 2);
        let m = res
            .servers
            .iter()
            .find(|s| s.engine == "mariadb")
            .expect("m");
        assert!(m.is_installed);
        assert!(m.is_running);
        assert_eq!(m.port, 3306);

        let p = res
            .servers
            .iter()
            .find(|s| s.engine == "postgresql")
            .expect("p");
        assert!(!p.is_installed);

        // Action restart
        let act_req = DatabaseServerActionRequest {
            engine: "mariadb".to_string(),
            action: "restart".to_string(),
        };
        let act_res = action_database_server(&mock, &act_req).expect("action");
        assert!(act_res.success);
    }

    #[test]
    fn test_database_create_rotate_delete_mariadb() {
        let mock = MockSystemRunner::new();

        let create_req = DatabaseCreateRequest {
            engine: "mariadb".to_string(),
            name: "p_alice_blog".to_string(),
            db_user: "p_alice_user".to_string(),
            password: "SecurePass123!@#".to_string(),
            remote_access: false,
            allowed_hosts: "localhost".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
            character_set: "utf8mb4".to_string(),
        };

        let create_res = create_database(&mock, &create_req).expect("create db");
        assert!(create_res.success);
        assert_eq!(create_res.name, "p_alice_blog");

        // Rotate password
        let rotate_req = DatabaseRotatePasswordRequest {
            engine: "mariadb".to_string(),
            name: "p_alice_blog".to_string(),
            db_user: "p_alice_user".to_string(),
            new_password: "NewStrongPassword456!".to_string(),
            allowed_hosts: "localhost".to_string(),
        };
        let rotate_res = rotate_database_password(&mock, &rotate_req).expect("rotate");
        assert!(rotate_res.success);

        // Delete database
        let del_req = DatabaseDeleteRequest {
            engine: "mariadb".to_string(),
            name: "p_alice_blog".to_string(),
            db_user: "p_alice_user".to_string(),
        };
        let del_res = delete_database(&mock, &del_req).expect("delete");
        assert!(del_res.success);
    }

    #[test]
    fn test_database_export_and_import() {
        let mock = MockSystemRunner::new();
        let export_path = "/home/p_alice/backup.sql";
        mock.set_file_content(export_path, b"CREATE TABLE t1;");

        let exp_req = DatabaseExportRequest {
            engine: "mariadb".to_string(),
            name: "p_alice_app".to_string(),
            db_user: Some("p_alice_user".to_string()),
            export_file_path: export_path.to_string(),
            tenant_username: "p_alice".to_string(),
            tenant_uid: 10001,
        };
        let exp_res = export_database(&mock, &exp_req).expect("export");
        assert!(exp_res.success);
        assert_eq!(exp_res.size_bytes, 16);

        let imp_req = DatabaseImportRequest {
            engine: "mariadb".to_string(),
            name: "p_alice_app".to_string(),
            db_user: Some("p_alice_user".to_string()),
            import_file_path: export_path.to_string(),
            tenant_username: "p_alice".to_string(),
            tenant_uid: 10001,
        };
        let imp_res = import_database(&mock, &imp_req).expect("import");
        assert!(imp_res.success);
    }
}
