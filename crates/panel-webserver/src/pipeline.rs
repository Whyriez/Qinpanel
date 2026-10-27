//! Atomic configuration application, syntax validation, and automatic rollback pipeline.

use crate::error::WebserverError;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Result of an atomic apply operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub site_id: String,
    pub available_path: PathBuf,
    pub enabled_path: Option<PathBuf>,
    pub enabled: bool,
}

/// Debouncer to prevent reload storms when multiple site operations happen concurrently.
#[derive(Debug)]
pub struct ReloadDebouncer {
    debounce_duration: Duration,
    last_reload: Mutex<Option<Instant>>,
}

impl Default for ReloadDebouncer {
    fn default() -> Self {
        Self::new(Duration::from_millis(500))
    }
}

impl ReloadDebouncer {
    pub fn new(debounce_duration: Duration) -> Self {
        Self {
            debounce_duration,
            last_reload: Mutex::new(None),
        }
    }

    /// Checks if a reload should be performed based on the debounce window.
    pub fn should_reload(&self) -> bool {
        let mut lock = match self.last_reload.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let now = Instant::now();
        if (*lock).is_some_and(|last| now.duration_since(last) < self.debounce_duration) {
            return false;
        }
        *lock = Some(now);
        true
    }

    /// Resets the debounce timer.
    pub fn reset(&self) {
        if let Ok(mut lock) = self.last_reload.lock() {
            *lock = None;
        }
    }
}

/// Atomic configuration applicator and rollback engine for Nginx virtual hosts.
#[derive(Debug, Clone)]
pub struct NginxApplyPipeline {
    pub base_dir: PathBuf,
}

impl NginxApplyPipeline {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn sites_available_dir(&self) -> PathBuf {
        self.base_dir.join("sites-available")
    }

    pub fn sites_enabled_dir(&self) -> PathBuf {
        self.base_dir.join("sites-enabled")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.base_dir.join("staging")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.base_dir.join(".backup")
    }

    /// Prepares all required directories.
    pub fn ensure_directories(&self) -> Result<(), WebserverError> {
        fs::create_dir_all(self.sites_available_dir())?;
        fs::create_dir_all(self.sites_enabled_dir())?;
        fs::create_dir_all(self.staging_dir())?;
        fs::create_dir_all(self.backup_dir())?;
        Ok(())
    }

    fn rollback_site_apply(
        &self,
        conf_name: &str,
        had_previous_backup: bool,
        was_previously_enabled: bool,
    ) -> Result<(), WebserverError> {
        let available_conf = self.sites_available_dir().join(conf_name);
        let enabled_conf = self.sites_enabled_dir().join(conf_name);
        let backup_file = self.backup_dir().join(format!("{conf_name}.bak"));

        if had_previous_backup && backup_file.exists() {
            if let Err(e) = fs::copy(&backup_file, &available_conf) {
                return Err(WebserverError::RollbackFailed(format!(
                    "Failed to restore available config: {e}"
                )));
            }
            if was_previously_enabled {
                let _ = create_vhost_link(&available_conf, &enabled_conf);
            } else {
                let _ = remove_link_or_file(&enabled_conf);
            }
            let _ = fs::remove_file(&backup_file);
        } else {
            let _ = remove_link_or_file(&enabled_conf);
            let _ = fs::remove_file(&available_conf);
        }
        Ok(())
    }

    fn stage_and_backup(
        &self,
        conf_name: &str,
        content: &str,
    ) -> Result<(bool, bool), WebserverError> {
        let staging_file = self.staging_dir().join(format!("{conf_name}.tmp"));
        let staged_conf = self.staging_dir().join(conf_name);
        let available_conf = self.sites_available_dir().join(conf_name);
        let enabled_conf = self.sites_enabled_dir().join(conf_name);
        let backup_file = self.backup_dir().join(format!("{conf_name}.bak"));

        fs::write(&staging_file, content)?;
        fs::rename(&staging_file, &staged_conf)?;

        let had_previous_backup = available_conf.exists();
        let was_previously_enabled = enabled_conf.exists() || is_symlink(&enabled_conf);

        if had_previous_backup {
            fs::copy(&available_conf, &backup_file)?;
        }

        fs::copy(&staged_conf, &available_conf)?;
        let _ = fs::remove_file(&staged_conf);

        Ok((had_previous_backup, was_previously_enabled))
    }

