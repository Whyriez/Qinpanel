//! PHP runtime domain models, version definitions, package lists, and service helpers.

use crate::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// Whitelisted, supported PHP versions in Qinpanel.
/// Parallel versions provided via Ondřej Surý PPA / Debian repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PhpVersion {
    #[serde(rename = "7.4")]
    Php74,
    #[serde(rename = "8.0")]
    Php80,
    #[serde(rename = "8.1")]
    Php81,
    #[serde(rename = "8.2")]
    Php82,
    #[serde(rename = "8.3")]
    Php83,
    #[serde(rename = "8.4")]
    Php84,
    #[serde(rename = "8.5")]
    Php85,
}

impl PhpVersion {
    /// Returns all supported PHP versions ordered from oldest to newest.
    pub const fn all_supported() -> &'static [PhpVersion] {
        &[
            PhpVersion::Php74,
            PhpVersion::Php80,
            PhpVersion::Php81,
            PhpVersion::Php82,
            PhpVersion::Php83,
            PhpVersion::Php84,
            PhpVersion::Php85,
        ]
    }

    /// Parses a version string with strict whitelisting. Prevents command injection.
    pub fn parse(s: &str) -> Result<Self, RuntimeError> {
        let clean = s.trim().to_ascii_lowercase();
        let stripped = clean.strip_prefix("php").unwrap_or(&clean);

        match stripped {
            "7.4" => Ok(Self::Php74),
            "8.0" => Ok(Self::Php80),
            "8.1" => Ok(Self::Php81),
            "8.2" => Ok(Self::Php82),
            "8.3" => Ok(Self::Php83),
            "8.4" => Ok(Self::Php84),
            "8.5" => Ok(Self::Php85),
            _ => Err(RuntimeError::UnsupportedPhpVersion(s.to_string())),
        }
    }

    /// Returns the canonical version string (e.g. "8.3").
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Php74 => "7.4",
            Self::Php80 => "8.0",
            Self::Php81 => "8.1",
            Self::Php82 => "8.2",
            Self::Php83 => "8.3",
            Self::Php84 => "8.4",
            Self::Php85 => "8.5",
        }
    }

    /// Systemd unit name for the default OS FPM service (must be masked by the panel).
    pub fn fpm_service_name(&self) -> String {
        format!("php{}-fpm.service", self.as_str())
    }

    /// Default path to the FPM binary.
    pub fn fpm_binary_path(&self) -> PathBuf {
        PathBuf::from(format!("/usr/sbin/php-fpm{}", self.as_str()))
    }

    /// Default path to the CLI binary.
    pub fn cli_binary_path(&self) -> PathBuf {
        PathBuf::from(format!("/usr/bin/php{}", self.as_str()))
    }

    /// Default path to the FPM configuration directory.
    pub fn fpm_config_dir(&self) -> PathBuf {
        PathBuf::from(format!("/etc/php/{}/fpm", self.as_str()))
    }

    /// Default path to the CLI configuration directory.
    pub fn cli_config_dir(&self) -> PathBuf {
        PathBuf::from(format!("/etc/php/{}/cli", self.as_str()))
    }

    /// Returns list of Debian/Ubuntu packages to install for this PHP version.
    pub fn base_packages(&self) -> Vec<String> {
        let v = self.as_str();
        let mut packages = vec![
            format!("php{v}-fpm"),
            format!("php{v}-cli"),
            format!("php{v}-common"),
            format!("php{v}-curl"),
            format!("php{v}-mbstring"),
            format!("php{v}-xml"),
            format!("php{v}-zip"),
            format!("php{v}-mysql"),
        ];
        // PHP 8.5 always links OPcache into the binary; no separate package exists.
        if *self < Self::Php85 {
            packages.insert(3, format!("php{v}-opcache"));
        }
        packages
    }
}

impl fmt::Display for PhpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Packages without an install candidate in `LC_ALL=C apt-cache policy` output.
/// A missing package stanza is also unavailable; successful index refresh alone
/// does not prove that the PPA publishes the requested version/architecture.
pub fn missing_php_package_candidates(version: PhpVersion, policy: &str) -> Vec<String> {
    version
        .base_packages()
        .into_iter()
        .filter(|package| {
            let header = format!("{package}:");
            !policy
                .lines()
                .skip_while(|line| line.trim_end() != header)
                .skip(1)
                .take_while(|line| line.is_empty() || line.starts_with(char::is_whitespace))
                .filter_map(|line| line.trim().strip_prefix("Candidate:"))
                .any(|candidate| !candidate.trim().is_empty() && candidate.trim() != "(none)")
        })
        .collect()
}

