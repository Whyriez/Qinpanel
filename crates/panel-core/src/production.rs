//! Shared production defaults. Local development must opt into its own database.

pub const DEFAULT_DATABASE_PATH: &str = "/var/lib/panel/panel.db";
pub const INTERNAL_API_PORT: u16 = 7700;
pub const PUBLIC_PANEL_PORT: u16 = 17890;
pub const SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Resolve system utilities independently of an interactive user's PATH.
pub fn system_command(binary: &str) -> std::process::Command {
    #[cfg(unix)]
    {
        let candidate = SYSTEM_PATH
            .split(':')
            .map(|dir| std::path::Path::new(dir).join(binary))
            .find(|path| path.is_file());
        let mut command = std::process::Command::new(
            candidate.unwrap_or_else(|| std::path::PathBuf::from(binary)),
        );
        command.env("PATH", SYSTEM_PATH);
        command
    }
    #[cfg(not(unix))]
    std::process::Command::new(binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn system_command_uses_canonical_search_directories() {
        let command = system_command("nginx");
        assert!(command.get_envs().any(
            |(name, value)| name == "PATH" && value == Some(std::ffi::OsStr::new(SYSTEM_PATH))
        ));
        let expected = SYSTEM_PATH
            .split(':')
            .map(|dir| std::path::Path::new(dir).join("nginx"))
            .find(|path| path.is_file());
        if let Some(path) = expected {
            assert_eq!(command.get_program(), path.as_os_str());
        }
    }

    #[test]
    fn production_defaults_are_absolute_and_separate_public_from_internal() {
        assert!(DEFAULT_DATABASE_PATH.starts_with('/'));
        assert_eq!(INTERNAL_API_PORT, 7700);
        assert_eq!(PUBLIC_PANEL_PORT, 17890);
        assert_ne!(INTERNAL_API_PORT, PUBLIC_PANEL_PORT);
        assert!(SYSTEM_PATH.split(':').any(|dir| dir == "/usr/sbin"));
    }
}
