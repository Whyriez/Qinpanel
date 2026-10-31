//! Domain logic, validators, and password utilities for database provisioning.

use crate::CoreError;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

/// Supported database management engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum DatabaseEngine {
    MariaDb,
    PostgreSql,
}

impl DatabaseEngine {
    pub const ALL: [Self; 2] = [Self::MariaDb, Self::PostgreSql];

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MariaDb => "MariaDB",
            Self::PostgreSql => "PostgreSQL",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MariaDb => "mariadb",
            Self::PostgreSql => "postgresql",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.to_ascii_lowercase().as_str() {
            "mariadb" | "mysql" => Ok(Self::MariaDb),
            "postgresql" | "postgres" => Ok(Self::PostgreSql),
            other => Err(CoreError::Validation(format!(
                "Engine database tidak didukung: '{other}'. Pilih 'mariadb' atau 'postgresql'."
            ))),
        }
    }

    pub fn default_port(&self) -> u16 {
        match self {
            Self::MariaDb => 3306,
            Self::PostgreSql => 5432,
        }
    }

    pub fn default_socket_path(&self) -> &'static str {
        match self {
            Self::MariaDb => "/run/mysqld/mysqld.sock",
            Self::PostgreSql => "/var/run/postgresql/.s.PGSQL.5432",
        }
    }

    pub fn systemd_service_name(&self) -> &'static str {
        match self {
            Self::MariaDb => "mariadb",
            Self::PostgreSql => "postgresql",
        }
    }

    pub fn max_db_name_len(&self) -> usize {
        match self {
            Self::MariaDb => 64,
            Self::PostgreSql => 63,
        }
    }

    pub fn max_user_name_len(&self) -> usize {
        match self {
            Self::MariaDb => 32,
            Self::PostgreSql => 63,
        }
    }
}

/// Computes the standard database identifier prefix for a tenant.
/// Example: `p_alice` -> `p_alice_`, `bob` -> `bob_`.
pub fn tenant_db_prefix(username: &str) -> String {
    let clean = username.trim().to_ascii_lowercase();
    if clean.ends_with('_') {
        clean
    } else {
        format!("{clean}_")
    }
}

/// SQL charset/collation tokens are identifiers, never SQL fragments.
pub fn validate_database_encoding(value: &str) -> Result<(), CoreError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(CoreError::Validation(
            "Charset atau collation database tidak valid".into(),
        ));
    }
    Ok(())
}

/// Reserved database names across MariaDB and PostgreSQL.
const RESERVED_DATABASES: &[&str] = &[
    "mysql",
    "information_schema",
    "performance_schema",
    "sys",
    "test",
    "postgres",
    "template0",
    "template1",
];

/// Reserved database user names across MariaDB and PostgreSQL.
const RESERVED_USERS: &[&str] = &[
    "root",
    "admin",
    "administrator",
    "mysql",
    "mariadb",
    "postgres",
    "debian-sys-maint",
    "system",
    "guest",
];