/// Detailed status information about a PHP runtime version on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhpVersionInfo {
    pub version: String,
    pub installed: bool,
    pub fpm_binary_exists: bool,
    pub cli_binary_exists: bool,
    pub service_masked: bool,
    pub status: String,
    pub fpm_binary_path: String,
    pub cli_binary_path: String,
    pub service_name: String,
}

impl PhpVersionInfo {
    /// Creates status from version and host check indicators.
    pub fn new(
        version: PhpVersion,
        fpm_binary_exists: bool,
        cli_binary_exists: bool,
        service_masked: bool,
    ) -> Self {
        let installed = fpm_binary_exists && cli_binary_exists;
        let status = if installed {
            "installed".to_string()
        } else {
            "not_installed".to_string()
        };

        Self {
            version: version.as_str().to_string(),
            installed,
            fpm_binary_exists,
            cli_binary_exists,
            service_masked,
            status,
            fpm_binary_path: version.fpm_binary_path().to_string_lossy().to_string(),
            cli_binary_path: version.cli_binary_path().to_string_lossy().to_string(),
            service_name: version.fpm_service_name(),
        }
    }
}

/// Checks known Ubuntu PPA and Debian Sury repository files in APT sources.
pub fn is_sury_repo_installed(path_checker: impl Fn(&Path) -> bool) -> bool {
    let candidates = [
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php-jammy.list"),
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php-noble.list"),
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php-jammy.sources"),
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php-noble.sources"),
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php.list"),
        Path::new("/etc/apt/sources.list.d/ondrej-ubuntu-php.sources"),
        Path::new("/etc/apt/sources.list.d/php.list"),
        Path::new("/etc/apt/sources.list.d/php.sources"),
        Path::new("/etc/apt/sources.list.d/nusa-php.sources"),
        Path::new("/etc/apt/sources.list.d/sury-php.sources"),
    ];
    candidates.iter().any(|p| path_checker(p))
}

/// Represents a curated PHP extension available in Qinpanel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PhpExtensionDefinition {
    pub name: &'static str,
    pub package_suffix: &'static str,
    pub ini_names: &'static [&'static str],
    pub category: &'static str,
    pub description_id: &'static str,
    pub is_base: bool,
}

impl PhpExtensionDefinition {
    pub fn is_builtin(&self, version: PhpVersion) -> bool {
        self.name == "opcache" && version >= PhpVersion::Php85
    }
    /// Returns the Debian/Ubuntu package name for a specific PHP version.
    pub fn package_name(&self, version: PhpVersion) -> String {
        if self.is_builtin(version) {
            return format!("php{}-common", version.as_str());
        }
        format!("php{}-{}", version.as_str(), self.package_suffix)
    }
}

/// Detailed status information about a specific PHP extension on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhpExtensionInfo {
    pub name: String,
    pub version: String,
    pub package_name: String,
    pub installed: bool,
    pub enabled: bool,
    pub is_base: bool,
    pub category: String,
    pub description: String,
}

