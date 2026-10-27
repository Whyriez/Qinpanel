//! GeoIP country resolver supporting MaxMind DB-IP / GeoLite2 `.mmdb` files.
//!
//! Provides fast ISO-3166-1 alpha-2 country code lookup (e.g., "ID", "US", "SG", "DE")
//! with automatic private IP ("LAN") and loopback ("LOCAL") identification.

use crate::error::AnalyticsError;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

/// Thread-safe GeoIP resolver.
#[derive(Clone)]
pub struct GeoIpResolver {
    reader: Option<Arc<maxminddb::Reader<Vec<u8>>>>,
    overrides: Arc<RwLock<HashMap<String, String>>>,
    db_path: Option<PathBuf>,
}

impl Default for GeoIpResolver {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl GeoIpResolver {
    /// Create an empty resolver without a backing .mmdb file (local/LAN/overrides only).
    pub fn new_empty() -> Self {
        Self {
            reader: None,
            overrides: Arc::new(RwLock::new(HashMap::new())),
            db_path: None,
        }
    }

    /// Open and initialize from a MaxMind GeoIP / DB-IP Lite country database file (.mmdb).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AnalyticsError> {
        let p = path.as_ref();
        if !p.exists() {
            return Err(AnalyticsError::Internal(format!(
                "GeoIP database file not found: {}",
                p.display()
            )));
        }

        let reader = maxminddb::Reader::open_readfile(p).map_err(|e| {
            AnalyticsError::Internal(format!("Failed to open GeoIP mmdb database: {e}"))
        })?;

        Ok(Self {
            reader: Some(Arc::new(reader)),
            overrides: Arc::new(RwLock::new(HashMap::new())),
            db_path: Some(p.to_path_buf()),
        })
    }

    /// Try to locate default system or panel GeoIP databases, or return empty resolver if none found.
    pub fn detect_or_empty() -> Self {
        let candidates = [
            PathBuf::from("/var/lib/panel/geoip/country.mmdb"),
            PathBuf::from("/usr/share/GeoIP/GeoLite2-Country.mmdb"),
            PathBuf::from("/usr/share/GeoIP/dbip-country-lite.mmdb"),
        ];

        for path in &candidates {
            if let Ok(resolver) = Self::open(path) {
                return resolver;
            }
        }

        Self::new_empty()
    }

    /// Register a custom IP to country code override for tests or internal subnet mappings.
    pub fn register_override(&self, ip: impl Into<String>, country_code: impl Into<String>) {
        if let Ok(mut lock) = self.overrides.write() {
            lock.insert(ip.into(), country_code.into().to_uppercase());
        }
    }

    /// Lookup ISO country code for an IP string. Returns e.g. "ID", "US", "SG", "LOCAL", "LAN", or "UNKNOWN".
    pub fn lookup_country(&self, ip_str: &str) -> String {
        let trimmed = ip_str.trim();

        // 1. Check overrides
        if let Some(c) = self
            .overrides
            .read()
            .ok()
            .and_then(|lock| lock.get(trimmed).cloned())
        {
            return c;
        }

        // 2. Parse IP
        let ip = match IpAddr::from_str(trimmed) {
            Ok(parsed) => parsed,
            Err(_) => return "UNKNOWN".to_string(),
        };

        // 3. Loopback & Private subnets
        if ip.is_loopback() {
            return "LOCAL".to_string();
        }

        match ip {
            IpAddr::V4(v4) => {
                if v4.is_private() || v4.is_link_local() {
                    return "LAN".to_string();
                }
            }
            IpAddr::V6(v6) => {
                let segments = v6.segments();
                // fc00::/7 (unique local) or fe80::/10 (link local)
                if (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80 {
                    return "LAN".to_string();
                }
            }
        }

        // 4. MaxMind MMDB lookup
        if let Some(reader) = &self.reader {
            let maybe_iso = reader
                .lookup(ip)
                .ok()
                .and_then(|res| res.decode::<maxminddb::geoip2::Country>().ok().flatten())
                .and_then(|data| data.country.iso_code)
                .map(|code| code.to_string());
            if let Some(iso) = maybe_iso {
                return iso;
            }
        }

        "UNKNOWN".to_string()
    }

    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geoip_loopback_and_lan() {
        let resolver = GeoIpResolver::new_empty();

        assert_eq!(resolver.lookup_country("127.0.0.1"), "LOCAL");
        assert_eq!(resolver.lookup_country("::1"), "LOCAL");
        assert_eq!(resolver.lookup_country("192.168.1.100"), "LAN");
        assert_eq!(resolver.lookup_country("10.0.4.5"), "LAN");
        assert_eq!(resolver.lookup_country("172.16.50.1"), "LAN");
        assert_eq!(resolver.lookup_country("invalid-ip"), "UNKNOWN");
    }

    #[test]
    fn test_geoip_overrides() {
        let resolver = GeoIpResolver::new_empty();
        resolver.register_override("203.0.113.195", "ID");
        resolver.register_override("1.1.1.1", "US");

        assert_eq!(resolver.lookup_country("203.0.113.195"), "ID");
        assert_eq!(resolver.lookup_country("1.1.1.1"), "US");
        assert_eq!(resolver.lookup_country("8.8.8.8"), "UNKNOWN");
    }
}
