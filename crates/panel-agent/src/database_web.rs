//! Private web tools and local, database-scoped SSO accounts. Secrets stay in the agent vault.
use crate::{AgentError, crypto, system::SystemRunner};
use panel_core::database::{tenant_db_prefix, validate_database_name, validate_database_user};
use panel_core::{DatabaseEngine, compute_bytes_sha256, generate_secure_db_password};
use panel_proto::{DatabaseWebCredentials, DatabaseWebCredentialsRequest, DatabaseWebStatus};
use std::path::{Path, PathBuf};

const ENTRY: &str = "/usr/share/panel/db-web/entry.php";
const SERVICE: &str = "panel-db-web.service";
static ACCOUNT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn status(runner: &dyn SystemRunner) -> DatabaseWebStatus {
    let installed = [
        ENTRY,
        "/usr/share/phpmyadmin/index.php",
        "/usr/share/adminer/adminer.php",
        "/usr/bin/php-cgi",
    ]
    .iter()
    .all(|path| runner.path_exists(Path::new(path)));
    let ready = installed
        && runner.path_exists(Path::new("/run/panel-db-web/manager.sock"))
        && runner
            .run_command("systemctl", &["is-active", SERVICE])
            .is_ok_and(|out| {
                out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "active"
            });
    DatabaseWebStatus { installed, ready }
}

