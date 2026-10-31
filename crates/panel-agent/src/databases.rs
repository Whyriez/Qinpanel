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

fn sql_identifier(value: &str) -> Result<(), AgentError> {
    if value.trim() != value {
        return Err(AgentError::InvalidInput(
            "Identifier database tidak valid".into(),
        ));
    }
    panel_core::database::validate_database_encoding(value)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))
}

fn administrator_sql(
    runner: &dyn SystemRunner,
    engine: DatabaseEngine,
    sql: &str,
) -> Result<(), AgentError> {
    let result = match engine {
        DatabaseEngine::MariaDb => {
            runner.run_command_with_input("mariadb", &["--batch"], sql.as_bytes())
        }
        DatabaseEngine::PostgreSql => runner.run_command_with_input(
            "runuser",
            &[
                "--user",
                "postgres",
                "--",
                "psql",
                "--no-psqlrc",
                "--set",
                "ON_ERROR_STOP=1",
                "--dbname",
                "postgres",
            ],
            sql.as_bytes(),
        ),
    };
    result.map(|_| ()).map_err(|_| {
        AgentError::Internal("Operasi administrasi database gagal; periksa layanan database".into())
    })
}

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

    for value in [&req.name, &req.db_user, &req.character_set, &req.collation] {
        sql_identifier(value)?;
    }

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
                req.name.replace('_', "\\_"),
                req.db_user,
                host_spec
            );

            let combined_sql = format!("{create_db_sql} {create_user_sql} {grant_sql}");
            administrator_sql(runner, engine, &combined_sql)?;
        }
        DatabaseEngine::PostgreSql => {
            // A caller-selected password may contain '$$'. A fresh delimiter
            // prevents it from terminating the surrounding procedural block.
            let delimiter = format!("$qp_{}$", uuid::Uuid::new_v4().simple());
            let create_user_sql = format!(
                "DO {delimiter} BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '{}') THEN CREATE USER \"{}\" WITH PASSWORD '{}'; ELSE ALTER USER \"{}\" WITH PASSWORD '{}'; END IF; END {delimiter};",
                req.db_user, req.db_user, req.password, req.db_user, req.password
            );
            administrator_sql(runner, engine, &create_user_sql)?;

            let create_db_sql = format!(
                "SELECT 'CREATE DATABASE \"{}\" OWNER \"{}\" ENCODING ''{}''' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '{}')\n\\gexec\n",
                req.name, req.db_user, req.character_set, req.name
            );
            administrator_sql(runner, engine, &create_db_sql)?;

            let grant_sql = format!(
                "GRANT ALL PRIVILEGES ON DATABASE \"{}\" TO \"{}\";",
                req.name, req.db_user
            );
            administrator_sql(runner, engine, &grant_sql)?;
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

    sql_identifier(&req.name)?;
    sql_identifier(&req.db_user)?;
    match engine {
        DatabaseEngine::MariaDb => {
            let sql = format!(
                "DROP DATABASE IF EXISTS `{}`; DROP USER IF EXISTS '{}'@'localhost'; DROP USER IF EXISTS '{}'@'%'; FLUSH PRIVILEGES;",
                req.name, req.db_user, req.db_user
            );
            administrator_sql(runner, engine, &sql)?;
            crate::database_web::remove_account(runner, engine, &req.name)?;
        }
        DatabaseEngine::PostgreSql => {
            let drop_db_sql = format!("DROP DATABASE IF EXISTS \"{}\";", req.name);
            administrator_sql(runner, engine, &drop_db_sql)?;
            crate::database_web::remove_account(runner, engine, &req.name)?;

            let drop_user_sql = format!("DROP USER IF EXISTS \"{}\";", req.db_user);
            let _ = administrator_sql(runner, engine, &drop_user_sql);
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

    sql_identifier(&req.name)?;
    sql_identifier(&req.db_user)?;
    validate_allowed_hosts(&req.allowed_hosts, true)
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
            administrator_sql(runner, engine, &sql)?;
        }
        DatabaseEngine::PostgreSql => {
            let sql = format!(
                "ALTER USER \"{}\" WITH PASSWORD '{}';",
                req.db_user, req.new_password
            );
            administrator_sql(runner, engine, &sql)?;
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

    sql_identifier(&req.name)?;
    sql_identifier(&req.db_user)?;
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
                req.name.replace('_', "\\_"),
                req.db_user,
                host_spec
            );
            administrator_sql(runner, engine, &sql)?;
        } else {
            let sql = format!(
                "REVOKE ALL PRIVILEGES ON `{}`.* FROM '{}'@'%'; FLUSH PRIVILEGES;",
                req.name.replace('_', "\\_"),
                req.db_user
            );
            let _ = administrator_sql(runner, engine, &sql);
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

    crate::database_transfer::transfer(
        runner,
        &crate::database_transfer::DatabaseFileTransfer {
            engine: &req.engine,
            name: &req.name,
            db_user: req.db_user.as_deref().ok_or_else(|| {
                AgentError::InvalidInput("Pengguna database wajib tersedia".into())
            })?,
            tenant_username: &req.tenant_username,
            tenant_uid: req.tenant_uid,
            path: &req.export_file_path,
            import: false,
        },
    )?;
    let size_bytes = std::fs::metadata(&req.export_file_path)
        .map(|m| m.len())
        .unwrap_or(0);

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

    crate::database_transfer::transfer(
        runner,
        &crate::database_transfer::DatabaseFileTransfer {
            engine: &req.engine,
            name: &req.name,
            db_user: req.db_user.as_deref().ok_or_else(|| {
                AgentError::InvalidInput("Pengguna database wajib tersedia".into())
            })?,
            tenant_username: &req.tenant_username,
            tenant_uid: req.tenant_uid,
            path: &req.import_file_path,
            import: true,
        },
    )?;

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

    sql_identifier(&req.name)?;
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
    #[test]
    fn sql_fragments_are_rejected_before_privileged_commands() {
        for encoding in ["utf8mb4; SELECT 1", "utf8mb4\n", "utf8mb4'", "UTF8$$", ""] {
            let mock = MockSystemRunner::new();
            let req = DatabaseCreateRequest {
                engine: "mariadb".into(),
                name: "p_alice_db".into(),
                db_user: "p_alice_db".into(),
                password: "fixturePassword123!".into(),
                remote_access: false,
                allowed_hosts: "localhost".into(),
                character_set: encoding.into(),
                collation: "utf8mb4_unicode_ci".into(),
            };
            assert!(create_database(&mock, &req).is_err(), "{encoding}");
            assert!(mock.recorded_actions().is_empty());
        }
    }
    #[test]
    fn postgres_password_cannot_terminate_the_procedural_block() {
        let mock = MockSystemRunner::new();
        let req = DatabaseCreateRequest {
            engine: "postgresql".into(),
            name: "p_alice_db".into(),
            db_user: "p_alice_db".into(),
            password: "fixturePass$$word123!".into(),
            remote_access: false,
            allowed_hosts: "localhost".into(),
            character_set: "UTF8".into(),
            collation: "utf8mb4_unicode_ci".into(),
        };
        create_database(&mock, &req).unwrap();
        let commands = mock.get_commands();
        assert!(
            !commands
                .iter()
                .any(|command| command.contains("fixturePass$$word123!"))
        );
        // The validated SQL is sent over stdin; the mock records no password.
        assert!(
            commands
                .iter()
                .any(|command| command.starts_with("runuser --user postgres -- psql --no-psqlrc"))
        );
    }
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
        mock.set_file_content(crate::crypto::DEFAULT_MASTER_KEY_PATH, &[7; 32]);
        mock.set_response(
            "getent passwd p_alice",
            0,
            b"p_alice:x:10001:10001::/home/p_alice:/bin/bash\n",
            b"",
        );
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
        assert_eq!(exp_res.size_bytes, 0); // The mock never writes to the host.
        assert!(mock.recorded_actions().iter().any(|a| matches!(
            a,
            crate::system::MockAction::DatabaseTransfer {
                import: false,
                uid: 10001,
                ..
            }
        )));

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