/// Curated list of standard PHP extensions supported by Qinpanel.
pub const CURATED_PHP_EXTENSIONS: &[PhpExtensionDefinition] = &[
    PhpExtensionDefinition {
        name: "bcmath",
        package_suffix: "bcmath",
        ini_names: &["bcmath"],
        category: "Math",
        description_id: "Dukungan matematika presisi sembarang (Binary Calculator)",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "bz2",
        package_suffix: "bz2",
        ini_names: &["bz2"],
        category: "Compression",
        description_id: "Kompresi dan dekompresi bzip2",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "curl",
        package_suffix: "curl",
        ini_names: &["curl"],
        category: "Networking",
        description_id: "Klien HTTP dan transfer data jaringan cURL",
        is_base: true,
    },
    PhpExtensionDefinition {
        name: "gd",
        package_suffix: "gd",
        ini_names: &["gd"],
        category: "Graphics",
        description_id: "Pemrosesan dan manipulasi gambar GD",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "imagick",
        package_suffix: "imagick",
        ini_names: &["imagick"],
        category: "Graphics",
        description_id: "Pemrosesan gambar canggih ImageMagick",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "intl",
        package_suffix: "intl",
        ini_names: &["intl"],
        category: "Internationalization",
        description_id: "Internasionalisasi dan lokalisasi ICU",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "mbstring",
        package_suffix: "mbstring",
        ini_names: &["mbstring"],
        category: "Strings",
        description_id: "Dukungan string multi-byte UTF-8",
        is_base: true,
    },
    PhpExtensionDefinition {
        name: "mysql",
        package_suffix: "mysql",
        ini_names: &["mysqli", "pdo_mysql"],
        category: "Database",
        description_id: "Driver konektivitas MySQLi dan PDO MySQL",
        is_base: true,
    },
    PhpExtensionDefinition {
        name: "opcache",
        package_suffix: "opcache",
        ini_names: &["opcache"],
        category: "Performance",
        description_id: "Zend OPcache bytecode caching",
        is_base: true,
    },
    PhpExtensionDefinition {
        name: "pgsql",
        package_suffix: "pgsql",
        ini_names: &["pgsql", "pdo_pgsql"],
        category: "Database",
        description_id: "Driver konektivitas PostgreSQL dan PDO PgSQL",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "readline",
        package_suffix: "readline",
        ini_names: &["readline"],
        category: "CLI",
        description_id: "Interaksi CLI dan history Readline",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "redis",
        package_suffix: "redis",
        ini_names: &["redis"],
        category: "Cache",
        description_id: "Driver performa tinggi Redis caching",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "soap",
        package_suffix: "soap",
        ini_names: &["soap"],
        category: "WebServices",
        description_id: "Klien dan server protokol SOAP/XML",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "sqlite3",
        package_suffix: "sqlite3",
        ini_names: &["sqlite3", "pdo_sqlite"],
        category: "Database",
        description_id: "Database SQLite3 tersemat dan PDO SQLite",
        is_base: false,
    },
    PhpExtensionDefinition {
        name: "xml",
        package_suffix: "xml",
        ini_names: &["xml"],
        category: "XML",
        description_id: "Parser DOM, SimpleXML, dan XMLReader/Writer",
        is_base: true,
    },
    PhpExtensionDefinition {
        name: "zip",
        package_suffix: "zip",
        ini_names: &["zip"],
        category: "Compression",
        description_id: "Membaca dan membuat arsip berkas Zip",
        is_base: true,
    },
];

