//! Untrusted SQL and tenant paths never run with OS or database administrator privileges.
use crate::{AgentError, system::SystemRunner};
#[cfg(unix)]
use panel_core::DatabaseEngine;
use panel_proto::{DatabaseWebCredentials, DatabaseWebCredentialsRequest};

pub struct DatabaseFileTransfer<'a> {
    pub engine: &'a str,
    pub name: &'a str,
    pub db_user: &'a str,
    pub tenant_username: &'a str,
    pub tenant_uid: u32,
    pub path: &'a str,
    pub import: bool,
}

pub fn transfer(
    runner: &dyn SystemRunner,
    request: &DatabaseFileTransfer<'_>,
) -> Result<(), AgentError> {
    crate::backups::identity(runner, request.tenant_username, Some(request.tenant_uid))?;
    crate::node_site::validate_app_directory(
        request.tenant_username,
        request.tenant_uid,
        request.path,
    )?;
    let credentials = crate::database_web::credentials(
        runner,
        &DatabaseWebCredentialsRequest {
            engine: request.engine.into(),
            name: request.name.into(),
            db_user: request.db_user.into(),
            tenant_username: request.tenant_username.into(),
            tenant_uid: request.tenant_uid,
            provision: true,
        },
    )?;
    runner.database_file_transfer(request, &credentials)
}

#[cfg(unix)]
fn tenant_command(binary: &str, request: &DatabaseFileTransfer<'_>) -> std::process::Command {
    let executable = panel_core::production::system_command(binary)
        .get_program()
        .to_owned();
    let mut command = panel_core::production::system_command("systemd-run");
    command
        .env_clear()
        .env("PATH", panel_core::production::SYSTEM_PATH)
        .args(["--quiet", "--wait", "--pipe", "--collect"])
        .arg(format!("--uid={}", request.tenant_uid))
        .arg(format!("--gid={}", request.tenant_uid))
        .arg(format!("--slice=panel-u{}.slice", request.tenant_uid))
        .args([
            "--property=NoNewPrivileges=yes",
            "--property=PrivateTmp=yes",
            "--property=ProtectSystem=strict",
            "--property=MemoryMax=512M",
            "--property=TasksMax=64",
            "--property=RuntimeMaxSec=900s",
            "--property=TimeoutStopSec=5s",
        ])
        .arg(format!(
            "--property=ReadWritePaths=/home/{}",
            request.tenant_username
        ))
        .arg(format!("--setenv=HOME=/home/{}", request.tenant_username))
        .arg(format!(
            "--setenv=PATH={}",
            panel_core::production::SYSTEM_PATH
        ))
        .args([
            "--setenv=LC_ALL=C",
            "--setenv=PGPASSFILE",
            "--working-directory=/",
            "--",
        ])
        .arg(executable);
    // Only a path to a private credential file crosses systemd's environment.
    // Password values never enter unit properties, argv, or the manager environment.
    command
}

#[cfg(unix)]
struct CredentialFiles(std::path::PathBuf);
#[cfg(unix)]
impl Drop for CredentialFiles {
    fn drop(&mut self) {
        // The directory is root-owned and never tenant-writable. Delete only
        // these fixed entries; do not recursively follow anything supplied by a tenant.
        let _ = std::fs::remove_file(self.0.join("client.cnf"));
        let _ = std::fs::remove_file(self.0.join("pgpass"));
        let _ = std::fs::remove_dir(&self.0);
    }
}
#[cfg(unix)]
fn credential_files(
    request: &DatabaseFileTransfer<'_>,
    credentials: &DatabaseWebCredentials,
) -> Result<CredentialFiles, AgentError> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let path = std::path::Path::new("/run/panel")
        .join(format!("database-transfer-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&path)?;
    let files = CredentialFiles(path);
    std::os::unix::fs::chown(&files.0, Some(0), Some(request.tenant_uid))?;
    std::fs::set_permissions(&files.0, std::fs::Permissions::from_mode(0o710))?;
    panel_core::validate_database_password(&credentials.password)
        .map_err(|_| AgentError::SecurityViolation("Kredensial transfer tidak valid".into()))?;
    let mysql = format!("[client]\npassword=\"{}\"\n", credentials.password);
    let escape_pg = |text: &str| text.replace('\\', "\\\\").replace(':', "\\:");
    let pgpass = format!(
        "127.0.0.1:5432:{}:{}:{}\n",
        escape_pg(request.name),
        escape_pg(&credentials.username),
        escape_pg(&credentials.password)
    );
    for (name, contents) in [("client.cnf", mysql), ("pgpass", pgpass)] {
        let target = files.0.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)?;
        file.write_all(contents.as_bytes())?;
        std::os::unix::fs::chown(&target, Some(request.tenant_uid), Some(request.tenant_uid))?;
    }
    Ok(files)
}

