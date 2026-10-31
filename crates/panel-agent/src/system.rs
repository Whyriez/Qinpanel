//! System command execution and filesystem abstraction for host operations.

use crate::AgentError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::warn;

pub const ALLOWED_BINARIES: &[&str] = &[
    "/usr/bin/php7.4",
    "/usr/bin/php8.0",
    "/usr/bin/php8.1",
    "/usr/bin/php8.2",
    "/usr/bin/php8.3",
    "/usr/bin/php8.4",
    "/usr/bin/php8.5",
    "/usr/local/libexec/qonpanel-mail-auth",
    "dnf",
    "rpm",
    "rpmkeys",
    "postgresql-setup",
    "semanage",
    "getsebool",
    "setsebool",
    "restorecon",
    "postfix",
    "postmap",
    "doveadm",
    "doveconf",
    "opendkim",
    "opendkim-genkey",
    "sendmail",
    "useradd",
    "usermod",
    "userdel",
    "groupadd",
    "groupdel",
    "setfacl",
    "getfacl",
    "pkill",
    "systemctl",
    "runuser",
    "setquota",
    "repquota",
    "xfs_quota",
    "findmnt",
    "df",
    "mount",
    "getent",
    "nginx",
    "apt-get",
    "apt-cache",
    "add-apt-repository",
    "dpkg-query",
    "debconf-set-selections",
    "phpenmod",
    "phpdismod",
    "systemd-analyze",
    "php-fpm7.4",
    "php-fpm8.0",
    "php-fpm8.1",
    "php-fpm8.2",
    "php-fpm8.3",
    "php-fpm8.4",
    "php-fpm8.5",
    "sshd",
    "curl",
    "node",
    "corepack",
    "npm",
    "pnpm",
    "yarn",
    "uv",
    "/usr/local/bin/uv",
    "/usr/bin/uv",
    "python",
    "python3",
    "gunicorn",
    "uvicorn",
    "systemd-run",
    "journalctl",
    "ssh-keygen",
    "git",
    "ssh",
    "sh",
    "bash",
    "mariadb",
    "mysql",
    "mariadb-dump",
    "mysqldump",
    "psql",
    "pg_dump",
    "createdb",
    "dropdb",
    "createuser",
    "dropuser",
    "tar",
    "zstd",
    "gzip",
    "sysctl",
    "nft",
];