pub fn install(runner: &dyn SystemRunner) -> Result<DatabaseWebStatus, AgentError> {
    let rpm = runner.operating_system()?.is_rpm();
    if rpm {
        crate::platform::install(
            runner,
            &[
                "acl",
                "php-cli",
                "php-mysqlnd",
                "php-pgsql",
                "php-mbstring",
                "php-xml",
                "php-common",
                "php-pecl-zip",
                "phpMyAdmin",
            ],
        )?;
        crate::platform::alias(
            runner,
            Path::new("/usr/share/phpMyAdmin"),
            Path::new("/usr/share/phpmyadmin"),
        )?;
        // EPEL has no Adminer package. Pin the upstream release and its published
        // SHA-256; never execute a rolling 'latest.php' download as root.
        let adminer = runner.run_command(
            "curl",
            &[
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "120",
                "--max-filesize",
                "2097152",
                "https://github.com/vrana/adminer/releases/download/v6.2.0/adminer-6.2.0.php",
            ],
        )?;
        if compute_bytes_sha256(&adminer.stdout)
            != "9047c00dfc8258c6046c6720d7d201eaa0ca016fa0fc554bf0c4c137306e1c16"
        {
            return Err(AgentError::InvalidInput(
                "Checksum Adminer tidak sesuai rilis tersemat".into(),
            ));
        }
        runner.create_dir_all(Path::new("/usr/share/adminer"), 0o755, 0, 0)?;
        runner.write_file(
            Path::new("/usr/share/adminer/adminer.php"),
            &adminer.stdout,
            0o644,
        )?;
    } else {
        if !runner.path_exists(Path::new("/usr/share/phpmyadmin/index.php")) {
            // phpMyAdmin signon does not need its optional configuration database.
            // This also supports PostgreSQL-only hosts and avoids a public Apache alias.
            runner.run_command_with_input("debconf-set-selections", &[], b"phpmyadmin phpmyadmin/dbconfig-install boolean false\nphpmyadmin phpmyadmin/reconfigure-webserver multiselect \n")?;
        }
        // Distribution packages receive distribution security updates. No downloaded PHP executables.
        runner.run_command(
            "apt-get",
            &[
                "install",
                "-y",
                "--no-install-recommends",
                "acl",
                "php-cgi",
                "php-mysql",
                "php-pgsql",
                "php-mbstring",
                "php-xml",
                "php-curl",
                "php-zip",
                "phpmyadmin",
                "adminer",
            ],
        )?;
    }
    if runner
        .run_command("getent", &["passwd", "panel-db-web"])
        .is_err()
    {
        runner.run_command(
            "useradd",
            &[
                "--system",
                "--no-create-home",
                "--user-group",
                "--shell",
                "/usr/sbin/nologin",
                "panel-db-web",
            ],
        )?;
    }
    runner.create_dir_all(Path::new("/usr/share/panel/db-web"), 0o755, 0, 0)?;
    runner.create_dir_all(Path::new("/etc/phpmyadmin/conf.d"), 0o755, 0, 0)?;
    for (path, content) in [
        (
            ENTRY,
            include_str!("../../../templates/database-web/entry.php"),
        ),
        (
            "/usr/share/panel/db-web/signon.php",
            include_str!("../../../templates/database-web/signon.php"),
        ),
        (
            "/etc/phpmyadmin/conf.d/zz-panel.php",
            include_str!("../../../templates/database-web/phpmyadmin.php"),
        ),
        (
            "/etc/panel/db-web.ini",
            include_str!("../../../templates/database-web/php.ini"),
        ),
        (
            "/etc/systemd/system/panel-db-web.service",
            include_str!("../../../templates/database-web/panel-db-web.service"),
        ),
        (
            "/usr/lib/tmpfiles.d/panel-db-web.conf",
            "d /var/lib/panel-db-web 0700 panel-db-web panel-db-web 1d\n",
        ),
    ] {
        let content = if rpm {
            content.replace("/run/mysqld/mysqld.sock", "/var/lib/mysql/mysql.sock")
        } else {
            content.into()
        };
        runner.write_file(Path::new(path), content.as_bytes(), 0o644)?;
    }
    if rpm {
        let path = Path::new("/etc/phpMyAdmin/config.inc.php");
        let mut content = String::from_utf8(runner.read_file(path)?)
            .map_err(|_| AgentError::InvalidInput("Konfigurasi phpMyAdmin bukan UTF-8".into()))?;
        let loader = "if (getenv('PANEL_DB_ENGINE') !== false && is_file('/etc/phpmyadmin/conf.d/zz-panel.php')) { require '/etc/phpmyadmin/conf.d/zz-panel.php'; }";
        if !content.contains(loader) {
            if content.trim_end().ends_with("?>") {
                content.push_str("\n<?php\n");
            }
            content.push_str("\n// Managed by QonPanel: private FastCGI signon.\n");
            content.push_str(loader);
            content.push('\n');
            runner.write_file(path, content.as_bytes(), 0o600)?;
        }
    }
    // Read only the distro tool's own protected configuration. Joining www-data
    // would also grant access to tenant FPM sockets, which this worker must not get.
    for directory in ["/etc/phpmyadmin", "/etc/phpMyAdmin", "/var/lib/phpmyadmin"] {
        if runner.path_exists(Path::new(directory)) {
            runner.run_command(
                "setfacl",
                &["-m", "u:panel-db-web:rx,d:u:panel-db-web:rx", directory],
            )?;
        }
    }
    for file in [
        "/etc/phpMyAdmin/config.inc.php",
        "/etc/phpmyadmin/config.inc.php",
        "/etc/phpmyadmin/config-db.php",
        "/var/lib/phpmyadmin/blowfish_secret.inc.php",
        "/var/lib/phpmyadmin/config.inc.php",
    ] {
        if runner.path_exists(Path::new(file)) {
            runner.set_facl(Path::new(file), "panel-db-web", "r")?;
        }
    }
    runner.run_command("systemctl", &["daemon-reload"])?;
    runner.run_command("systemctl", &["enable", SERVICE])?;
    runner.run_command("systemctl", &["restart", SERVICE])?;
    let mut result = status(runner);
    for _ in 0..20 {
        if result.ready {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        result = status(runner);
    }
    if !result.ready {
        return Err(AgentError::Internal(
            "Pengelola database belum siap; periksa panel-db-web.service".into(),
        ));
    }
    Ok(result)
}

fn account(engine: DatabaseEngine, name: &str) -> (String, PathBuf) {
    let hash = compute_bytes_sha256(format!("{}:{name}", engine.as_str()).as_bytes());
    let username = format!("qpw_{}", &hash[..20]);
    let path = PathBuf::from(format!(
        "/etc/panel/db-web-vault/{}-{name}.secret",
        engine.as_str()
    ));
    (username, path)
}

pub fn credentials(
    runner: &dyn SystemRunner,
    req: &DatabaseWebCredentialsRequest,
) -> Result<DatabaseWebCredentials, AgentError> {
    let engine =
        DatabaseEngine::parse(&req.engine).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    if panel_core::tenant::TenantUid::parse(req.tenant_uid).is_err()
        || panel_core::Username::parse(&req.tenant_username).is_err()
    {
        return Err(AgentError::InvalidInput(
            "Identitas tenant database tidak valid".into(),
        ));
    }
    let prefix = tenant_db_prefix(&req.tenant_username);
    validate_database_name(&req.name, &prefix, engine)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    validate_database_user(&req.db_user, &prefix, engine)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| AgentError::Internal("Lock akun pengelola tidak tersedia".into()))?;
    let (username, path) = account(engine, &req.name);
    let key = crypto::get_or_create_master_key(runner, None)?;
    let password = if runner.path_exists(&path) {
        let encrypted = String::from_utf8(runner.read_file(&path)?)
            .map_err(|_| AgentError::Internal("Vault pengelola rusak".into()))?;
        if !encrypted.starts_with(crypto::CIPHER_PREFIX) {
            return Err(AgentError::SecurityViolation(
                "Vault pengelola wajib terenkripsi".into(),
            ));
        }
        crypto::decrypt_value(&key, &encrypted)?
    } else if req.provision {
        let password = generate_secure_db_password();
        runner.create_dir_all(Path::new("/etc/panel/db-web-vault"), 0o700, 0, 0)?;
        runner.write_file(
            &path,
            crypto::encrypt_value(&key, &password)?.as_bytes(),
            0o600,
        )?;
        password
    } else {
        return Err(AgentError::InvalidInput(
            "Buka pengelola melalui tombol di panel terlebih dahulu".into(),
        ));
    };
    panel_core::validate_database_password(&password)
        .map_err(|_| AgentError::SecurityViolation("Vault pengelola tidak valid".into()))?;
    if req.provision {
        // Secrets use stdin and generic diagnostics, never process arguments or audit payloads.
        let result = match engine {
            DatabaseEngine::MariaDb => {
                // GRANT treats '_' and '%' as wildcards even inside quoted identifiers.
                let database_pattern = req.name.replace('_', "\\_");
                let sql = format!(
                    "CREATE USER IF NOT EXISTS '{username}'@'localhost' IDENTIFIED BY '{password}'; ALTER USER '{username}'@'localhost' IDENTIFIED BY '{password}'; REVOKE ALL PRIVILEGES, GRANT OPTION FROM '{username}'@'localhost'; GRANT ALL PRIVILEGES ON `{database_pattern}`.* TO '{username}'@'localhost';\n"
                );
                runner.run_command_with_input("mariadb", &["--batch"], sql.as_bytes())
            }
            DatabaseEngine::PostgreSql => {
                let sql = format!(
                    "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{username}') THEN CREATE ROLE \"{username}\" LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION; END IF; END $$; ALTER ROLE \"{username}\" LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION; GRANT \"{}\" TO \"{username}\"; ALTER ROLE \"{username}\" IN DATABASE \"{}\" SET role TO '{}';\n",
                    req.db_user, req.name, req.db_user
                );
                runner.run_command_with_input(
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
                )
            }
        };
        if result.is_err() {
            return Err(AgentError::Internal(
                "Gagal menyiapkan akun pengelola database lokal; periksa layanan database".into(),
            ));
        }
    }
    Ok(DatabaseWebCredentials { username, password })
}