/// Finds a curated PHP extension definition by name with strict validation.
pub fn find_curated_extension(name: &str) -> Result<&'static PhpExtensionDefinition, RuntimeError> {
    let clean = name.trim().to_ascii_lowercase();
    CURATED_PHP_EXTENSIONS
        .iter()
        .find(|ext| ext.name == clean)
        .ok_or_else(|| RuntimeError::UnsupportedPhpExtension(name.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn package_candidates_require_every_base_package() {
        let version = PhpVersion::Php84;
        let policy: String = version
            .base_packages()
            .iter()
            .map(|package| format!("{package}:\n  Installed: (none)\n  Candidate: 8.4.1-1\n  Version table:\n     8.4.1-1 500\n        500 https://ppa.launchpadcontent.net/ondrej/php/ubuntu noble/main amd64 Packages\n"))
            .collect();
        assert!(missing_php_package_candidates(version, &policy).is_empty());
        let policy = policy.replacen("Candidate: 8.4.1-1", "Candidate: (none)", 1);
        assert_eq!(
            missing_php_package_candidates(version, &policy),
            vec!["php8.4-fpm"]
        );
        assert_eq!(
            missing_php_package_candidates(version, ""),
            version.base_packages()
        );
    }

    #[test]
    fn installed_version_and_neighbour_candidate_do_not_prove_availability() {
        let policy = "php8.4-fpm:\n  Installed: 8.4.1\n  Candidate: (none)\nphp8.4-cli:\n  Candidate: 8.4.1\n";
        let missing = missing_php_package_candidates(PhpVersion::Php84, policy);
        assert!(missing.contains(&"php8.4-fpm".to_string()));
        assert!(!missing.contains(&"php8.4-cli".to_string()));
        assert!(missing.contains(&"php8.4-mysql".to_string()));
    }

    #[test]
    fn test_php85_builtin_opcache_has_no_separate_package() {
        let version = PhpVersion::Php85;
        assert!(!version.base_packages().contains(&"php8.5-opcache".into()));
        assert!(version.base_packages().contains(&"php8.5-fpm".into()));
        assert!(
            PhpVersion::Php84
                .base_packages()
                .contains(&"php8.4-opcache".into())
        );
        let opcache = find_curated_extension("opcache").unwrap();
        assert!(opcache.is_builtin(version));
        assert!(!opcache.is_builtin(PhpVersion::Php84));
        assert_eq!(opcache.package_name(version), "php8.5-common");
    }

    #[test]
    fn test_php_version_parsing_and_whitelist() {
        assert_eq!(PhpVersion::parse("7.4").unwrap(), PhpVersion::Php74);
        assert_eq!(PhpVersion::parse("8.0").unwrap(), PhpVersion::Php80);
        assert_eq!(PhpVersion::parse("8.1").unwrap(), PhpVersion::Php81);
        assert_eq!(PhpVersion::parse("8.2").unwrap(), PhpVersion::Php82);
        assert_eq!(PhpVersion::parse("8.3").unwrap(), PhpVersion::Php83);
        assert_eq!(PhpVersion::parse("8.4").unwrap(), PhpVersion::Php84);
        assert_eq!(PhpVersion::parse("php8.3").unwrap(), PhpVersion::Php83);

        // Reject arbitrary versions or injections
        assert!(PhpVersion::parse("5.6").is_err());
        assert!(PhpVersion::parse("8.3; rm -rf /").is_err());
        assert!(PhpVersion::parse("latest").is_err());
        assert!(PhpVersion::parse("../etc/passwd").is_err());
    }

    #[test]
    fn test_php_version_binary_paths_and_packages() {
        let v83 = PhpVersion::Php83;
        assert_eq!(v83.as_str(), "8.3");
        assert_eq!(v83.fpm_service_name(), "php8.3-fpm.service");
        assert_eq!(v83.fpm_binary_path(), PathBuf::from("/usr/sbin/php-fpm8.3"));
        assert_eq!(v83.cli_binary_path(), PathBuf::from("/usr/bin/php8.3"));

        let pkgs = v83.base_packages();
        assert!(pkgs.contains(&"php8.3-fpm".to_string()));
        assert!(pkgs.contains(&"php8.3-cli".to_string()));
        assert!(pkgs.contains(&"php8.3-mbstring".to_string()));
    }

    #[test]
    fn test_php_version_info_status() {
        let info = PhpVersionInfo::new(PhpVersion::Php83, true, true, true);
        assert!(info.installed);
        assert_eq!(info.status, "installed");
        assert!(info.service_masked);

        let uninstalled = PhpVersionInfo::new(PhpVersion::Php83, false, false, false);
        assert!(!uninstalled.installed);
        assert_eq!(uninstalled.status, "not_installed");
    }

    #[test]
    fn test_php_repository_detects_ubuntu_deb822_sources() {
        assert!(is_sury_repo_installed(|path| path
            == Path::new(
                "/etc/apt/sources.list.d/ondrej-ubuntu-php-noble.sources"
            )));
        assert!(is_sury_repo_installed(|path| path
            == Path::new(
                "/etc/apt/sources.list.d/ondrej-ubuntu-php-noble.list"
            )));
        assert!(is_sury_repo_installed(
            |path| path == Path::new("/etc/apt/sources.list.d/nusa-php.sources")
        ));
        assert!(!is_sury_repo_installed(|_| false));
    }

    #[test]
    fn test_curated_php_extensions_lookup_and_packages() {
        assert_eq!(CURATED_PHP_EXTENSIONS.len(), 16);

        let redis = find_curated_extension("redis").unwrap();
        assert_eq!(redis.name, "redis");
        assert_eq!(redis.package_name(PhpVersion::Php83), "php8.3-redis");
        assert!(!redis.is_base);

        let mbstring = find_curated_extension("  MbString ").unwrap();
        assert_eq!(mbstring.name, "mbstring");
        assert!(mbstring.is_base);

        // Disallowed / non-curated extension rejected
        assert!(find_curated_extension("xdebug").is_err());
        assert!(find_curated_extension("exec; rm -rf /").is_err());
        assert!(find_curated_extension("../../../etc/passwd").is_err());
    }
}
