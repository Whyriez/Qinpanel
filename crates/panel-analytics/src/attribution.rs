use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Extracts `site_id` from standard Qinpanel Nginx log paths.
///
/// Supported conventions:
/// - `/var/log/panel/sites/site-123/access.log` -> `123`
/// - `/var/log/panel/sites/site_123/access.log` -> `123`
/// - `/var/log/panel/sites/123/access.log` -> `123`
/// - `/var/log/panel/p_alice/site-123/access.log` -> `123`
/// - `/var/log/panel/p_alice/123/access.log` -> `123`
/// - `/var/log/nginx/site-123.access.log` -> `123`
/// - `/var/log/nginx/site-123-access.log` -> `123`
/// - `site-123/access.log` -> `123`
/// - `site_123.access.log` -> `123`
pub fn extract_site_id_from_path(path: &Path) -> Option<i64> {
    // 1. Check parent directory name
    if let Some(parent_str) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    {
        if let Some(id_str) = parent_str
            .strip_prefix("site-")
            .or_else(|| parent_str.strip_prefix("site_"))
            .or_else(|| parent_str.strip_prefix("vhost-"))
            .or_else(|| parent_str.strip_prefix("vhost_"))
        {
            if let Ok(id) = id_str.parse::<i64>() {
                return Some(id);
            }
        } else if let Ok(id) = parent_str.parse::<i64>() {
            return Some(id);
        }
    }

    // 2. Check filename itself
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        let stripped = file_name
            .strip_prefix("site-")
            .or_else(|| file_name.strip_prefix("site_"))
            .or_else(|| file_name.strip_prefix("vhost-"))
            .or_else(|| file_name.strip_prefix("vhost_"));
        if let Some(prefix_stripped) = stripped {
            let id_part = prefix_stripped
                .split(['.', '-', '_'])
                .next()
                .unwrap_or(prefix_stripped);
            if let Ok(id) = id_part.parse::<i64>() {
                return Some(id);
            }
        }
    }

    None
}

/// Dynamic site attribution engine combining filesystem path inspection and hostname mappings.
#[derive(Debug, Default)]
pub struct SiteAttributor {
    path_registry: RwLock<HashMap<PathBuf, i64>>,
    host_registry: RwLock<HashMap<String, i64>>,
}

impl SiteAttributor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an explicit mapping between a log file path and a site ID.
    pub fn register_path(&self, path: impl Into<PathBuf>, site_id: i64) {
        if let Ok(mut lock) = self.path_registry.write() {
            lock.insert(path.into(), site_id);
        }
    }

    /// Register a domain/host to site ID mapping.
    pub fn register_host(&self, host: impl Into<String>, site_id: i64) {
        if let Ok(mut lock) = self.host_registry.write() {
            lock.insert(host.into().to_lowercase(), site_id);
        }
    }

    /// Convenience helper to register a site with domain (and automatic wildcard) and optional path.
    pub fn register_site(
        &self,
        site_id: i64,
        domain: impl Into<String>,
        path: Option<impl Into<PathBuf>>,
    ) {
        let domain_str = domain.into();
        self.register_host(&domain_str, site_id);
        if !domain_str.starts_with("*.") {
            self.register_host(format!("*.{domain_str}"), site_id);
        }
        if let Some(p) = path {
            self.register_path(p, site_id);
        }
    }

    /// Resolve `site_id` using path heuristics, path registry, parent domain folder, or hostname fallback.
    pub fn resolve_site_id(&self, path: &Path, host: Option<&str>) -> Option<i64> {
        // 1. Direct path registry
        if let Some(id) = self
            .path_registry
            .read()
            .ok()
            .and_then(|lock| lock.get(path).copied())
        {
            return Some(id);
        }

        // 2. Path heuristic extraction (site-123, 123)
        if let Some(id) = extract_site_id_from_path(path) {
            return Some(id);
        }

        // 3. Parent directory domain matching (e.g. /var/log/panel/p_alice/myblog.com/access.log)
        if let Some(parent_str) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        {
            let lower = parent_str.to_lowercase();
            if let Some(id) = self
                .host_registry
                .read()
                .ok()
                .and_then(|lock| lock.get(&lower).copied())
            {
                return Some(id);
            }
        }

        // 4. Host parameter fallback (exact or wildcard)
        if let Some(host_str) = host {
            let lower = host_str.to_lowercase();
            let host_clean = lower.split(':').next().unwrap_or(&lower);
            if let Ok(lock) = self.host_registry.read() {
                if let Some(&id) = lock.get(host_clean) {
                    return Some(id);
                }
                for (registered, &id) in lock.iter() {
                    let matches_wildcard = registered.strip_prefix("*.").is_some_and(|suffix| {
                        host_clean.ends_with(suffix) && host_clean.len() > suffix.len()
                    });
                    if matches_wildcard {
                        return Some(id);
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_site_id_from_standard_paths() {
        assert_eq!(
            extract_site_id_from_path(Path::new("/var/log/panel/sites/site-42/access.log")),
            Some(42)
        );
        assert_eq!(
            extract_site_id_from_path(Path::new("/var/log/panel/sites/site_99/access.log")),
            Some(99)
        );
        assert_eq!(
            extract_site_id_from_path(Path::new("/var/log/panel/sites/105/access.log")),
            Some(105)
        );
        assert_eq!(
            extract_site_id_from_path(Path::new("/var/log/nginx/site-7.access.log")),
            Some(7)
        );
        assert_eq!(
            extract_site_id_from_path(Path::new("relative/site-12/access.log")),
            Some(12)
        );
        assert_eq!(
            extract_site_id_from_path(Path::new("/var/log/nginx/access.log")),
            None
        );
    }

    #[test]
    fn test_site_attributor_resolution() {
        let attributor = SiteAttributor::new();
        attributor.register_host("myblog.com", 88);
        attributor.register_path("/custom/path/log.txt", 101);

        // Path heuristic
        assert_eq!(
            attributor.resolve_site_id(Path::new("/var/log/panel/sites/site-42/access.log"), None),
            Some(42)
        );

        // Explicit path registration
        assert_eq!(
            attributor.resolve_site_id(Path::new("/custom/path/log.txt"), None),
            Some(101)
        );

        // Host fallback
        assert_eq!(
            attributor.resolve_site_id(
                Path::new("/var/log/nginx/access.log"),
                Some("myblog.com:443")
            ),
            Some(88)
        );

        // Unresolved
        assert_eq!(
            attributor.resolve_site_id(Path::new("/var/log/nginx/access.log"), Some("unknown.org")),
            None
        );
    }
}