/// Trait abstracting underlying operating system commands and filesystem operations.
pub trait SystemRunner: Send + Sync {
    fn database_file_transfer(
        &self,
        _request: &crate::database_transfer::DatabaseFileTransfer<'_>,
        _credentials: &panel_proto::DatabaseWebCredentials,
    ) -> Result<(), AgentError> {
        Err(AgentError::SecurityViolation(
            "Transfer SQL terisolasi tidak tersedia".into(),
        ))
    }
    fn operating_system(&self) -> Result<panel_core::platform::LinuxPlatform, AgentError> {
        let bytes = self.read_file(Path::new("/etc/os-release"))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| AgentError::InvalidInput("Metadata OS bukan UTF-8.".into()))?;
        panel_core::platform::LinuxPlatform::parse_os_release(text)
            .map_err(|message| AgentError::InvalidInput(message.into()))
    }
    fn verify_origin_tls(&self, _hostname: &str, _certificate_pem: &str) -> Result<(), AgentError> {
        Err(AgentError::Internal(
            "Verifikasi TLS origin tidak tersedia.".into(),
        ))
    }
    fn verify_panel_tls(&self, _port: u16, _certificate_pem: &str) -> Result<(), AgentError> {
        Err(AgentError::Internal(
            "Verifikasi listener TLS tidak tersedia pada runner ini.".into(),
        ))
    }
    fn run_command_with_input(
        &self,
        _binary: &str,
        _args: &[&str],
        _input: &[u8],
    ) -> Result<std::process::Output, AgentError> {
        Err(AgentError::SecurityViolation(
            "Command stdin tidak tersedia pada runner ini".into(),
        ))
    }
    fn run_command(&self, binary: &str, args: &[&str]) -> Result<std::process::Output, AgentError>;
    /// Backup streams use private, agent-generated files rather than shell pipes or RAM buffers.
    fn backup_file_stream(
        &self,
        binary: &str,
        args: &[&str],
        _file: &Path,
        _file_is_input: bool,
    ) -> Result<std::process::Output, AgentError> {
        self.run_command(binary, args)
    }
    /// Streaming remains opt-in, with a default implementation for deterministic mocks.
    fn run_command_streamed(
        &self,
        binary: &str,
        args: &[&str],
        on_output: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<std::process::Output, AgentError> {
        let result = self.run_command(binary, args);
        match &result {
            Ok(output) => {
                for bytes in [&output.stdout, &output.stderr] {
                    for chunk in bytes.chunks(2048) {
                        on_output(&String::from_utf8_lossy(chunk));
                    }
                }
            }
            Err(error) => on_output(&error.to_string()),
        }
        result
    }
    /// Node downloads can be cancelled without interrupting package-manager operations.
    fn run_cancellable_download(
        &self,
        args: &[&str],
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<std::process::Output, AgentError> {
        if cancel.is_cancelled() {
            return Err(AgentError::Internal("Unduhan Node.js dibatalkan".into()));
        }
        self.run_command("curl", args)
    }
    /// Only the managed Python installer supports process cancellation, never APT.
    fn run_cancellable_python(
        &self,
        args: &[&str],
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<std::process::Output, AgentError> {
        if cancel.is_cancelled() {
            return Err(AgentError::Internal("Pemasangan Python dibatalkan".into()));
        }
        self.run_command(crate::uv::UV_BIN, args)
    }
    fn create_dir_all(&self, path: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError>;
    fn set_permissions(&self, path: &Path, mode: u32) -> Result<(), AgentError>;
    fn set_ownership(&self, path: &Path, uid: u32, gid: u32) -> Result<(), AgentError>;
    fn set_facl(&self, path: &Path, user: &str, permissions: &str) -> Result<(), AgentError>;
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), AgentError>;
    fn create_symlink(&self, original: &Path, link: &Path) -> Result<(), AgentError>;
    fn read_link(&self, path: &Path) -> Result<PathBuf, AgentError>;
    fn remove_dir_all(&self, path: &Path) -> Result<(), AgentError>;
    fn path_exists(&self, path: &Path) -> bool;
    fn write_file(&self, path: &Path, content: &[u8], mode: u32) -> Result<(), AgentError>;
    /// Creates a new file without replacing a file concurrently written by a tenant.
    fn write_file_if_absent(
        &self,
        path: &Path,
        content: &[u8],
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<bool, AgentError> {
        if self.path_exists(path) {
            return Ok(false);
        }
        self.write_file(path, content, mode)?;
        self.set_ownership(path, uid, gid)?;
        Ok(true)
    }
    fn prepare_owned_file(
        &self,
        path: &Path,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<(), AgentError> {
        if !self.path_exists(path) {
            self.write_file(path, &[], mode)?;
        }
        self.set_ownership(path, uid, gid)?;
        self.set_permissions(path, mode)
    }
    fn read_file(&self, path: &Path) -> Result<Vec<u8>, AgentError>;
    fn remove_file(&self, path: &Path) -> Result<(), AgentError>;
    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, AgentError>;
}

/// Allow tenant runtimes and Nginx to search the IPC directory on the way to
/// their own sockets. This does not grant listing, write, or access to agent.sock.
pub(crate) fn ensure_runtime_parent_access(
    runner: &dyn SystemRunner,
    username: &str,
) -> Result<(), AgentError> {
    let username = panel_core::Username::parse(username)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    let acl = format!("u:{}:--x,u:www-data:--x", username.as_str());
    // set_facl tolerates errors for legacy callers. Runtime access is mandatory.
    let output = runner.run_command("setfacl", &["-m", &acl, "--", "/run/panel"])?;
    if !output.status.success() {
        return Err(AgentError::CommandFailed {
            binary: "setfacl".into(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2048)
                .collect(),
        });
    }
    Ok(())
}

/// Real system runner executing actual Linux system binaries and system calls.
pub struct RealSystemRunner;

/// Kill and reap the download process before reporting cancellation. Concurrent
/// pipe readers prevent a full stderr buffer from blocking cancellation.
fn run_cancellable_curl(
    command: std::process::Command,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<std::process::Output, AgentError> {
    run_cancellable_installer(command, "curl", cancel)
}

fn run_cancellable_installer(
    mut command: std::process::Command,
    binary: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<std::process::Output, AgentError> {
    use std::io::Read;
    use std::process::Stdio;
    if cancel.is_cancelled() {
        return Err(AgentError::Internal("Pemasangan runtime dibatalkan".into()));
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::scope(|scope| {
        let read = |stream: Option<Box<dyn Read + Send>>| -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                stream.read_to_end(&mut bytes)?;
            }
            Ok(bytes)
        };
        let out =
            scope.spawn(move || read(stdout.map(|pipe| Box::new(pipe) as Box<dyn Read + Send>)));
        let err =
            scope.spawn(move || read(stderr.map(|pipe| Box::new(pipe) as Box<dyn Read + Send>)));
        let status = wait_for_download(&mut child, cancel);
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let stdout = out
            .join()
            .map_err(|_| AgentError::Internal("Pembaca stdout unduhan gagal".into()))??;
        let stderr = err
            .join()
            .map_err(|_| AgentError::Internal("Pembaca stderr unduhan gagal".into()))??;
        let status = status?;
        if !status.success() {
            return Err(AgentError::CommandFailed {
                binary: binary.into(),
                code: status.code(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            });
        }
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    })
}

fn wait_for_download(
    child: &mut std::process::Child,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<std::process::ExitStatus, AgentError> {
    loop {
        if cancel.is_cancelled() {
            return Err(AgentError::Internal("Pemasangan runtime dibatalkan".into()));
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(unix)]
// Descriptor-relative no-follow operations require libc; each FFI call is documented below.
#[allow(unsafe_code)]
pub(crate) fn open_owned_runtime_file(
    path: &Path,
    mode: u32,
    uid: u32,
    gid: u32,
    exclusive: bool,
) -> Result<Option<std::fs::File>, AgentError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut directory = std::fs::File::open("/")?;
    let parent = path
        .parent()
        .ok_or_else(|| AgentError::InvalidInput("Parent starter tidak valid".into()))?;
    if !path.is_absolute() {
        return Err(AgentError::InvalidInput(
            "Path starter harus absolut".into(),
        ));
    }
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            if component == std::path::Component::RootDir {
                continue;
            }
            return Err(AgentError::InvalidInput(
                "Komponen path starter tidak valid".into(),
            ));
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| AgentError::InvalidInput("Path starter mengandung NUL".into()))?;
        let mut created = false;
        // SAFETY: directory owns a valid descriptor; name is a NUL-terminated component.
        let mut fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            // SAFETY: same owned directory and validated component; never follow symlinks.
            if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o750) } < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
            } else {
                created = true;
            }
            // SAFETY: as above; a raced symlink or non-directory is rejected.
            fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
        }
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: openat returned a fresh owned descriptor.
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
        if created {
            // SAFETY: fchown acts on the opened inode, never on a replaced path.
            if unsafe { libc::fchown(directory.as_raw_fd(), uid, gid) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    let name = path
        .file_name()
        .ok_or_else(|| AgentError::InvalidInput("Nama starter tidak valid".into()))?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| AgentError::InvalidInput("Nama starter mengandung NUL".into()))?;
    // SAFETY: descriptor and component are valid. O_EXCL prevents replacing user files.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | if exclusive { libc::O_EXCL } else { 0 }
                | libc::O_NONBLOCK
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Ok(None);
        }
        return Err(error.into());
    }
    // SAFETY: fd is a fresh owned descriptor returned by openat.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || (metadata.uid() != 0 && metadata.uid() != uid)
    {
        return Err(AgentError::SecurityViolation(
            "File runtime harus regular, tanpa hardlink dan milik tenant/root".into(),
        ));
    }
    // SAFETY: ownership changes apply to the validated, opened inode.
    if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    Ok(Some(file))
}

/// Walk each directory using owned descriptors. A tenant swapping a symlink
/// cannot redirect root mkdir/chmod/chown into another part of the host.
#[cfg(unix)]
#[allow(unsafe_code)]
fn ensure_owned_directory(path: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    };
    if !path.is_absolute() || path == Path::new("/") {
        return Err(AgentError::InvalidInput(
            "Direktori terkelola harus absolut dan bukan root".into(),
        ));
    }
    let mut directory = std::fs::File::open("/")?;
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            if component == std::path::Component::RootDir {
                continue;
            }
            return Err(AgentError::InvalidInput(
                "Komponen direktori tidak valid".into(),
            ));
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| AgentError::InvalidInput("Direktori mengandung NUL".into()))?;
        let mut created = false;
        // SAFETY: valid owned directory descriptor and terminated single component.
        let mut fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            // SAFETY: mkdirat does not follow the new component.
            if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o750) } < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
            } else {
                created = true;
            }
            // SAFETY: reject symlink substitution between mkdirat and openat.
            fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
        }
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: fd is a newly owned descriptor.
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
        // Newly created ancestors inherit the requested identity; existing
        // ancestors remain untouched. Leaf changes also use this exact inode.
        if created {
            // SAFETY: fchown operates on the open inode only.
            if unsafe { libc::fchown(directory.as_raw_fd(), uid, gid) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    let owner = directory.metadata()?.uid();
    if owner != 0 && owner != uid {
        return Err(AgentError::SecurityViolation(
            "Direktori dimiliki identity lain".into(),
        ));
    }
    // SAFETY: the final directory is pinned and cannot be substituted by a path race.
    if unsafe { libc::fchown(directory.as_raw_fd(), uid, gid) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    directory.set_permissions(std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

impl SystemRunner for RealSystemRunner {
    fn database_file_transfer(
        &self,
        request: &crate::database_transfer::DatabaseFileTransfer<'_>,
        credentials: &panel_proto::DatabaseWebCredentials,
    ) -> Result<(), AgentError> {
        crate::database_transfer::execute(request, credentials)
    }
    fn verify_origin_tls(&self, hostname: &str, certificate_pem: &str) -> Result<(), AgentError> {
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(hostname.to_owned())
            .map_err(|_| AgentError::InvalidInput("SNI origin tidak valid.".into()))?;
        let mut result = crate::panel_tls::verify_listener_name(443, certificate_pem, name.clone());
        for _ in 0..3 {
            if result.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
            result = crate::panel_tls::verify_listener_name(443, certificate_pem, name.clone());
        }
        result
    }
    fn verify_panel_tls(&self, port: u16, certificate_pem: &str) -> Result<(), AgentError> {
        let mut result = crate::panel_tls::verify_listener(port, certificate_pem);
        for _ in 0..3 {
            if result.is_ok() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
            result = crate::panel_tls::verify_listener(port, certificate_pem);
        }
        result
    }
    fn run_command_with_input(
        &self,
        binary: &str,
        args: &[&str],
        input: &[u8],
    ) -> Result<std::process::Output, AgentError> {
        use std::io::Write;
        use std::process::Stdio;
        let database_input = (binary == "debconf-set-selections" && args.is_empty())
            || (binary == "mariadb" && args == ["--batch"])
            || (binary == "runuser"
                && args
                    == [
                        "--user",
                        "postgres",
                        "--",
                        "psql",
                        "--no-psqlrc",
                        "--set",
                        "ON_ERROR_STOP=1",
                        "--dbname",
                        "postgres",
                    ]);
        let storage_input = binary == "python3" && args == [crate::storage_managed::HELPER];
        if (!database_input && !storage_input && !["sendmail", "doveadm"].contains(&binary))
            || input.len() > 2 * 1024 * 1024
        {
            return Err(AgentError::SecurityViolation(
                "Input command tidak valid".into(),
            ));
        }
        let mut child = panel_core::production::system_command(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AgentError::Internal("stdin tidak tersedia".into()))?;
        std::thread::scope(|scope| {
            let writer = scope.spawn(move || stdin.write_all(input));
            let output = child.wait_with_output()?;
            writer
                .join()
                .map_err(|_| AgentError::Internal("Penulisan input terhenti".into()))??;
            if !output.status.success() {
                return Err(AgentError::Internal(format!(
                    "{binary} menolak operasi; periksa log layanan"
                )));
            }
            Ok(output)
        })
    }
    fn run_command_streamed(
        &self,
        binary: &str,
        args: &[&str],
        on_output: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<std::process::Output, AgentError> {
        if ![
            "dnf",
            "rpm",
            "rpmkeys",
            "apt-get",
            "apt-cache",
            "add-apt-repository",
            "systemctl",
            "dpkg-query",
        ]
        .contains(&binary)
        {
            return Err(AgentError::SecurityViolation(
                "Command is not an installation command".into(),
            ));
        }
        let mut command = panel_core::production::system_command(binary);
        command
            .args(args)
            .env("LC_ALL", "C")
            .env("DEBIAN_FRONTEND", "noninteractive");
        crate::installation::run_streamed_command(command, binary, on_output)
    }
    fn run_cancellable_download(
        &self,
        args: &[&str],
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<std::process::Output, AgentError> {
        let mut command = panel_core::production::system_command("curl");
        command.args(args);
        run_cancellable_curl(command, cancel)
    }
    fn run_cancellable_python(
        &self,
        args: &[&str],
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<std::process::Output, AgentError> {
        let mut command = panel_core::production::system_command(crate::uv::UV_BIN);
        command.args(args);
        run_cancellable_installer(command, crate::uv::UV_BIN, cancel)
    }
    fn backup_file_stream(
        &self,
        binary: &str,
        args: &[&str],
        file: &Path,
        file_is_input: bool,
    ) -> Result<std::process::Output, AgentError> {
        use std::fs::OpenOptions;
        use std::process::Stdio;
        if !["runuser", "mariadb", "mariadb-dump", "mysqldump"].contains(&binary)
            || !file.starts_with("/var/lib/qonpanel-migration/legacy-dumps")
            || file
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err(AgentError::SecurityViolation(
                "Stream cadangan tidak valid".into(),
            ));
        }
        let mut command = panel_core::production::system_command(binary);
        command.args(args);
        if file_is_input {
            command
                .stdin(std::fs::File::open(file)?)
                .stdout(Stdio::null());
        } else {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            command.stdin(Stdio::null()).stdout(options.open(file)?);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(AgentError::CommandFailed {
                binary: binary.into(),
                code: output.status.code(),
                stderr: "Operasi stream cadangan gagal".into(),
            });
        }
        Ok(output)
    }
    fn run_command(&self, binary: &str, args: &[&str]) -> Result<std::process::Output, AgentError> {
        if !ALLOWED_BINARIES.contains(&binary) {
            return Err(AgentError::SecurityViolation(format!(
                "Binary '{}' is not in the allowed binaries list",
                binary
            )));
        }

        let mut command = panel_core::production::system_command(binary);
        // Candidate parsing must not depend on the server's display language.
        if binary == "apt-cache" {
            command.env("LC_ALL", "C");
        }
        if binary == "doveadm" {
            command.args(args);
            return crate::installation::run_streamed_command(command, binary, &|_| {}).map_err(
                |_| {
                    AgentError::Internal(
                        "Dovecot gagal memproses permintaan; periksa log layanan".into(),
                    )
                },
            );
        }
        let output = command.args(args).output().map_err(AgentError::Io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // Allow group already exists or process kill non-zero if no processes matched
            let is_ignorable = (binary == "groupadd" && stderr.contains("already exists"))
                || (binary == "groupdel" && stderr.contains("does not exist"))
                || (binary == "pkill" && output.status.code() == Some(1));

            if !is_ignorable {
                return Err(AgentError::CommandFailed {
                    binary: binary.to_string(),
                    code: output.status.code(),
                    stderr,
                });
            }
        }

        Ok(output)
    }

    fn create_dir_all(&self, path: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            ensure_owned_directory(path, mode, uid, gid)
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(path)?;
            self.set_permissions(path, mode)?;
            self.set_ownership(path, uid, gid)?;
            Ok(())
        }
    }

    fn set_permissions(&self, path: &Path, mode: u32) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
                if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(1) {
                    warn!(
                        "set_permissions on {} to {:o} failed ({}: Operation not permitted). Continuing without chmod.",
                        path.display(),
                        mode,
                        e
                    );
                } else {
                    return Err(AgentError::Io(e));
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
        }
        Ok(())
    }

    fn set_ownership(&self, path: &Path, uid: u32, gid: u32) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::chown;
            if let Err(e) = chown(path, Some(uid), Some(gid)) {
                if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(1) {
                    warn!(
                        "set_ownership on {} to {}:{} failed ({}: Operation not permitted). Continuing without chown.",
                        path.display(),
                        uid,
                        gid,
                        e
                    );
                } else {
                    return Err(AgentError::Io(e));
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (path, uid, gid);
        }
        Ok(())
    }

    fn set_facl(&self, path: &Path, user: &str, permissions: &str) -> Result<(), AgentError> {
        let path_str = path.to_str().ok_or_else(|| {
            AgentError::InvalidInput(format!("Path contains invalid UTF-8: {:?}", path))
        })?;
        let acl_spec = format!("u:{user}:{permissions}");
        match self.run_command("setfacl", &["-m", &acl_spec, path_str]) {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!(
                    "setfacl failed on {}: {}. Continuing without ACL.",
                    path_str, e
                );
                Ok(())
            }
        }
    }

    fn move_path(&self, from: &Path, to: &Path) -> Result<(), AgentError> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn create_symlink(&self, original: &Path, link: &Path) -> Result<(), AgentError> {
        if let Some(parent) = link.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
        }
        if link.is_symlink() || link.exists() {
            let _ = std::fs::remove_file(link);
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).map_err(AgentError::Io)?;
        }
        #[cfg(windows)]
        {
            if original.is_dir() {
                std::os::windows::fs::symlink_dir(original, link).map_err(AgentError::Io)?;
            } else {
                std::os::windows::fs::symlink_file(original, link).map_err(AgentError::Io)?;
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (original, link);
        }
        Ok(())
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, AgentError> {
        std::fs::read_link(path).map_err(AgentError::Io)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), AgentError> {
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn write_file(&self, path: &Path, content: &[u8], mode: u32) -> Result<(), AgentError> {
        if let Some(parent) = path.parent().filter(|p| !p.exists()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        self.set_permissions(path, mode)?;
        Ok(())
    }

    fn write_file_if_absent(
        &self,
        path: &Path,
        content: &[u8],
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<bool, AgentError> {
        use std::io::Write;
        #[cfg(unix)]
        let mut file = match open_owned_runtime_file(path, mode, uid, gid, true)? {
            Some(file) => file,
            None => return Ok(false),
        };
        #[cfg(not(unix))]
        let mut file = {
            let _ = (mode, uid, gid);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            }
        };
        file.write_all(content)?;
        Ok(true)
    }

    fn prepare_owned_file(
        &self,
        path: &Path,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<(), AgentError> {
        #[cfg(unix)]
        let _ = open_owned_runtime_file(path, mode, uid, gid, false)?;
        #[cfg(not(unix))]
        {
            let _ = (mode, uid, gid);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
        }
        Ok(())
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>, AgentError> {
        std::fs::read(path).map_err(AgentError::Io)
    }

    fn remove_file(&self, path: &Path) -> Result<(), AgentError> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, AgentError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(path)
            .map_err(AgentError::Io)?
            .filter_map(|e| e.ok().map(|ent| ent.path()))
            .collect();
        Ok(entries)
    }
}

/// Record of an action executed on the mock runner for testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockAction {
    DatabaseTransfer {
        name: String,
        path: String,
        uid: u32,
        import: bool,
        database_user: String,
    },
    Command {
        binary: String,
        args: Vec<String>,
    },
    CreateDir {
        path: PathBuf,
        mode: u32,
        uid: u32,
        gid: u32,
    },
    SetPermissions {
        path: PathBuf,
        mode: u32,
    },
    SetOwnership {
        path: PathBuf,
        uid: u32,
        gid: u32,
    },
    SetFacl {
        path: PathBuf,
        user: String,
        permissions: String,
    },
    MovePath {
        from: PathBuf,
        to: PathBuf,
    },
    CreateSymlink {
        original: PathBuf,
        link: PathBuf,
    },
    RemoveDir {
        path: PathBuf,
    },
    WriteFile {
        path: PathBuf,
        content: Vec<u8>,
        mode: u32,
    },
    RemoveFile {
        path: PathBuf,
    },
}

pub type MockCommandResponse = (i32, Vec<u8>, Vec<u8>);

/// Helper to determine if a path refers to a host system directory that should
/// never leak into test mocks.
pub fn is_mock_system_path(path: &Path) -> bool {
    let p = path.to_string_lossy();
    p == "/.dockerenv"
        || p == "/.containerenv"
        || p.starts_with("/home")
        || p.starts_with("/etc")
        || p.starts_with("/usr")
        || p.starts_with("/var")
        || p.starts_with("/run")
        || p.starts_with("/sys")
        || p.starts_with("/proc")
        || p.starts_with("/bin")
        || p.starts_with("/sbin")
        || p.starts_with("/dev")
}

/// In-memory mock runner for test assertions and safe non-root validation.
#[derive(Debug, Clone, Default)]
pub struct MockSystemRunner {
    pub actions: Arc<Mutex<Vec<MockAction>>>,
    pub responses: Arc<Mutex<std::collections::HashMap<String, MockCommandResponse>>>,
    pub files: Arc<Mutex<std::collections::HashMap<PathBuf, Vec<u8>>>>,
    pub symlinks: Arc<Mutex<std::collections::HashMap<PathBuf, PathBuf>>>,
    pub spawn_errors: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl MockSystemRunner {
    pub fn new() -> Self {
        Self {
            actions: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(std::collections::HashMap::new())),
            files: Arc::new(Mutex::new(std::collections::HashMap::new())),
            symlinks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            spawn_errors: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Simulate failure to start an isolation helper without executing anything.
    pub fn set_spawn_error(&self, binary: &str) {
        if let Ok(mut errors) = self.spawn_errors.lock() {
            errors.insert(binary.to_owned());
        }
    }

    pub fn set_file_content(&self, path: impl AsRef<Path>, content: &[u8]) {
        if let Ok(mut lock) = self.files.lock() {
            lock.insert(path.as_ref().to_path_buf(), content.to_vec());
        }
    }

    pub fn set_response(&self, key: &str, exit_code: i32, stdout: &[u8], stderr: &[u8]) {
        if let Ok(mut lock) = self.responses.lock() {
            lock.insert(
                key.to_string(),
                (exit_code, stdout.to_vec(), stderr.to_vec()),
            );
        }
    }

    pub fn recorded_actions(&self) -> Vec<MockAction> {
        self.actions.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn get_commands(&self) -> Vec<String> {
        self.recorded_actions()
            .into_iter()
            .filter_map(|action| match action {
                MockAction::Command { binary, args } => {
                    Some(format!("{} {}", binary, args.join(" ")))
                }
                _ => None,
            })
            .collect()
    }

    pub fn add_existing_path(&self, path: impl AsRef<Path>) {
        self.set_file_content(path, b"");
    }
}

impl SystemRunner for MockSystemRunner {
    fn database_file_transfer(
        &self,
        request: &crate::database_transfer::DatabaseFileTransfer<'_>,
        credentials: &panel_proto::DatabaseWebCredentials,
    ) -> Result<(), AgentError> {
        self.actions
            .lock()
            .map_err(|_| AgentError::Internal("Fixture lock".into()))?
            .push(MockAction::DatabaseTransfer {
                name: request.name.into(),
                path: request.path.into(),
                uid: request.tenant_uid,
                import: request.import,
                database_user: credentials.username.clone(),
            });
        Ok(())
    }
    fn operating_system(&self) -> Result<panel_core::platform::LinuxPlatform, AgentError> {
        // Explicit default for test fixtures only. Real hosts must have valid metadata.
        if !self.path_exists(Path::new("/etc/os-release")) {
            return Ok(panel_core::platform::LinuxPlatform::Ubuntu2404);
        }
        let bytes = self.read_file(Path::new("/etc/os-release"))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| AgentError::InvalidInput("Fixture OS bukan UTF-8.".into()))?;
        panel_core::platform::LinuxPlatform::parse_os_release(text)
            .map_err(|message| AgentError::InvalidInput(message.into()))
    }
    fn verify_origin_tls(&self, _hostname: &str, _certificate_pem: &str) -> Result<(), AgentError> {
        if self.run_command("nginx", &["-t"])?.status.success() {
            Ok(())
        } else {
            Err(AgentError::Internal("Fixture origin TLS failed.".into()))
        }
    }
    fn verify_panel_tls(&self, _port: u16, _certificate_pem: &str) -> Result<(), AgentError> {
        if self
            .responses
            .lock()
            .ok()
            .and_then(|responses| responses.get("panel tls verify").cloned())
            .is_some_and(|(code, _, _)| code != 0)
        {
            return Err(AgentError::Internal(
                "Fixture listener verification failed".into(),
            ));
        }
        Ok(())
    }
    fn run_command_with_input(
        &self,
        binary: &str,
        args: &[&str],
        _input: &[u8],
    ) -> Result<std::process::Output, AgentError> {
        // Never execute commands or persist message contents in mock diagnostics.
        self.run_command(binary, args)
    }
    fn run_command(&self, binary: &str, args: &[&str]) -> Result<std::process::Output, AgentError> {
        if !ALLOWED_BINARIES.contains(&binary) {
            return Err(AgentError::SecurityViolation(format!(
                "Binary '{}' is not in the allowed binaries list",
                binary
            )));
        }

        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::Command {
                binary: binary.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            });
        }

        if self
            .spawn_errors
            .lock()
            .is_ok_and(|errors| errors.contains(binary))
        {
            return Err(AgentError::Io(std::io::Error::other(
                "Fixture spawn failure",
            )));
        }
        let cmd_key = format!("{binary} {}", args.join(" "));
        let (exit_code, stdout, stderr) = if let Ok(lock) = self.responses.lock() {
            if let Some((code, out, err)) = lock.get(&cmd_key).or_else(|| lock.get(binary)) {
                (*code, out.clone(), err.clone())
            } else {
                (0, Vec::new(), Vec::new())
            }
        } else {
            (0, Vec::new(), Vec::new())
        };

        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(exit_code)
        };
        #[cfg(not(unix))]
        let status = {
            std::process::Command::new("cmd")
                .args(["/c", &format!("exit {exit_code}")])
                .status()
                .unwrap_or_else(|_| panic!("Failed to get dummy ExitStatus"))
        };

        if !status.success() {
            let stderr_str = String::from_utf8_lossy(&stderr).to_string();
            let is_ignorable = (binary == "groupadd" && stderr_str.contains("already exists"))
                || (binary == "groupdel" && stderr_str.contains("does not exist"))
                || (binary == "pkill" && status.code() == Some(1));

            if !is_ignorable {
                return Err(AgentError::CommandFailed {
                    binary: binary.to_string(),
                    code: status.code(),
                    stderr: stderr_str,
                });
            }
        }

        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }

    fn create_dir_all(&self, path: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), AgentError> {
        if let Ok(mut files) = self.files.lock() {
            files.insert(path.to_path_buf(), Vec::new());
        }
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::CreateDir {
                path: path.to_path_buf(),
                mode,
                uid,
                gid,
            });
        }
        Ok(())
    }

    fn set_permissions(&self, path: &Path, mode: u32) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::SetPermissions {
                path: path.to_path_buf(),
                mode,
            });
        }
        Ok(())
    }

    fn set_ownership(&self, path: &Path, uid: u32, gid: u32) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::SetOwnership {
                path: path.to_path_buf(),
                uid,
                gid,
            });
        }
        Ok(())
    }

    fn set_facl(&self, path: &Path, user: &str, permissions: &str) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::SetFacl {
                path: path.to_path_buf(),
                user: user.to_string(),
                permissions: permissions.to_string(),
            });
        }
        Ok(())
    }

    fn move_path(&self, from: &Path, to: &Path) -> Result<(), AgentError> {
        if let Ok(mut files) = self.files.lock() {
            let content = files.remove(from);
            if let Some(content) = content {
                files.insert(to.to_path_buf(), content);
            }
        }
        if let Ok(mut symlinks) = self.symlinks.lock() {
            let target = symlinks.remove(from);
            if let Some(target) = target {
                symlinks.insert(to.to_path_buf(), target);
            }
        }
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::MovePath {
                from: from.to_path_buf(),
                to: to.to_path_buf(),
            });
        }
        Ok(())
    }

    fn create_symlink(&self, original: &Path, link: &Path) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.symlinks.lock() {
            lock.insert(link.to_path_buf(), original.to_path_buf());
        }
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::CreateSymlink {
                original: original.to_path_buf(),
                link: link.to_path_buf(),
            });
        }
        Ok(())
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, AgentError> {
        if let Ok(lock) = self.symlinks.lock()
            && let Some(target) = lock.get(path)
        {
            return Ok(target.clone());
        }
        if path.is_symlink() {
            std::fs::read_link(path).map_err(AgentError::Io)
        } else {
            Err(AgentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Mock symlink not found: {:?}", path),
            )))
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::RemoveDir {
                path: path.to_path_buf(),
            });
        }
        if !is_mock_system_path(path) && path.exists() {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(())
    }

    fn path_exists(&self, path: &Path) -> bool {
        if self
            .files
            .lock()
            .ok()
            .is_some_and(|lock| lock.contains_key(path))
        {
            return true;
        }
        if self
            .symlinks
            .lock()
            .ok()
            .is_some_and(|lock| lock.contains_key(path))
        {
            return true;
        }
        if is_mock_system_path(path) {
            return false;
        }
        path.exists()
    }

    fn write_file(&self, path: &Path, content: &[u8], mode: u32) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.files.lock() {
            lock.insert(path.to_path_buf(), content.to_vec());
        }
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::WriteFile {
                path: path.to_path_buf(),
                content: content.to_vec(),
                mode,
            });
        }
        Ok(())
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>, AgentError> {
        if let Some(content) = self
            .files
            .lock()
            .ok()
            .and_then(|lock| lock.get(path).cloned())
        {
            return Ok(content);
        }
        if !is_mock_system_path(path) && path.exists() {
            std::fs::read(path).map_err(AgentError::Io)
        } else {
            Err(AgentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Mock file not found: {:?}", path),
            )))
        }
    }

    fn remove_file(&self, path: &Path) -> Result<(), AgentError> {
        if let Ok(mut lock) = self.files.lock() {
            lock.remove(path);
        }
        if let Ok(mut lock) = self.symlinks.lock() {
            lock.remove(path);
        }
        if let Ok(mut lock) = self.actions.lock() {
            lock.push(MockAction::RemoveFile {
                path: path.to_path_buf(),
            });
        }
        if !is_mock_system_path(path) && path.exists() {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, AgentError> {
        let mut results = Vec::new();
        if let Ok(files) = self.files.lock() {
            for key in files.keys() {
                if key.parent() == Some(path) {
                    results.push(key.clone());
                }
            }
        }
        if !is_mock_system_path(path)
            && let Ok(entries) = std::fs::read_dir(path)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if !results.contains(&p) {
                    results.push(p);
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn runtime_parent_access_is_search_only_without_changing_ipc_permissions() {
        let runner = MockSystemRunner::new();
        ensure_runtime_parent_access(&runner, "p_alice").unwrap();
        let actions = runner.recorded_actions();
        let [MockAction::Command { binary, args }] = actions.as_slice() else {
            panic!("Only a non-recursive parent ACL may be changed: {actions:?}");
        };
        assert_eq!(binary, "setfacl");
        assert_eq!(
            args,
            &["-m", "u:p_alice:--x,u:www-data:--x", "--", "/run/panel"]
        );
        // Named entries allow path traversal but cannot list or mutate the parent.
        for entry in args[1].split(',') {
            let permissions = entry.rsplit(':').next().unwrap();
            assert_eq!(permissions, "--x");
            assert!(!permissions.contains(['r', 'w']));
        }
        // No chmod/chown, inherited ACL, recursive change, group membership change,
        // or permission change on the separately protected 0660 IPC socket.
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn runtime_parent_access_rejects_invalid_or_reserved_identity_before_commands() {
        for username in [
            "root",
            "www-data",
            "panel-api",
            "p_alice,u:root:rwx",
            "p_alice\n",
        ] {
            let runner = MockSystemRunner::new();
            assert!(ensure_runtime_parent_access(&runner, username).is_err());
            assert!(runner.recorded_actions().is_empty());
        }
    }

    #[test]
    #[allow(unsafe_code)] // Read the fixture process identity on Unix.
    fn exclusive_starter_creation_preserves_existing_and_concurrent_project_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.py");
        #[cfg(unix)]
        // SAFETY: these calls only read the test process identity.
        let identity = unsafe { (libc::getuid(), libc::getgid()) };
        #[cfg(not(unix))]
        let identity = (0, 0);
        let handles: Vec<_> = [b"first".to_vec(), b"second".to_vec()]
            .into_iter()
            .map(|content| {
                let path = path.clone();
                std::thread::spawn(move || {
                    RealSystemRunner
                        .write_file_if_absent(&path, &content, 0o644, identity.0, identity.1)
                        .unwrap()
                })
            })
            .collect();
        let created = handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>();
        assert_eq!(created, 1);
        let before = std::fs::read(&path).unwrap();
        assert!(
            !RealSystemRunner
                .write_file_if_absent(&path, b"replacement", 0o644, identity.0, identity.1)
                .unwrap()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        RealSystemRunner
            .prepare_owned_file(&path, 0o640, identity.0, identity.1)
            .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "log repair must never truncate content"
        );
    }

    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)] // Read the fixture process identity without changing it.
    fn owned_runtime_files_reject_symlink_parents_and_linked_log_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("alias")).unwrap();
        // SAFETY: these calls only query this test process's identity.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        assert!(
            RealSystemRunner
                .write_file_if_absent(
                    &root.path().join("alias/app.py"),
                    b"starter",
                    0o644,
                    uid,
                    gid
                )
                .is_err()
        );
        assert!(!outside.path().join("app.py").exists());
        let original = root.path().join("original");
        let linked = root.path().join("linked");
        std::fs::write(&original, b"preserve").unwrap();
        std::fs::hard_link(&original, &linked).unwrap();
        assert!(
            RealSystemRunner
                .prepare_owned_file(&linked, 0o640, uid, gid)
                .is_err()
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"preserve");
    }

    #[cfg(unix)]
    #[test]
    fn managed_directories_never_chmod_or_chown_a_symlink_target() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let before = outside.path().metadata().unwrap();
        symlink(outside.path(), root.path().join("alias")).unwrap();
        let uid = root.path().metadata().unwrap().uid();
        let gid = root.path().metadata().unwrap().gid();
        for path in [root.path().join("alias"), root.path().join("alias/nested")] {
            assert!(
                RealSystemRunner
                    .create_dir_all(&path, 0o777, uid, gid)
                    .is_err()
            );
        }
        let after = outside.path().metadata().unwrap();
        assert_eq!(before.permissions().mode(), after.permissions().mode());
        assert_eq!(before.uid(), after.uid());
        assert!(!outside.path().join("nested").exists());
        let leaf = root.path().join("safe/leaf");
        RealSystemRunner
            .create_dir_all(&leaf, 0o700, uid, gid)
            .unwrap();
        assert_eq!(leaf.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    }

    // This is a child test fixture, never the panel-agent daemon or a system command.
    #[test]
    fn download_child_fixture() {
        if let Ok(marker) = std::env::var("PANEL_DOWNLOAD_CHILD_MARKER") {
            std::fs::write(marker, b"started").unwrap();
            eprintln!("fixture download started");
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
    }

    #[test]
    fn cancelling_download_kills_and_reaps_child_without_waiting_for_completion() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("child-started");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "system::download_tests::download_child_fixture",
                "--nocapture",
            ])
            .env("PANEL_DOWNLOAD_CHILD_MARKER", &marker);
        let cancel = CancellationToken::new();
        let child_cancel = cancel.clone();
        let handle = std::thread::spawn(move || run_cancellable_curl(command, &child_cancel));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let started = marker.exists();
        let stop = std::time::Instant::now();
        cancel.cancel();
        let result = handle.join().unwrap();
        assert!(started, "child fixture must have started");
        assert!(result.unwrap_err().to_string().contains("dibatalkan"));
        assert!(stop.elapsed() < std::time::Duration::from_secs(3));
    }
}