#[cfg(unix)]
pub(crate) fn execute(
    request: &DatabaseFileTransfer<'_>,
    credentials: &DatabaseWebCredentials,
) -> Result<(), AgentError> {
    use std::process::Stdio;
    panel_core::tenant::TenantUid::parse(request.tenant_uid)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    let engine = DatabaseEngine::parse(request.engine)
        .map_err(|e| AgentError::InvalidInput(e.to_string()))?;
    let binary = match (engine, request.import) {
        (DatabaseEngine::MariaDb, true) => "mariadb",
        (DatabaseEngine::MariaDb, false) => "mariadb-dump",
        (DatabaseEngine::PostgreSql, true) => "psql",
        (DatabaseEngine::PostgreSql, false) => "pg_dump",
    };
    let credential_files = credential_files(request, credentials)?;
    let mut database = tenant_command(binary, request);
    match engine {
        DatabaseEngine::MariaDb => {
            database.args([
                &format!(
                    "--defaults-file={}",
                    credential_files.0.join("client.cnf").display()
                ),
                "--host=localhost",
                "--user",
                &credentials.username,
            ]);
            if request.import {
                database.args([
                    "--binary-mode",
                    "--batch",
                    "--local-infile=0",
                    "--database",
                    request.name,
                ]);
            } else {
                database.args([
                    "--single-transaction",
                    "--quick",
                    "--routines",
                    "--triggers",
                    "--no-tablespaces",
                    request.name,
                ]);
            }
        }
        DatabaseEngine::PostgreSql => {
            database
                .args([
                    "--host=127.0.0.1",
                    "--no-password",
                    "--username",
                    &credentials.username,
                    "--dbname",
                    request.name,
                ])
                .env("PGPASSFILE", credential_files.0.join("pgpass"));
            if request.import {
                database.args(["--no-psqlrc", "--set", "ON_ERROR_STOP=1"]);
            } else {
                database.args(["--no-owner", "--no-acl"]);
            }
        }
    }
    // Neither child inherits agent secrets or root. Tenant path
    // resolution happens only after dropping credentials, including symlinks.
    let mut file = tenant_command(if request.import { "cat" } else { "tee" }, request);
    file.args(["--", request.path]);
    if !request.import {
        let parent = std::path::Path::new(request.path)
            .parent()
            .ok_or_else(|| AgentError::InvalidInput("Direktori dump tidak valid".into()))?;
        let status = tenant_command("mkdir", request)
            .args([
                std::ffi::OsStr::new("-p"),
                std::ffi::OsStr::new("--"),
                parent.as_os_str(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(AgentError::Internal(
                "Direktori dump tidak dapat dibuat sebagai tenant".into(),
            ));
        }
    }
    let (mut producer, mut consumer) = if request.import {
        (file, database)
    } else {
        (database, file)
    };
    let mut source = producer
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let pipe = source
        .stdout
        .take()
        .ok_or_else(|| AgentError::Internal("Pipe dump tidak tersedia".into()))?;
    let destination = consumer
        .stdin(Stdio::from(pipe))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut destination = match destination {
        Ok(child) => child,
        Err(error) => {
            let _ = source.kill();
            let _ = source.wait();
            return Err(error.into());
        }
    };
    let target_status = destination.wait();
    if target_status
        .as_ref()
        .map_or(true, |status| !status.success())
    {
        let _ = source.kill();
    }
    let source_status = source.wait();
    if !target_status?.success() || !source_status?.success() {
        return Err(AgentError::Internal(
            "Transfer SQL sebagai tenant gagal; periksa akun dan berkas database".into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn execute(
    _request: &DatabaseFileTransfer<'_>,
    _credentials: &DatabaseWebCredentials,
) -> Result<(), AgentError> {
    Err(AgentError::SecurityViolation(
        "Transfer SQL memerlukan isolasi Linux".into(),
    ))
}
