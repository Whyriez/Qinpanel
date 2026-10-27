//! Configuration loader for `panel-api`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse TOML config: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Invalid production configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    pub server: ServerConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub analytics: AnalyticsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsConfig {
    pub path: PathBuf,
}
impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            path: "/var/lib/panel/analytics.db".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub environment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub socket_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        let url = std::env::var("PANEL_DB_PATH")
            .unwrap_or_else(|_| panel_core::production::DEFAULT_DATABASE_PATH.to_string());
        Self { url }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub session_duration_secs: u64,
    pub max_failed_attempts: u32,
    pub lockout_duration_secs: u64,
    pub cookie_secure: Option<bool>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_duration_secs: 86400, // 24 hours
            max_failed_attempts: 5,       // Lockout after 5 failed attempts
            lockout_duration_secs: 900,   // 15 minutes lockout
            cookie_secure: None,
        }
    }
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen_addr: "127.0.0.1:7700".to_string(),
                environment: "development".to_string(),
            },
            agent: AgentConfig {
                socket_path: PathBuf::from("/run/panel/agent.sock"),
            },
            database: DatabaseConfig::default(),
            auth: AuthConfig::default(),
            analytics: AnalyticsConfig::default(),
        }
    }
}

impl PanelConfig {
    pub fn load(path_override: Option<&Path>) -> Result<Self, ConfigError> {
        let candidates = if let Some(p) = path_override {
            vec![p.to_path_buf()]
        } else if let Ok(env_path) = std::env::var("PANEL_CONFIG") {
            vec![PathBuf::from(env_path)]
        } else {
            vec![
                PathBuf::from("/etc/panel/panel.toml"),
                PathBuf::from("panel.toml"),
                PathBuf::from("../../panel.toml"),
            ]
        };

        for candidate in candidates {
            if candidate.exists() {
                let content = std::fs::read_to_string(&candidate)?;
                let config: PanelConfig = toml::from_str(&content)?;
                return config.with_environment();
            }
        }

        // Return default configuration if no file exists
        Self::default().with_environment()
    }

    fn with_environment(mut self) -> Result<Self, ConfigError> {
        if let Ok(url) = std::env::var("PANEL_DB_PATH") {
            self.database.url = url;
        }
        if let Ok(addr) = std::env::var("PANEL_LISTEN_ADDR") {
            self.server.listen_addr = addr;
        }
        if let Ok(environment) = std::env::var("PANEL_ENVIRONMENT") {
            self.server.environment = environment;
        }
        if let Some(path) = std::env::var_os("PANEL_ANALYTICS_DB_PATH") {
            self.analytics.path = path.into();
        }
        if let Ok(scheme) = std::env::var("PANEL_PUBLIC_SCHEME") {
            self.apply_public_scheme(&scheme)?;
        }
        self.validate()?;
        Ok(self)
    }

    fn apply_public_scheme(&mut self, scheme: &str) -> Result<(), ConfigError> {
        let secure = match scheme {
            "http" => false,
            "https" => true,
            _ => {
                return Err(ConfigError::Invalid(
                    "PANEL_PUBLIC_SCHEME must be http or https".to_string(),
                ));
            }
        };
        // Explicit auth configuration remains authoritative for TLS deployments.
        if self.auth.cookie_secure.is_none() {
            self.auth.cookie_secure = Some(secure);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let url = if self.database.url.starts_with("sqlite:") {
            self.database.url.clone()
        } else {
            format!("sqlite://{}", self.database.url.replace('\\', "/"))
        };
        let options: sqlx::sqlite::SqliteConnectOptions = url
            .parse()
            .map_err(|e| ConfigError::Invalid(format!("Invalid SQLite URL: {e}")))?;
        let core = options.get_filename();
        if core != Path::new(":memory:")
            && (panel_db::operations::same_file_path(core, &self.analytics.path)
                || panel_db::operations::same_file_path(
                    &panel_db::operations::sidecar_path(core),
                    &self.analytics.path,
                ))
        {
            return Err(ConfigError::Invalid(
                "Core, operations and analytics databases must use distinct files".to_owned(),
            ));
        }
        if self.server.environment == "production" {
            if !self.analytics.path.is_absolute()
                && !self.analytics.path.to_string_lossy().starts_with('/')
            {
                return Err(ConfigError::Invalid(
                    "production analytics database path must be absolute".to_owned(),
                ));
            }
            if self.server.listen_addr != "127.0.0.1:7700" {
                return Err(ConfigError::Invalid(
                    "production API must listen on 127.0.0.1:7700".to_string(),
                ));
            }
            let path = self
                .database
                .url
                .strip_prefix("sqlite://")
                .unwrap_or(&self.database.url);
            // This configuration targets Linux even when checked on Windows.
            if !path.starts_with('/') {
                return Err(ConfigError::Invalid(
                    "production database path must be absolute".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_cannot_share_a_file_with_core_or_operations() {
        let temp = tempfile::tempdir().unwrap();
        let core = temp.path().join("panel.db");
        let mut config = PanelConfig::default();
        config.database.url = core.to_string_lossy().into_owned();
        config.analytics.path = core.clone();
        assert!(config.validate().is_err());
        config.analytics.path = panel_db::operations::sidecar_path(&core);
        assert!(config.validate().is_err());
        config.analytics.path = temp.path().join("analytics.db");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn fresh_http_install_can_use_session_cookies() {
        let mut config = PanelConfig::default();
        assert!(config.apply_public_scheme("http").is_ok());
        assert_eq!(config.auth.cookie_secure, Some(false));
        config.auth.cookie_secure = None;
        assert!(config.apply_public_scheme("https").is_ok());
        assert_eq!(config.auth.cookie_secure, Some(true));
        assert!(config.apply_public_scheme("invalid").is_err());
        assert!(config.apply_public_scheme("http").is_ok());
        assert_eq!(config.auth.cookie_secure, Some(true));
    }

    #[test]
    fn production_rejects_relative_database_and_public_api() {
        let mut config = PanelConfig::default();
        config.server.environment = "production".to_string();
        config.database.url = "sqlite://panel.db".to_string();
        assert!(config.validate().is_err());
        config.database.url = "sqlite:///var/lib/panel/panel.db".to_string();
        assert!(config.validate().is_ok());
        config.server.listen_addr = "0.0.0.0:7700".to_string();
        assert!(config.validate().is_err());
    }
}
