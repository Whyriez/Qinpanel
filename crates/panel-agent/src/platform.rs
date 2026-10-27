//! Closed OS adapters used by typed operations, never exposed as an RPC command.
use crate::{AgentError, system::SystemRunner};
use panel_core::platform::LinuxPlatform;
use panel_runtime::php::{PhpExtensionDefinition, PhpVersion};
use std::{path::Path, process::Output};

pub fn install(runner: &dyn SystemRunner, packages: &[&str]) -> Result<Output, AgentError> {
    let rpm = runner.operating_system()?.is_rpm();
    let mapped = packages
        .iter()
        .map(|name| package(rpm, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut args = if rpm {
        vec![
            "-y",
            "--setopt=keepcache=True",
            "--setopt=localpkg_gpgcheck=True",
            "install",
            "--",
        ]
    } else {
        vec!["install", "-y", "--no-install-recommends"]
    };
    args.extend(mapped.iter().map(String::as_str));
    runner.run_command(if rpm { "dnf" } else { "apt-get" }, &args)
}

pub fn package(rpm: bool, name: &str) -> Result<String, AgentError> {
    if !rpm {
        return Ok(name.into());
    }
    let mapped = match name {
        "mariadb-client" => "mariadb",
        "postgresql" => "postgresql-server",
        "dovecot-core" | "dovecot-imapd" | "dovecot-lmtpd" => "dovecot",
        "opendkim-tools" => "opendkim",
        "phpmyadmin" => "phpMyAdmin",
        "php-mysql" => "php-mysqlnd",
        "php-curl" => "php-common",
        "php-cgi" => "php-cli",
        "php-zip" => "php-pecl-zip",
        _ if name.starts_with("php") && name.as_bytes().get(3).is_some_and(u8::is_ascii_digit) => {
            let (version, suffix) = name[3..]
                .split_once('-')
                .ok_or_else(|| AgentError::InvalidInput("Paket PHP tidak valid".into()))?;
            let version =
                PhpVersion::parse(version).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
            let suffix = match suffix {
                "curl" | "bz2" | "sqlite3" => "common",
                "readline" => "cli",
                "mysql" => "mysqlnd",
                "zip" => "pecl-zip",
                "redis" => "pecl-redis6",
                "imagick" => "pecl-imagick-im7",
                other => other,
            };
            return Ok(format!(
                "php{}-php-{suffix}",
                version.as_str().replace('.', "")
            ));
        }
        _ => name,
    };
    Ok(mapped.into())
}

pub fn remove(runner: &dyn SystemRunner, packages: &[&str]) -> Result<Output, AgentError> {
    let rpm = runner.operating_system()?.is_rpm();
    let mapped = packages
        .iter()
        .map(|name| package(rpm, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut args = if rpm {
        vec![
            "-y",
            "--setopt=clean_requirements_on_remove=False",
            "remove",
            "--",
        ]
    } else {
        vec!["remove", "-y", "--purge"]
    };
    args.extend(mapped.iter().map(String::as_str));
    runner.run_command(if rpm { "dnf" } else { "apt-get" }, &args)
}

pub fn setup_remi(runner: &dyn SystemRunner) -> Result<(), AgentError> {
    let os = runner.operating_system()?;
    let major = os.release_major();
    let key_url = match os {
        LinuxPlatform::AlmaLinux9 => "https://rpms.remirepo.net/RPM-GPG-KEY-remi2021",
        LinuxPlatform::AlmaLinux10 => "https://rpms.remirepo.net/RPM-GPG-KEY-remi2024",
        _ => {
            return Err(AgentError::InvalidInput(
                "Repositori Remi hanya untuk AlmaLinux".into(),
            ));
        }
    };
    let key = runner.run_command(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-time",
            "60",
            "--max-filesize",
            "65536",
            key_url,
        ],
    )?;
    if !(100..=65536).contains(&key.stdout.len()) {
        return Err(AgentError::InvalidInput("Kunci Remi tidak valid".into()));
    }
    runner.write_file(
        Path::new("/etc/pki/rpm-gpg/QONPANEL-REMI"),
        &key.stdout,
        0o644,
    )?;
    runner.run_command("rpm", &["--import", "/etc/pki/rpm-gpg/QONPANEL-REMI"])?;
    let content = format!(
        "[qonpanel-remi-safe]\nname=Remi parallel PHP for Enterprise Linux {major}\nbaseurl=https://rpms.remirepo.net/enterprise/{major}/safe/$basearch/\nenabled=1\ngpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/QONPANEL-REMI\nsslverify=1\n"
    );
    runner.write_file(
        Path::new("/etc/yum.repos.d/qonpanel-remi-safe.repo"),
        content.as_bytes(),
        0o644,
    )?;
    runner.run_command("dnf", &["--refresh", "makecache"])?;
    Ok(())
}

pub fn php_service(platform: LinuxPlatform, version: PhpVersion) -> String {
    if platform.is_rpm() {
        format!("php{}-php-fpm.service", version.as_str().replace('.', ""))
    } else {
        version.fpm_service_name()
    }
}

pub fn php_builtin(
    runner: &dyn SystemRunner,
    version: PhpVersion,
    def: &PhpExtensionDefinition,
) -> Result<bool, AgentError> {
    if def.is_builtin(version) {
        return Ok(true);
    }
    if def.name != "readline" || !runner.operating_system()?.is_rpm() {
        return Ok(false);
    }
    // -n skips all system/tenant INI files; --ri inspects one fixed module and
    // does not evaluate PHP code. This distinguishes a compiled-in CLI module
    // from a shared extension instead of guessing from the package name.
    let binary = version.cli_binary_path();
    Ok(runner
        .run_command(&binary.to_string_lossy(), &["-n", "--ri", "readline"])
        .is_ok_and(|out| {
            out.status.success()
                && String::from_utf8_lossy(&out.stdout).contains("Readline Support => enabled")
        }))
}

/// Keep the versioned paths already persisted in tenant units/configuration.
/// Refuse to replace a custom file; aliases are exact, root-owned, and repeatable.
pub fn php_aliases(runner: &dyn SystemRunner, version: PhpVersion) -> Result<(), AgentError> {
    let prefix = format!("php{}", version.as_str().replace('.', ""));
    let root = format!("/opt/remi/{prefix}/root/usr");
    let config = format!("/etc/opt/remi/{prefix}");
    runner.create_dir_all(&version.fpm_config_dir(), 0o755, 0, 0)?;
    runner.create_dir_all(&version.cli_config_dir(), 0o755, 0, 0)?;
    for (target, link) in [
        (format!("{root}/sbin/php-fpm"), version.fpm_binary_path()),
        (format!("{root}/bin/php"), version.cli_binary_path()),
        (
            format!("{config}/php.ini"),
            version.fpm_config_dir().join("php.ini"),
        ),
        (
            format!("{config}/php.ini"),
            version.cli_config_dir().join("php.ini"),
        ),
        (
            format!("{config}/php.d"),
            version.fpm_config_dir().join("conf.d"),
        ),
        (
            format!("{config}/php.d"),
            version.cli_config_dir().join("conf.d"),
        ),
    ] {
        if !runner.path_exists(Path::new(&target)) {
            return Err(AgentError::InvalidInput(format!(
                "Paket Remi belum lengkap: {target}"
            )));
        }
        alias(runner, Path::new(&target), &link)?;
    }
    Ok(())
}

pub fn alias(runner: &dyn SystemRunner, target: &Path, link: &Path) -> Result<(), AgentError> {
    if let Ok(existing) = runner.read_link(link) {
        if existing == target {
            return Ok(());
        }
        return Err(AgentError::InvalidInput(format!(
            "Alias custom dipertahankan: {}",
            link.display()
        )));
    }
    if runner.path_exists(link) {
        return Err(AgentError::InvalidInput(format!(
            "File custom dipertahankan: {}",
            link.display()
        )));
    }
    runner.create_symlink(target, link)
}

pub fn relabel(runner: &dyn SystemRunner, path: &Path) -> Result<(), AgentError> {
    if runner.operating_system()?.is_rpm() {
        runner.run_command("restorecon", &["-RF", "--", &path.to_string_lossy()])?;
    }
    Ok(())
}

/// RPM configuration uses one native php.d for CLI and tenant FPM. Move only
/// files belonging to the curated extension, retaining package-owned filenames.
pub fn toggle_php(
    runner: &dyn SystemRunner,
    version: PhpVersion,
    def: &PhpExtensionDefinition,
    enabled: bool,
) -> Result<(), AgentError> {
    let live = format!(
        "/etc/opt/remi/php{}/php.d",
        version.as_str().replace('.', "")
    );
    let disabled = format!("/etc/panel/php-disabled/{}", version.as_str());
    runner.create_dir_all(Path::new(&disabled), 0o755, 0, 0)?;
    let (source, destination) = if enabled {
        (&disabled, &live)
    } else {
        (&live, &disabled)
    };
    let mut planned = Vec::new();
    for entry in runner.read_dir(Path::new(source))? {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let stem = name.strip_suffix(".ini").unwrap_or("");
        let module = stem.split_once('-').map_or(stem, |(_, value)| value);
        if def.ini_names.contains(&module) {
            let target = Path::new(destination).join(name);
            if runner.path_exists(&target) {
                return Err(AgentError::InvalidInput(
                    "Konfigurasi ekstensi duplikat; file dipertahankan".into(),
                ));
            }
            planned.push((entry, target));
        }
    }
    for (index, (source, target)) in planned.iter().enumerate() {
        if let Err(error) = runner.move_path(source, target) {
            for (previous_source, previous_target) in planned[..index].iter().rev() {
                runner
                    .move_path(previous_target, previous_source)
                    .map_err(|_| {
                        AgentError::Internal(
                            "Perubahan ekstensi gagal dan rollback perlu diperiksa".into(),
                        )
                    })?;
            }
            return Err(error);
        }
    }
    if planned.is_empty()
        && !runner
            .read_dir(Path::new(destination))?
            .iter()
            .any(|entry| {
                entry
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let stem = name.strip_suffix(".ini").unwrap_or("");
                        def.ini_names
                            .contains(&stem.split_once('-').map_or(stem, |(_, value)| value))
                    })
            })
    {
        return Err(AgentError::InvalidInput(
            "Konfigurasi ekstensi tidak ditemukan; tidak ada perubahan yang diterapkan".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    fn fixture(release: &str) -> MockSystemRunner {
        let runner = MockSystemRunner::new();
        runner.set_file_content("/etc/os-release", release.as_bytes());
        runner
    }
    #[test]
    fn installing_on_alma_never_uses_apt_and_unknown_os_cannot_mutate() {
        for major in ["9.7", "10.2"] {
            let runner = fixture(&format!("ID=almalinux\nVERSION_ID={major}\n"));
            install(&runner, &["mariadb-server", "mariadb-client"]).unwrap();
            let commands = runner.get_commands();
            assert!(
                commands
                    .iter()
                    .any(|value| value.contains("dnf -y")
                        && value.ends_with("mariadb-server mariadb"))
            );
            assert!(!commands.iter().any(|value| value.contains("apt")));
        }
        let unknown = fixture("ID=rocky\nVERSION_ID=9\n");
        assert!(install(&unknown, &["mariadb-server"]).is_err());
        assert!(unknown.get_commands().is_empty());
    }
    #[test]
    fn php_alias_migration_is_repeatable_and_preserves_operator_files() {
        let runner = fixture("ID=almalinux\nVERSION_ID=9\n");
        for path in [
            "/opt/remi/php83/root/usr/sbin/php-fpm",
            "/opt/remi/php83/root/usr/bin/php",
            "/etc/opt/remi/php83/php.ini",
            "/etc/opt/remi/php83/php.d",
        ] {
            runner.add_existing_path(path);
        }
        php_aliases(&runner, PhpVersion::Php83).unwrap();
        php_aliases(&runner, PhpVersion::Php83).unwrap();
        assert_eq!(
            runner.read_link(Path::new("/usr/sbin/php-fpm8.3")).unwrap(),
            Path::new("/opt/remi/php83/root/usr/sbin/php-fpm")
        );
        let custom = fixture("ID=almalinux\nVERSION_ID=10\n");
        custom.add_existing_path("/opt/remi/php83/root/usr/sbin/php-fpm");
        custom.set_file_content("/usr/sbin/php-fpm8.3", b"operator binary");
        assert!(php_aliases(&custom, PhpVersion::Php83).is_err());
        assert_eq!(
            custom.read_file(Path::new("/usr/sbin/php-fpm8.3")).unwrap(),
            b"operator binary"
        );
    }
    #[test]
    fn toggling_a_remi_extension_preserves_unrelated_ini_and_is_reversible() {
        let runner = fixture("ID=almalinux\nVERSION_ID=10\n");
        runner.set_file_content(
            "/etc/opt/remi/php83/php.d/40-redis.ini",
            b"extension=redis\n",
        );
        runner.set_file_content("/etc/opt/remi/php83/php.d/20-curl.ini", b"extension=curl\n");
        let extension = panel_runtime::php::find_curated_extension("redis").unwrap();
        toggle_php(&runner, PhpVersion::Php83, extension, false).unwrap();
        assert!(!runner.path_exists(Path::new("/etc/opt/remi/php83/php.d/40-redis.ini")));
        assert!(runner.path_exists(Path::new("/etc/opt/remi/php83/php.d/20-curl.ini")));
        toggle_php(&runner, PhpVersion::Php83, extension, true).unwrap();
        assert_eq!(
            runner
                .read_file(Path::new("/etc/opt/remi/php83/php.d/40-redis.ini"))
                .unwrap(),
            b"extension=redis\n"
        );
    }
    #[test]
    fn rpm_names_preserve_parallel_php_and_native_services() {
        assert_eq!(package(true, "php8.3-fpm").unwrap(), "php83-php-fpm");
        assert_eq!(package(true, "php7.4-mysql").unwrap(), "php74-php-mysqlnd");
        assert_eq!(package(true, "postgresql").unwrap(), "postgresql-server");
        assert_eq!(
            php_service(LinuxPlatform::AlmaLinux10, PhpVersion::Php85),
            "php85-php-fpm.service"
        );
        assert!(package(true, "php9.9-fpm").is_err());
        assert_eq!(package(true, "php8.3-readline").unwrap(), "php83-php-cli");
    }

    #[test]
    fn builtin_cli_readline_is_read_only_and_cannot_remove_the_cli_package() {
        let runner = fixture("ID=almalinux\nVERSION_ID=10\n");
        runner.add_existing_path("/usr/bin/php8.3");
        runner.set_response("/usr/bin/php8.3", 0, b"Readline Support => enabled\n", b"");
        let list = crate::php::list_php_extensions(&runner, "8.3").unwrap();
        let readline = list
            .extensions
            .iter()
            .find(|item| item.name == "readline")
            .unwrap();
        assert!(readline.is_base && readline.installed && readline.enabled);
        assert_eq!(readline.package_name, "php83-php-cli");
        assert!(crate::php::toggle_php_extension(&runner, "8.3", "readline", false).is_err());
        assert!(crate::php::remove_php_extension(&runner, "8.3", "readline").is_err());
        assert!(
            !runner
                .get_commands()
                .iter()
                .any(|command| command.starts_with("dnf"))
        );
    }

    #[test]
    fn bundled_extension_conflict_preserves_every_ini_before_mutation() {
        let runner = fixture("ID=almalinux\nVERSION_ID=9\n");
        let first = "/etc/opt/remi/php83/php.d/20-pgsql.ini";
        let second = "/etc/opt/remi/php83/php.d/30-pdo_pgsql.ini";
        runner.set_file_content(first, b"extension=pgsql\n");
        runner.set_file_content(second, b"extension=pdo_pgsql\n");
        runner.set_file_content(
            "/etc/panel/php-disabled/8.3/30-pdo_pgsql.ini",
            b"operator configuration\n",
        );
        let extension = panel_runtime::php::find_curated_extension("pgsql").unwrap();
        assert!(toggle_php(&runner, PhpVersion::Php83, extension, false).is_err());
        assert_eq!(
            runner.read_file(Path::new(first)).unwrap(),
            b"extension=pgsql\n"
        );
        assert_eq!(
            runner.read_file(Path::new(second)).unwrap(),
            b"extension=pdo_pgsql\n"
        );
    }
    #[test]
    fn alma_php_install_uses_parallel_packages_native_mask_and_legacy_paths() {
        for major in ["9", "10"] {
            let runner = fixture(&format!("ID=almalinux\nVERSION_ID={major}\n"));
            runner.set_response("curl", 0, &[b'k'; 128], b"");
            runner.set_response(
                "systemctl show --property=UnitFileState --value php83-php-fpm.service",
                0,
                b"masked\n",
                b"",
            );
            for path in [
                "/opt/remi/php83/root/usr/sbin/php-fpm",
                "/opt/remi/php83/root/usr/bin/php",
                "/etc/opt/remi/php83/php.ini",
                "/etc/opt/remi/php83/php.d",
            ] {
                runner.add_existing_path(path);
            }
            let response = crate::php::install_php_runtime(&runner, "8.3").unwrap();
            assert!(response.success && response.masked);
            let commands = runner.get_commands();
            assert!(
                commands
                    .iter()
                    .any(|value| value.starts_with("dnf -y") && value.contains("php83-php-fpm"))
            );
            assert!(commands.contains(&"systemctl mask php83-php-fpm.service".to_string()));
            assert!(
                !commands
                    .iter()
                    .any(|value| value.starts_with("apt") || value.starts_with("add-apt"))
            );
            assert_eq!(
                runner.read_link(Path::new("/usr/bin/php8.3")).unwrap(),
                Path::new("/opt/remi/php83/root/usr/bin/php")
            );
        }
    }

    #[test]
    fn alma_postgresql_initializes_scram_only_for_a_new_cluster() {
        for major in ["9", "10"] {
            let runner = fixture(&format!("ID=almalinux\nVERSION_ID={major}\n"));
            let hba = Path::new("/var/lib/pgsql/data/pg_hba.conf");
            runner.set_file_content(hba, b"local all all peer\nhost all all 127.0.0.1/32 ident\nhost all all 10.0.0.0/8 ident\n");
            runner.set_file_content(
                "/var/lib/pgsql/data/postgresql.conf",
                b"# native configuration\n",
            );
            let request = panel_proto::DatabaseServerInstallRequest {
                engine: "postgresql".into(),
            };
            crate::databases::install_database_server(&runner, &request).unwrap();
            let config = String::from_utf8(runner.read_file(hba).unwrap()).unwrap();
            assert!(config.contains("127.0.0.1/32 scram-sha-256"));
            assert!(config.contains("10.0.0.0/8 ident"));
            let encryption = runner
                .read_file(Path::new("/var/lib/pgsql/data/postgresql.conf"))
                .unwrap();
            assert!(
                String::from_utf8(encryption)
                    .unwrap()
                    .contains("password_encryption = 'scram-sha-256'")
            );
            runner.set_file_content("/var/lib/pgsql/data/PG_VERSION", b"13");
            runner.set_file_content(hba, b"operator configuration must survive\n");
            crate::databases::install_database_server(&runner, &request).unwrap();
            assert_eq!(
                runner.read_file(hba).unwrap(),
                b"operator configuration must survive\n"
            );
            assert_eq!(
                runner
                    .get_commands()
                    .iter()
                    .filter(|value| value.as_str() == "postgresql-setup --initdb")
                    .count(),
                1
            );
        }
    }
}