    fn validate_and_rollback<F>(
        &self,
        site_id: &str,
        conf_name: &str,
        had_backup: bool,
        was_enabled: bool,
        checker: F,
    ) -> Result<(), WebserverError>
    where
        F: Fn() -> Result<(), String>,
    {
        if let Err(stderr) = checker() {
            error!(
                "Syntax check failed for site '{}'. Triggering automatic rollback. Error: {}",
                site_id, stderr
            );
            self.rollback_site_apply(conf_name, had_backup, was_enabled)?;
            warn!(
                "Automatic rollback completed cleanly for site '{}'",
                site_id
            );
            return Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back: true,
            });
        }
        Ok(())
    }

    /// Applies virtual host configuration atomically with syntax check and automatic rollback.
    pub fn apply_site_config<F>(
        &self,
        site_id: &str,
        content: &str,
        enable: bool,
        syntax_checker: F,
    ) -> Result<ApplyResult, WebserverError>
    where
        F: Fn() -> Result<(), String>,
    {
        self.ensure_directories()?;

        let conf_name = format!("{site_id}.conf");
        let available_conf = self.sites_available_dir().join(&conf_name);
        let enabled_conf = self.sites_enabled_dir().join(&conf_name);
        let backup_file = self.backup_dir().join(format!("{conf_name}.bak"));

        let (had_backup, was_enabled) = self.stage_and_backup(&conf_name, content)?;

        if enable {
            create_vhost_link(&available_conf, &enabled_conf)?;
        } else if was_enabled {
            let _ = remove_link_or_file(&enabled_conf);
        }

        self.validate_and_rollback(site_id, &conf_name, had_backup, was_enabled, syntax_checker)?;

        if had_backup && backup_file.exists() {
            let _ = fs::remove_file(&backup_file);
        }
        info!(
            "Site '{}' configuration applied successfully (enabled: {})",
            site_id, enable
        );
        Ok(ApplyResult {
            site_id: site_id.to_string(),
            available_path: available_conf,
            enabled_path: if enable { Some(enabled_conf) } else { None },
            enabled: enable,
        })
    }

    /// Disables a site by removing its symlink in `sites-enabled/` with automatic rollback on syntax failure.
    pub fn disable_site<F>(
        &self,
        site_id: &str,
        syntax_checker: F,
    ) -> Result<ApplyResult, WebserverError>
    where
        F: Fn() -> Result<(), String>,
    {
        let conf_name = format!("{site_id}.conf");
        let available_conf = self.sites_available_dir().join(&conf_name);
        let enabled_conf = self.sites_enabled_dir().join(&conf_name);

        if !available_conf.exists() {
            return Err(WebserverError::Config(format!(
                "Site '{}' not found in sites-available",
                site_id
            )));
        }

        let was_enabled = enabled_conf.exists() || is_symlink(&enabled_conf);
        if was_enabled {
            remove_link_or_file(&enabled_conf)?;
        }

        if let Err(stderr) = syntax_checker() {
            // Rollback disable
            if was_enabled {
                let _ = create_vhost_link(&available_conf, &enabled_conf);
            }
            return Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back: true,
            });
        }

        Ok(ApplyResult {
            site_id: site_id.to_string(),
            available_path: available_conf,
            enabled_path: None,
            enabled: false,
        })
    }

    /// Enables an existing site in `sites-available/` by creating its symlink in `sites-enabled/` with automatic rollback.
    pub fn enable_site<F>(
        &self,
        site_id: &str,
        syntax_checker: F,
    ) -> Result<ApplyResult, WebserverError>
    where
        F: Fn() -> Result<(), String>,
    {
        let conf_name = format!("{site_id}.conf");
        let available_conf = self.sites_available_dir().join(&conf_name);
        let enabled_conf = self.sites_enabled_dir().join(&conf_name);

        if !available_conf.exists() {
            return Err(WebserverError::Config(format!(
                "Site '{}' not found in sites-available",
                site_id
            )));
        }

        let was_enabled = enabled_conf.exists() || is_symlink(&enabled_conf);
        if !was_enabled {
            create_vhost_link(&available_conf, &enabled_conf)?;
        }

        if let Err(stderr) = syntax_checker() {
            if !was_enabled {
                let _ = remove_link_or_file(&enabled_conf);
            }
            return Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back: true,
            });
        }

        Ok(ApplyResult {
            site_id: site_id.to_string(),
            available_path: available_conf,
            enabled_path: Some(enabled_conf),
            enabled: true,
        })
    }

    /// Removes a site's configurations completely with rollback on failure.
    pub fn remove_site<F>(&self, site_id: &str, syntax_checker: F) -> Result<(), WebserverError>
    where
        F: Fn() -> Result<(), String>,
    {
        let conf_name = format!("{site_id}.conf");
        let available_conf = self.sites_available_dir().join(&conf_name);
        let enabled_conf = self.sites_enabled_dir().join(&conf_name);
        let backup_file = self.backup_dir().join(format!("{conf_name}.del.bak"));

        if !available_conf.exists() {
            return Ok(());
        }

        self.ensure_directories()?;
        fs::copy(&available_conf, &backup_file)?;
        let was_enabled = enabled_conf.exists() || is_symlink(&enabled_conf);

        let _ = remove_link_or_file(&enabled_conf);
        let _ = fs::remove_file(&available_conf);

        if let Err(stderr) = syntax_checker() {
            // Rollback deletion
            let _ = fs::copy(&backup_file, &available_conf);
            if was_enabled {
                let _ = create_vhost_link(&available_conf, &enabled_conf);
            }
            let _ = fs::remove_file(&backup_file);
            return Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back: true,
            });
        }

        let _ = fs::remove_file(&backup_file);
        Ok(())
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn remove_link_or_file(path: &Path) -> Result<(), WebserverError> {
    if path.exists() || is_symlink(path) {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn create_vhost_link(target: &Path, link_path: &Path) -> Result<(), WebserverError> {
    if link_path.exists() || is_symlink(link_path) {
        let _ = fs::remove_file(link_path);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link_path)?;
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(target, link_path) {
            Ok(_) => {}
            Err(_) => {
                // Fallback copy on Windows if developer mode symlink privilege is not enabled
                fs::copy(target, link_path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_apply_new_site_success() {
        let temp = TempDir::new().unwrap();
        let pipeline = NginxApplyPipeline::new(temp.path());

        let res = pipeline
            .apply_site_config("site1", "server { listen 80; }", true, || Ok(()))
            .expect("apply site");

        assert_eq!(res.site_id, "site1");
        assert!(res.enabled);
        assert!(pipeline.sites_available_dir().join("site1.conf").exists());
        assert!(pipeline.sites_enabled_dir().join("site1.conf").exists());
        assert!(!pipeline.backup_dir().join("site1.conf.bak").exists());
    }

    #[test]
    fn test_apply_broken_config_rolls_back_new_site() {
        let temp = TempDir::new().unwrap();
        let pipeline = NginxApplyPipeline::new(temp.path());

        let err =
            pipeline.apply_site_config("broken_new", "broken syntax directive;", true, || {
                Err("nginx: [emerg] unknown directive".to_string())
            });

        match err {
            Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back,
            }) => {
                assert!(rolled_back);
                assert!(stderr.contains("unknown directive"));
            }
            other => panic!("Expected ValidationFailed with rolled_back=true, got {other:?}"),
        }

        // Must NOT leave broken file in available or enabled
        assert!(
            !pipeline
                .sites_available_dir()
                .join("broken_new.conf")
                .exists()
        );
        assert!(
            !pipeline
                .sites_enabled_dir()
                .join("broken_new.conf")
                .exists()
        );
    }

    #[test]
    fn test_apply_broken_config_rolls_back_to_previous_working_config() {
        let temp = TempDir::new().unwrap();
        let pipeline = NginxApplyPipeline::new(temp.path());

        // 1. Initial valid config applied
        let initial_conf = "server { server_name original.com; }";
        pipeline
            .apply_site_config("site_edit", initial_conf, true, || Ok(()))
            .expect("initial apply");

        assert_eq!(
            fs::read_to_string(pipeline.sites_available_dir().join("site_edit.conf")).unwrap(),
            initial_conf
        );

        // 2. Attempt to apply broken edit
        let broken_conf = "server { invalid_directive_breaks_nginx; }";
        let err = pipeline.apply_site_config("site_edit", broken_conf, true, || {
            Err("nginx: [emerg] invalid_directive_breaks_nginx".to_string())
        });

        match err {
            Err(WebserverError::ValidationFailed {
                stderr,
                rolled_back,
            }) => {
                assert!(rolled_back);
                assert!(stderr.contains("invalid_directive_breaks_nginx"));
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }

        // 3. Verify original content was restored by automatic rollback!
        let restored_conf =
            fs::read_to_string(pipeline.sites_available_dir().join("site_edit.conf")).unwrap();
        assert_eq!(
            restored_conf, initial_conf,
            "Automatic rollback must restore original configuration!"
        );
        assert!(pipeline.sites_enabled_dir().join("site_edit.conf").exists());
    }

    #[test]
    fn test_reload_debouncer_coalesces_rapid_calls() {
        let debouncer = ReloadDebouncer::new(Duration::from_millis(50));
        assert!(debouncer.should_reload());
        // Immediate second call should be debounced
        assert!(!debouncer.should_reload());

        std::thread::sleep(Duration::from_millis(60));
        // After window expires, should allow reload again
        assert!(debouncer.should_reload());
    }
}