/// Validates database name adhering to prefix and engine constraints.
pub fn validate_database_name(
    name: &str,
    expected_prefix: &str,
    engine: DatabaseEngine,
) -> Result<(), CoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Nama database tidak boleh kosong".to_string(),
        ));
    }

    let prefix = expected_prefix.trim().to_ascii_lowercase();
    if !trimmed.to_ascii_lowercase().starts_with(&prefix) {
        return Err(CoreError::Validation(format!(
            "Nama database wajib diawali dengan prefix tenant '{prefix}'"
        )));
    }

    if trimmed.len() > engine.max_db_name_len() {
        return Err(CoreError::Validation(format!(
            "Nama database untuk {} maksimal {} karakter (diberikan: {} karakter)",
            engine.as_str(),
            engine.max_db_name_len(),
            trimmed.len()
        )));
    }

    // Characters must be alphanumeric or underscore
    for c in trimmed.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            return Err(CoreError::Validation(format!(
                "Nama database hanya boleh memuat huruf, angka, dan garis bawah (_): ditemukan '{c}'"
            )));
        }
    }

    let suffix = &trimmed[prefix.len()..];
    if suffix.is_empty() {
        return Err(CoreError::Validation(
            "Nama database harus memiliki nama setelah prefix".to_string(),
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    if RESERVED_DATABASES.contains(&lower.as_str()) {
        return Err(CoreError::Validation(format!(
            "Nama database '{lower}' merupakan nama sistem yang diproteksi"
        )));
    }

    Ok(())
}

/// Validates database user adhering to prefix and engine constraints.
pub fn validate_database_user(
    user: &str,
    expected_prefix: &str,
    engine: DatabaseEngine,
) -> Result<(), CoreError> {
    let trimmed = user.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Nama pengguna database tidak boleh kosong".to_string(),
        ));
    }

    let prefix = expected_prefix.trim().to_ascii_lowercase();
    if !trimmed.to_ascii_lowercase().starts_with(&prefix) {
        return Err(CoreError::Validation(format!(
            "Nama pengguna database wajib diawali dengan prefix tenant '{prefix}'"
        )));
    }

    if trimmed.len() > engine.max_user_name_len() {
        return Err(CoreError::Validation(format!(
            "Nama pengguna untuk {} maksimal {} karakter (diberikan: {} karakter)",
            engine.as_str(),
            engine.max_user_name_len(),
            trimmed.len()
        )));
    }

    for c in trimmed.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            return Err(CoreError::Validation(format!(
                "Nama pengguna hanya boleh memuat huruf, angka, dan garis bawah (_): ditemukan '{c}'"
            )));
        }
    }

    let suffix = &trimmed[prefix.len()..];
    if suffix.is_empty() {
        return Err(CoreError::Validation(
            "Nama pengguna harus memiliki nama setelah prefix".to_string(),
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    if RESERVED_USERS.contains(&lower.as_str()) {
        return Err(CoreError::Validation(format!(
            "Nama pengguna '{lower}' merupakan nama akun sistem yang diproteksi"
        )));
    }

    Ok(())
}

/// Validates that database password is strong and safe against injection.
pub fn validate_database_password(password: &str) -> Result<(), CoreError> {
    if password.len() < 12 {
        return Err(CoreError::Validation(
            "Kata sandi database minimal harus 12 karakter".to_string(),
        ));
    }
    if password.len() > 128 {
        return Err(CoreError::Validation(
            "Kata sandi database maksimal 128 karakter".to_string(),
        ));
    }

    // Guard against SQL injection delimiters and control characters
    for c in password.chars() {
        if c == '\'' || c == '"' || c == ';' || c == '\\' || c == '\0' || c == '\n' || c == '\r' {
            return Err(CoreError::Validation(format!(
                "Kata sandi tidak boleh memuat karakter sensitif SQL atau newline: '{c}'"
            )));
        }
        if c.is_ascii_control() {
            return Err(CoreError::Validation(
                "Kata sandi tidak boleh memuat karakter kontrol".to_string(),
            ));
        }
    }

    Ok(())
}

/// Generates a cryptographically strong database password (18 chars, safe charset).
pub fn generate_secure_db_password() -> String {
    const CHARSET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!#%^&*_-+=";
    let mut rng = OsRng;
    let mut password = String::with_capacity(18);
    for _ in 0..18 {
        let idx = (rng.next_u32() as usize) % CHARSET.len();
        password.push(CHARSET[idx] as char);
    }
    password
}

/// Validates host access specification.
pub fn validate_allowed_hosts(allowed_hosts: &str, remote_access: bool) -> Result<(), CoreError> {
    let trimmed = allowed_hosts.trim();
    if !remote_access {
        if trimmed.is_empty()
            || trimmed == "localhost"
            || trimmed == "127.0.0.1"
            || trimmed == "::1"
        {
            return Ok(());
        }
        return Err(CoreError::Validation(
            "Akses jarak jauh nonaktif. Allowed hosts wajib bernilai 'localhost'".to_string(),
        ));
    }

    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Daftar host yang diizinkan tidak boleh kosong saat remote access aktif".to_string(),
        ));
    }

    if trimmed == "%" {
        return Ok(());
    }

    // Can be comma-separated list of IP addresses, CIDRs, or hostnames
    for host in trimmed.split(',') {
        let h = host.trim();
        if h.is_empty() {
            continue;
        }
        if h == "%" || h == "localhost" {
            continue;
        }

        // Check if valid IP or CIDR
        if let Some((ip_str, prefix_str)) = h.split_once('/') {
            if ip_str.parse::<std::net::IpAddr>().is_err() {
                return Err(CoreError::Validation(format!(
                    "Format IP tidak valid dalam CIDR: '{ip_str}'"
                )));
            }
            let prefix: u8 = prefix_str.parse().map_err(|_| {
                CoreError::Validation(format!("Prefix CIDR tidak valid: '{prefix_str}'"))
            })?;
            if prefix > 128 {
                return Err(CoreError::Validation(format!(
                    "Prefix CIDR melebihi batas: {prefix}"
                )));
            }
        } else if h.parse::<std::net::IpAddr>().is_err() {
            // Must be alphanumeric hostname, valid IP, or wildcard pattern
            for c in h.chars() {
                if !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '%' {
                    return Err(CoreError::Validation(format!(
                        "Host remote tidak valid: '{h}'. Gunakan IP, CIDR, atau '%'"
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Validates database dump/import file path within tenant confinement.
pub fn validate_dump_path(path: &str, tenant_username: &str) -> Result<(), CoreError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Path berkas tidak boleh kosong".to_string(),
        ));
    }
    if trimmed.contains("..") || trimmed.contains('\0') || trimmed.contains('\n') {
        return Err(CoreError::Validation(
            "Path berkas tidak boleh memuat penelusuran relatif (..) atau karakter ilegal"
                .to_string(),
        ));
    }

    let expected_home = format!("/home/{tenant_username}");
    if trimmed.starts_with('/') && !trimmed.starts_with(&expected_home) {
        return Err(CoreError::Validation(format!(
            "Path berkas wajib berada di dalam direktori tenant: '{expected_home}'"
        )));
    }

    let lower = trimmed.to_ascii_lowercase();
    if !lower.ends_with(".sql") && !lower.ends_with(".sql.gz") && !lower.ends_with(".dump") {
        return Err(CoreError::Validation(
            "Ekstensi berkas dump wajib berakhiran '.sql', '.sql.gz', atau '.dump'".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_engine_parse_and_properties() {
        assert_eq!(
            DatabaseEngine::parse("mariadb").unwrap(),
            DatabaseEngine::MariaDb
        );
        assert_eq!(
            DatabaseEngine::parse("mysql").unwrap(),
            DatabaseEngine::MariaDb
        );
        assert_eq!(
            DatabaseEngine::parse("postgresql").unwrap(),
            DatabaseEngine::PostgreSql
        );
        assert_eq!(
            DatabaseEngine::parse("postgres").unwrap(),
            DatabaseEngine::PostgreSql
        );
        assert!(DatabaseEngine::parse("sqlite").is_err());

        assert_eq!(DatabaseEngine::MariaDb.default_port(), 3306);
        assert_eq!(DatabaseEngine::PostgreSql.default_port(), 5432);
    }

    #[test]
    fn test_tenant_db_prefix() {
        assert_eq!(tenant_db_prefix("p_alice"), "p_alice_");
        assert_eq!(tenant_db_prefix("p_alice_"), "p_alice_");
        assert_eq!(tenant_db_prefix("bob"), "bob_");
    }

    #[test]
    fn test_validate_database_name() {
        let prefix = "p_alice_";
        // Valid
        assert!(
            validate_database_name("p_alice_wordpress", prefix, DatabaseEngine::MariaDb).is_ok()
        );
        assert!(
            validate_database_name("p_alice_app123", prefix, DatabaseEngine::PostgreSql).is_ok()
        );

        // Invalid: missing prefix
        assert!(validate_database_name("wordpress", prefix, DatabaseEngine::MariaDb).is_err());

        // Invalid: prefix only
        assert!(validate_database_name("p_alice_", prefix, DatabaseEngine::MariaDb).is_err());

        // Invalid: special chars
        assert!(validate_database_name("p_alice_my-db", prefix, DatabaseEngine::MariaDb).is_err());
        assert!(
            validate_database_name("p_alice_db;drop", prefix, DatabaseEngine::MariaDb).is_err()
        );

        // Invalid: reserved names
        assert!(validate_database_name("mysql", "my", DatabaseEngine::MariaDb).is_err());

        // Too long for postgres (63 chars)
        let long_name = format!("p_alice_{}", "a".repeat(60));
        assert!(validate_database_name(&long_name, prefix, DatabaseEngine::PostgreSql).is_err());
    }

    #[test]
    fn test_validate_database_user() {
        let prefix = "p_alice_";
        assert!(validate_database_user("p_alice_u1", prefix, DatabaseEngine::MariaDb).is_ok());
        assert!(validate_database_user("u1", prefix, DatabaseEngine::MariaDb).is_err());
        assert!(validate_database_user("p_alice_", prefix, DatabaseEngine::MariaDb).is_err());

        // MariaDB max user is 32
        let long_user = format!("p_alice_{}", "u".repeat(26));
        assert!(validate_database_user(&long_user, prefix, DatabaseEngine::MariaDb).is_err());
    }

    #[test]
    fn test_validate_database_password() {
        assert!(validate_database_password("Short1!").is_err());
        assert!(validate_database_password("ValidPassword123!@#").is_ok());
        assert!(validate_database_password("ValidPass'Injected").is_err());
        assert!(validate_database_password("ValidPass\"Injected").is_err());
        assert!(validate_database_password("ValidPass;Injected").is_err());
        assert!(validate_database_password("ValidPass\\Injected").is_err());
    }

    #[test]
    fn test_generate_secure_db_password() {
        let pass = generate_secure_db_password();
        assert!(pass.len() >= 18);
        assert!(validate_database_password(&pass).is_ok());
    }

    #[test]
    fn test_validate_allowed_hosts() {
        assert!(validate_allowed_hosts("localhost", false).is_ok());
        assert!(validate_allowed_hosts("%", false).is_err());

        assert!(validate_allowed_hosts("%", true).is_ok());
        assert!(validate_allowed_hosts("192.168.1.100", true).is_ok());
        assert!(validate_allowed_hosts("10.0.0.0/24, 192.168.1.50", true).is_ok());
        assert!(validate_allowed_hosts("invalid_ip$$", true).is_err());
    }

    #[test]
    fn test_validate_dump_path() {
        assert!(validate_dump_path("/home/p_alice/backup.sql", "p_alice").is_ok());
        assert!(validate_dump_path("/home/p_alice/dumps/data.sql.gz", "p_alice").is_ok());
        assert!(validate_dump_path("/home/p_bob/backup.sql", "p_alice").is_err());
        assert!(validate_dump_path("/home/p_alice/../etc/passwd.sql", "p_alice").is_err());
        assert!(validate_dump_path("/home/p_alice/backup.exe", "p_alice").is_err());
    }
}
