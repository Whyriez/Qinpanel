//! Explicit Linux support matrix. Never evaluate os-release as shell code.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxPlatform {
    Ubuntu2204,
    Ubuntu2404,
    Debian12,
    AlmaLinux9,
    AlmaLinux10,
}

impl LinuxPlatform {
    pub fn parse_os_release(text: &str) -> Result<Self, &'static str> {
        if text.len() > 16384 {
            return Err("Metadata OS terlalu besar.");
        }
        let mut id = None;
        let mut release = None;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
        {
            let Some((key, raw)) = line.split_once('=') else {
                continue;
            };
            let raw = raw.trim();
            let value = if let Some(quote) = raw
                .chars()
                .next()
                .filter(|value| matches!(value, '\'' | '"'))
            {
                raw.strip_prefix(quote)
                    .and_then(|value| value.strip_suffix(quote))
                    .ok_or("Tanda kutip identitas OS tidak lengkap.")?
            } else {
                raw
            };
            let field = match key.trim() {
                "ID" => &mut id,
                "VERSION_ID" => &mut release,
                _ => continue,
            };
            if field.replace(value).is_some() {
                return Err("Identitas OS duplikat.");
            }
        }
        match (id, release) {
            (Some("ubuntu"), Some("22.04")) => Ok(Self::Ubuntu2204),
            (Some("ubuntu"), Some("24.04")) => Ok(Self::Ubuntu2404),
            (Some("debian"), Some("12")) => Ok(Self::Debian12),
            (Some("almalinux"), Some(value))
                if value
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) =>
            {
                match value.split('.').next() {
                    Some("9") => Ok(Self::AlmaLinux9),
                    Some("10") => Ok(Self::AlmaLinux10),
                    _ => Err("Versi AlmaLinux belum didukung."),
                }
            }
            _ => Err("OS didukung: Debian 12, Ubuntu 22.04/24.04, AlmaLinux 9/10."),
        }
    }

    pub const fn is_rpm(self) -> bool {
        matches!(self, Self::AlmaLinux9 | Self::AlmaLinux10)
    }
    pub const fn release_major(self) -> &'static str {
        match self {
            Self::AlmaLinux9 => "9",
            Self::AlmaLinux10 => "10",
            Self::Debian12 => "12",
            Self::Ubuntu2204 => "22",
            Self::Ubuntu2404 => "24",
        }
    }
    pub const fn backup_identity(self) -> &'static str {
        match self {
            Self::Ubuntu2204 => "ubuntu:22.04",
            Self::Ubuntu2404 => "ubuntu:24.04",
            Self::Debian12 => "debian:12",
            Self::AlmaLinux9 => "almalinux:9",
            Self::AlmaLinux10 => "almalinux:10",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_distribution_matrix_and_minor_versions() {
        for (id, release, expected) in [
            ("ubuntu", "22.04", LinuxPlatform::Ubuntu2204),
            ("ubuntu", "24.04", LinuxPlatform::Ubuntu2404),
            ("debian", "12", LinuxPlatform::Debian12),
            ("almalinux", "9.7", LinuxPlatform::AlmaLinux9),
            ("almalinux", "10.2", LinuxPlatform::AlmaLinux10),
        ] {
            assert_eq!(
                LinuxPlatform::parse_os_release(&format!(
                    "ID=\"{id}\"\nVERSION_ID='{release}'\nID_LIKE=rhel fedora\n"
                )),
                Ok(expected)
            );
        }
    }
    #[test]
    fn unknown_clones_versions_duplicates_and_shell_payloads_are_rejected() {
        for text in [
            "ID=rocky\nVERSION_ID=9",
            "ID=ubuntu\nVERSION_ID=26.04",
            "ID=almalinux\nVERSION_ID=8.10",
            "ID=almalinux\nVERSION_ID=9;id",
            "ID=debian\nID=ubuntu\nVERSION_ID=12",
            "ID=$(id)\nVERSION_ID=12",
        ] {
            assert!(LinuxPlatform::parse_os_release(text).is_err());
        }
    }
}