/// Called before removing the application role, so PostgreSQL membership cannot prevent removal.
pub fn remove_account(
    runner: &dyn SystemRunner,
    engine: DatabaseEngine,
    name: &str,
) -> Result<(), AgentError> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(AgentError::InvalidInput("Nama database tidak valid".into()));
    }
    let (username, path) = account(engine, name);
    if !runner.path_exists(&path) {
        return Ok(());
    }
    let sql = match engine {
        DatabaseEngine::MariaDb => format!("DROP USER IF EXISTS '{username}'@'localhost';"),
        DatabaseEngine::PostgreSql => format!("DROP ROLE IF EXISTS \"{username}\";"),
    };
    match engine {
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
    }
    .map_err(|_| AgentError::Internal("Gagal menghapus akun pengelola database".into()))?;
    runner.remove_file(&path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};
    fn request() -> DatabaseWebCredentialsRequest {
        DatabaseWebCredentialsRequest {
            engine: "mariadb".into(),
            name: "p_alice_shop".into(),
            db_user: "p_alice_shop".into(),
            tenant_username: "p_alice".into(),
            tenant_uid: 10001,
            provision: true,
        }
    }
    #[test]
    fn scoped_account_uses_encrypted_vault_and_no_password_arguments() {
        let runner = MockSystemRunner::new();
        runner.set_file_content(crypto::DEFAULT_MASTER_KEY_PATH, &[7; 32]);
        let first = credentials(&runner, &request()).unwrap();
        assert!(first.username.starts_with("qpw_"));
        let (_, path) = account(DatabaseEngine::MariaDb, "p_alice_shop");
        let vault = String::from_utf8(runner.read_file(&path).unwrap()).unwrap();
        assert!(vault.starts_with(crypto::CIPHER_PREFIX));
        assert!(!vault.contains(&first.password));
        let mut read = request();
        read.provision = false;
        assert_eq!(credentials(&runner, &read).unwrap(), first);
        assert!(!format!("{:?}", runner.recorded_actions()).contains(&first.password));
        assert!(!format!("{first:?}").contains(&first.password));
        assert!(runner.recorded_actions().iter().any(|action| matches!(action, MockAction::Command { binary, args } if binary == "mariadb" && args == &["--batch"])));
        remove_account(&runner, DatabaseEngine::MariaDb, "p_alice_shop").unwrap();
        assert!(!runner.path_exists(&path));
    }
    #[test]
    fn cross_tenant_invalid_uid_and_sql_paths_fail_before_commands() {
        for field in 0..5 {
            let runner = MockSystemRunner::new();
            let mut req = request();
            match field {
                0 => req.name = "p_bob_shop".into(),
                1 => req.db_user = "root".into(),
                2 => req.tenant_uid = 0,
                3 => req.name = "p_alice_a; DROP".into(),
                _ => req.tenant_username = "p_root".into(),
            }
            assert!(credentials(&runner, &req).is_err());
            assert!(runner.recorded_actions().is_empty());
        }
    }
    #[test]
    fn get_does_not_provision_and_postgresql_runs_as_postgres() {
        let runner = MockSystemRunner::new();
        runner.set_file_content(crypto::DEFAULT_MASTER_KEY_PATH, &[7; 32]);
        let mut req = request();
        req.engine = "postgresql".into();
        req.provision = false;
        assert!(credentials(&runner, &req).is_err());
        assert!(
            !runner
                .recorded_actions()
                .iter()
                .any(|a| matches!(a, MockAction::Command { .. }))
        );
        req.provision = true;
        credentials(&runner, &req).unwrap();
        assert!(runner.recorded_actions().iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "runuser" && args[1] == "postgres" && args.contains(&"ON_ERROR_STOP=1".to_string()))));
    }
    #[test]
    fn installer_has_private_socket_and_unprivileged_service() {
        let runner = MockSystemRunner::new();
        for path in [
            ENTRY,
            "/usr/share/phpmyadmin/index.php",
            "/usr/share/adminer/adminer.php",
            "/usr/bin/php-cgi",
            "/run/panel-db-web/manager.sock",
        ] {
            runner.add_existing_path(path);
        }
        runner.set_response("systemctl", 0, b"active\n", b"");
        assert!(install(&runner).unwrap().ready);
        let unit = String::from_utf8(
            runner
                .read_file(Path::new("/etc/systemd/system/panel-db-web.service"))
                .unwrap(),
        )
        .unwrap();
        assert!(unit.contains("User=panel-db-web"));
        assert!(unit.contains("/run/panel-db-web/manager.sock"));
        assert!(unit.contains("ProtectHome=true"));
        assert!(!unit.contains("SupplementaryGroups=www-data"));
        assert!(unit.contains("d:u:panel-api:rw"));
        assert!(!unit.contains("127.0.0.1:"));
        assert!(runner.recorded_actions().iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "apt-get" && args.contains(&"phpmyadmin".to_string()) && args.contains(&"adminer".to_string()) && !args.contains(&"php-fpm".to_string()))));
    }
}
