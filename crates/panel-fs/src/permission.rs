//! Permission calculation, mode parsing, grid translation, and security warnings.
//!
//! Complies with PRD Section 5.3 & Task P5-T6:
//! - View/change mode with 3x3 check grid (Owner, Group, Others)
//! - Octal input parsing ("0755", "644", "777")
//! - Security warning on 0777 or world-writable modes
//! - Separation of file mode vs directory mode for recursive applications
//! - Safe standard presets (0644 for files, 0755 for directories)

use crate::FsError;
use serde::{Deserialize, Serialize};

/// Standard POSIX file permission bits
pub const S_IRUSR: u32 = 0o400;
pub const S_IWUSR: u32 = 0o200;
pub const S_IXUSR: u32 = 0o100;

pub const S_IRGRP: u32 = 0o040;
pub const S_IWGRP: u32 = 0o020;
pub const S_IXGRP: u32 = 0o010;

pub const S_IROTH: u32 = 0o004;
pub const S_IWOTH: u32 = 0o002;
pub const S_IXOTH: u32 = 0o001;

pub const DEFAULT_FILE_MODE: u32 = 0o644;
pub const DEFAULT_DIR_MODE: u32 = 0o755;

/// Warning message returned when an insecure mode is detected.
pub const PERMISSIVE_777_WARNING: &str = "Peringatan keamanan: Mode 0777 atau world-writable memberikan izin modifikasi kepada semua pengguna sistem. Disarankan menggunakan preset aman (0644 untuk berkas, 0755 untuk direktori).";

/// 3x3 grid representation of POSIX permissions for easy UI mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGrid {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_execute: bool,

    pub group_read: bool,
    pub group_write: bool,
    pub group_execute: bool,

    pub others_read: bool,
    pub others_write: bool,
    pub others_execute: bool,
}

impl PermissionGrid {
    /// Converts a numeric mode (e.g. 0o755) to a PermissionGrid.
    pub fn from_mode(mode: u32) -> Self {
        Self {
            owner_read: (mode & S_IRUSR) != 0,
            owner_write: (mode & S_IWUSR) != 0,
            owner_execute: (mode & S_IXUSR) != 0,

            group_read: (mode & S_IRGRP) != 0,
            group_write: (mode & S_IWGRP) != 0,
            group_execute: (mode & S_IXGRP) != 0,

            others_read: (mode & S_IROTH) != 0,
            others_write: (mode & S_IWOTH) != 0,
            others_execute: (mode & S_IXOTH) != 0,
        }
    }

    /// Converts this grid back into a numeric POSIX mode bits (e.g. 0o755).
    pub fn to_mode(&self) -> u32 {
        let mut mode = 0u32;
        if self.owner_read {
            mode |= S_IRUSR;
        }
        if self.owner_write {
            mode |= S_IWUSR;
        }
        if self.owner_execute {
            mode |= S_IXUSR;
        }

        if self.group_read {
            mode |= S_IRGRP;
        }
        if self.group_write {
            mode |= S_IWGRP;
        }
        if self.group_execute {
            mode |= S_IXGRP;
        }

        if self.others_read {
            mode |= S_IROTH;
        }
        if self.others_write {
            mode |= S_IWOTH;
        }
        if self.others_execute {
            mode |= S_IXOTH;
        }

        mode
    }

    /// Octal string representation (e.g. "0755").
    pub fn to_octal_string(&self) -> String {
        format_octal_mode(self.to_mode())
    }
}

/// Parses an octal mode string (such as "755", "0755", "644", "0644") into a numeric u32.
pub fn parse_octal_mode(input: &str) -> Result<u32, FsError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(FsError::InvalidMode(
            "Mode string tidak boleh kosong".to_string(),
        ));
    }

    // Strip leading 0o or leading zeros if any, but handle "0" specifically
    let raw = trimmed.strip_prefix("0o").unwrap_or(trimmed);
    let val = u32::from_str_radix(raw, 8)
        .map_err(|e| FsError::InvalidMode(format!("Format oktal '{trimmed}' tidak valid: {e}")))?;

    if val > 0o7777 {
        return Err(FsError::InvalidMode(format!(
            "Mode oktal {trimmed} melebihi batas POSIX 07777"
        )));
    }

    Ok(val)
}

/// Formats a mode as a 4-digit octal string, e.g. "0755".
pub fn format_octal_mode(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// Checks if a mode is overly permissive (0777 or world-writable) and returns a warning message.
pub fn check_mode_security_warning(mode: u32) -> Option<String> {
    let masked = mode & 0o777;
    if masked == 0o777 || (mode & S_IWOTH) != 0 {
        Some(PERMISSIVE_777_WARNING.to_string())
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_format_octal_mode() {
        assert_eq!(parse_octal_mode("0755").unwrap(), 0o755);
        assert_eq!(parse_octal_mode("755").unwrap(), 0o755);
        assert_eq!(parse_octal_mode("644").unwrap(), 0o644);
        assert_eq!(parse_octal_mode("0644").unwrap(), 0o644);
        assert_eq!(parse_octal_mode("0o777").unwrap(), 0o777);
        assert_eq!(format_octal_mode(0o755), "0755");
        assert_eq!(format_octal_mode(0o644), "0644");

        // Invalid
        assert!(parse_octal_mode("999").is_err());
        assert!(parse_octal_mode("abc").is_err());
        assert!(parse_octal_mode("177777").is_err());
    }

    #[test]
    fn test_permission_grid_conversion() {
        let grid = PermissionGrid::from_mode(0o755);
        assert!(grid.owner_read && grid.owner_write && grid.owner_execute);
        assert!(grid.group_read && !grid.group_write && grid.group_execute);
        assert!(grid.others_read && !grid.others_write && grid.others_execute);
        assert_eq!(grid.to_mode(), 0o755);
        assert_eq!(grid.to_octal_string(), "0755");

        let grid_file = PermissionGrid::from_mode(0o644);
        assert!(grid_file.owner_read && grid_file.owner_write && !grid_file.owner_execute);
        assert!(grid_file.group_read && !grid_file.group_write && !grid_file.group_execute);
        assert_eq!(grid_file.to_mode(), 0o644);
    }

    #[test]
    fn test_security_warnings_777_and_world_writable() {
        assert!(check_mode_security_warning(0o777).is_some());
        assert!(check_mode_security_warning(0o666).is_some()); // world-writable
        assert!(check_mode_security_warning(0o755).is_none());
        assert!(check_mode_security_warning(0o644).is_none());
    }
}
