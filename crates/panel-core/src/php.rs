//! Curated, bounded PHP directives shared by the API and privileged agent.
use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PhpSettings {
    pub memory_limit_mb: u32,
    pub upload_max_filesize_mb: u32,
    pub post_max_size_mb: u32,
    pub max_execution_time: u32,
    pub max_input_vars: u32,
    pub display_errors: bool,
    pub date_timezone: String,
    pub opcache_enable: bool,
    pub opcache_memory_consumption_mb: u32,
    pub opcache_validate_timestamps: bool,
    pub opcache_revalidate_freq: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PhpSettingsLimits {
    pub memory_limit_mb: u32,
    pub upload_limit_mb: u32,
    pub max_execution_time: u32,
    pub max_input_vars: u32,
    pub opcache_memory_consumption_mb: u32,
}

impl Default for PhpSettingsLimits {
    fn default() -> Self {
        Self {
            memory_limit_mb: 256,
            upload_limit_mb: 64,
            max_execution_time: 300,
            max_input_vars: 100_000,
            opcache_memory_consumption_mb: 128,
        }
    }
}

impl PhpSettingsLimits {
    pub fn from_plan(memory_bytes: i64, upload_bytes: i64) -> Self {
        let mut limits = Self::default();
        limits.memory_limit_mb = (memory_bytes / 1_048_576).clamp(1, 4096) as u32;
        limits.upload_limit_mb = (upload_bytes / 1_048_576).clamp(1, 1024) as u32;
        limits.opcache_memory_consumption_mb = limits.memory_limit_mb.min(128);
        limits
    }
}

impl Default for PhpSettings {
    fn default() -> Self {
        Self::for_limits(&PhpSettingsLimits::default())
    }
}

impl PhpSettings {
    pub fn for_limits(limits: &PhpSettingsLimits) -> Self {
        Self {
            memory_limit_mb: limits.memory_limit_mb.min(128),
            upload_max_filesize_mb: limits.upload_limit_mb.min(32),
            post_max_size_mb: limits.upload_limit_mb.min(32),
            max_execution_time: 30,
            max_input_vars: 1000,
            display_errors: false,
            date_timezone: "UTC".into(),
            opcache_enable: true,
            opcache_memory_consumption_mb: limits.opcache_memory_consumption_mb.min(64),
            opcache_validate_timestamps: true,
            opcache_revalidate_freq: 2,
        }
    }

    pub fn validate(&self, limits: &PhpSettingsLimits) -> Result<(), CoreError> {
        for (key, value, max) in [
            (
                "memory_limit",
                self.memory_limit_mb,
                limits.memory_limit_mb.min(4096),
            ),
            (
                "upload_max_filesize",
                self.upload_max_filesize_mb,
                limits.upload_limit_mb.min(1024),
            ),
            (
                "post_max_size",
                self.post_max_size_mb,
                limits.upload_limit_mb.min(1024),
            ),
            (
                "max_execution_time",
                self.max_execution_time,
                limits.max_execution_time.min(300),
            ),
            (
                "max_input_vars",
                self.max_input_vars,
                limits.max_input_vars.min(100_000),
            ),
            (
                "opcache.memory_consumption",
                self.opcache_memory_consumption_mb,
                limits.opcache_memory_consumption_mb.min(128),
            ),
        ] {
            if value == 0 || value > max {
                return Err(CoreError::Validation(format!(
                    "{key} harus antara 1 dan {max}"
                )));
            }
        }
        if self.upload_max_filesize_mb > self.post_max_size_mb || self.opcache_revalidate_freq > 60
        {
            return Err(CoreError::Validation(
                "post_max_size harus ≥ upload_max_filesize; frekuensi OPcache maksimal 60 detik"
                    .into(),
            ));
        }
        // Time zone names come from the installed zoneinfo database at the agent boundary.
        if self.date_timezone.is_empty()
            || self.date_timezone.len() > 64
            || self.date_timezone.starts_with('/')
            || self.date_timezone.contains("..")
            || !self
                .date_timezone
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_+-".contains(&c))
        {
            return Err(CoreError::Validation("Zona waktu PHP tidak valid".into()));
        }
        Ok(())
    }

    pub fn directives(&self) -> BTreeMap<String, String> {
        let flag = |b| if b { "1" } else { "0" }.to_string();
        [
            ("memory_limit", format!("{}M", self.memory_limit_mb)),
            (
                "upload_max_filesize",
                format!("{}M", self.upload_max_filesize_mb),
            ),
            ("post_max_size", format!("{}M", self.post_max_size_mb)),
            ("max_execution_time", self.max_execution_time.to_string()),
            ("max_input_vars", self.max_input_vars.to_string()),
            ("display_errors", flag(self.display_errors)),
            ("date.timezone", self.date_timezone.clone()),
            ("opcache.enable", flag(self.opcache_enable)),
            (
                "opcache.memory_consumption",
                self.opcache_memory_consumption_mb.to_string(),
            ),
            (
                "opcache.validate_timestamps",
                flag(self.opcache_validate_timestamps),
            ),
            (
                "opcache.revalidate_freq",
                self.opcache_revalidate_freq.to_string(),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect()
    }
}

/// Raw admin editing is parsed into an allowlist; filesystem and isolation directives stay panel-owned.
pub fn parse_admin_ini(raw: &str) -> Result<BTreeMap<String, String>, CoreError> {
    if raw.len() > 8192 {
        return Err(CoreError::Validation("php.ini maksimal 8 KiB".into()));
    }
    let mut result = BTreeMap::new();
    for line in raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with(';'))
    {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            CoreError::Validation("Gunakan direktif = nilai pada setiap baris".into())
        })?;
        let (key, value) = (key.trim(), value.trim());
        let valid = match key {
            "allow_url_fopen" | "display_startup_errors" => {
                matches!(value, "0" | "1" | "On" | "Off")
            }
            "default_socket_timeout" => value.parse::<u32>().is_ok_and(|n| (1..=300).contains(&n)),
            "error_reporting" => value.parse::<u32>().is_ok_and(|n| n <= 32767),
            "disable_functions" => {
                !value.is_empty()
                    && value.len() <= 1024
                    && value.split(',').all(|f| {
                        !f.is_empty() && f.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    })
            }
            _ => false,
        };
        if !valid || result.insert(key.into(), value.into()).is_some() {
            return Err(CoreError::Validation(format!(
                "Direktif {key} tidak diizinkan, duplikat, atau nilainya tidak valid"
            )));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plan_limits_and_relationships_are_enforced() {
        let limits = PhpSettingsLimits::from_plan(64 * 1_048_576, 8 * 1_048_576);
        let mut settings = PhpSettings::for_limits(&limits);
        assert!(settings.validate(&limits).is_ok());
        settings.memory_limit_mb = 65;
        assert!(settings.validate(&limits).is_err());
        settings.memory_limit_mb = 32;
        settings.post_max_size_mb = 4;
        assert!(settings.validate(&limits).is_err());
    }
    #[test]
    fn injection_unbounded_limits_and_raw_escape_are_rejected() {
        for tz in ["UTC\nuser=root", "../../etc", "UTC;", "${ENV}", "/UTC"] {
            let settings = PhpSettings {
                date_timezone: tz.into(),
                ..Default::default()
            };
            assert!(settings.validate(&PhpSettingsLimits::default()).is_err());
        }
        for ini in [
            "open_basedir=/",
            "extension=/tmp/pwn.so",
            "memory_limit=-1",
            "disable_functions=exec;evil",
            "error_reporting=1\nerror_reporting=2",
            "[global]\nuser=root",
        ] {
            assert!(parse_admin_ini(ini).is_err(), "{ini}");
        }
        assert!(
            parse_admin_ini("; admin\nerror_reporting = 32767\ndefault_socket_timeout=60").is_ok()
        );
    }
}
