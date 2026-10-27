//! Domain models and validation logic for structured site rules (P8-T3).
//!
//! Includes redirects, custom headers, CORS, hotlink protection, basic auth,
//! IP allow/deny, maintenance mode, custom error pages, and admin Nginx snippets.

use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;

/// Match type for structured redirects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RedirectMatchType {
    #[default]
    Exact,
    Prefix,
    Regex,
}

impl RedirectMatchType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Prefix => "prefix",
            Self::Regex => "regex",
        }
    }
}

/// A structured path- or pattern-based redirect rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredRedirect {
    pub id: String,
    pub source: String,
    pub target: String,
    pub status_code: u16,
    pub match_type: RedirectMatchType,
    pub enabled: bool,
}

/// Custom HTTP header configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomHeader {
    pub id: String,
    pub name: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub always: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Curated security headers configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecurityHeadersConfig {
    pub hsts_enabled: bool,
    pub hsts_subdomains: bool,
    pub hsts_preload: bool,
    /// X-Frame-Options: "DENY", "SAMEORIGIN", or empty/disabled.
    pub x_frame_options: Option<String>,
    pub x_content_type_options: bool,
    pub referrer_policy: Option<String>,
    pub permissions_policy: Option<String>,
    pub csp: Option<String>,
}

/// Preset modes for Cross-Origin Resource Sharing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CorsPreset {
    #[default]
    Disabled,
    AllowAll,
    SameOrigin,
    Custom,
}

/// CORS configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorsConfig {
    pub enabled: bool,
    pub preset: CorsPreset,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub allowed_headers: Vec<String>,
    #[serde(default)]
    pub exposed_headers: Vec<String>,
    #[serde(default)]
    pub allow_credentials: bool,
    #[serde(default = "default_cors_max_age")]
    pub max_age_secs: u32,
}

fn default_cors_max_age() -> u32 {
    86400
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: CorsPreset::Disabled,
            allowed_origins: vec![],
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "OPTIONS".to_string(),
                "PATCH".to_string(),
            ],
            allowed_headers: vec![
                "Content-Type".to_string(),
                "Authorization".to_string(),
                "X-Requested-With".to_string(),
            ],
            exposed_headers: vec![],
            allow_credentials: true,
            max_age_secs: default_cors_max_age(),
        }
    }
}

/// Action to perform when an invalid referer attempts hotlinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HotlinkAction {
    #[default]
    Deny,
    Redirect,
}

/// Hotlink protection configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotlinkConfig {
    pub enabled: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default = "default_hotlink_extensions")]
    pub file_extensions: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_empty_referer: bool,
    #[serde(default)]
    pub action: HotlinkAction,
    #[serde(default)]
    pub redirect_url: Option<String>,
}

fn default_hotlink_extensions() -> Vec<String> {
    vec![
        "jpg".to_string(),
        "jpeg".to_string(),
        "png".to_string(),
        "gif".to_string(),
        "webp".to_string(),
        "svg".to_string(),
        "ico".to_string(),
        "mp4".to_string(),
        "webm".to_string(),
    ]
}

impl Default for HotlinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_domains: vec![],
            file_extensions: default_hotlink_extensions(),
            allow_empty_referer: true,
            action: HotlinkAction::Deny,
            redirect_url: None,
        }
    }
}

/// A basic auth user credentials pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicAuthUser {
    pub username: String,
    pub password_hash: String,
}

/// HTTP Basic Authentication configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BasicAuthConfig {
    pub enabled: bool,
    #[serde(default = "default_root_path")]
    pub path: String,
    #[serde(default = "default_realm")]
    pub realm: String,
    #[serde(default)]
    pub users: Vec<BasicAuthUser>,
}

fn default_root_path() -> String {
    "/".to_string()
}

fn default_realm() -> String {
    "Area Terbatas".to_string()
}

/// Action for an IP access rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IpAction {
    #[default]
    Allow,
    Deny,
}

/// A single IP allow or deny entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAccessRule {
    pub id: String,
    pub action: IpAction,
    pub cidr_or_ip: String,
    pub description: Option<String>,
}

/// IP Access Control configuration (whitelist or blacklist mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAccessConfig {
    pub enabled: bool,
    /// "global" or "path"
    #[serde(default = "default_global_scope")]
    pub scope: String,
    pub path: Option<String>,
    /// Default action when no rules match: "allow" (blacklist mode) or "deny" (whitelist mode).
    #[serde(default = "default_ip_action")]
    pub default_action: IpAction,
    #[serde(default)]
    pub rules: Vec<IpAccessRule>,
}

fn default_global_scope() -> String {
    "global".to_string()
}

fn default_ip_action() -> IpAction {
    IpAction::Allow
}

impl Default for IpAccessConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scope: default_global_scope(),
            path: None,
            default_action: IpAction::Allow,
            rules: vec![],
        }
    }
}

/// Maintenance mode configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceConfig {
    pub enabled: bool,
    #[serde(default)]
    pub bypass_ips: Vec<String>,
    #[serde(default = "default_retry_after")]
    pub retry_after_secs: u32,
    #[serde(default = "default_maintenance_message")]
    pub custom_message: String,
}

fn default_retry_after() -> u32 {
    3600
}

fn default_maintenance_message() -> String {
    "Situs sedang dalam pemeliharaan terjadwal. Silakan coba kembali beberapa saat lagi."
        .to_string()
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bypass_ips: vec![],
            retry_after_secs: default_retry_after(),
            custom_message: default_maintenance_message(),
        }
    }
}

/// A custom error page entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomErrorPage {
    pub status_code: u16,
    pub target: String,
}

/// Raw Nginx snippet configuration (admin-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NginxSnippetConfig {
    pub enabled: bool,
    pub content: String,
}

/// Unified site rules configuration container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SiteRulesConfig {
    #[serde(default)]
    pub redirects: Vec<StructuredRedirect>,
    #[serde(default)]
    pub custom_headers: Vec<CustomHeader>,
    #[serde(default)]
    pub security_headers: SecurityHeadersConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub hotlink_protection: HotlinkConfig,
    #[serde(default)]
    pub basic_auth: BasicAuthConfig,
    #[serde(default)]
    pub ip_access: IpAccessConfig,
    #[serde(default)]
    pub maintenance: MaintenanceConfig,
    #[serde(default)]
    pub custom_error_pages: Vec<CustomErrorPage>,
    #[serde(default)]
    pub nginx_snippet: NginxSnippetConfig,
}

// =========================================================================
// Validation Helpers
// =========================================================================

/// Validates a single structured redirect rule.
pub fn validate_redirect_rule(rule: &StructuredRedirect) -> Result<(), CoreError> {
    let source = rule.source.trim();
    let target = rule.target.trim();

    if source.is_empty() {
        return Err(CoreError::Validation(
            "Path asal redirect (source) tidak boleh kosong".to_string(),
        ));
    }
    if target.is_empty() {
        return Err(CoreError::Validation(
            "Tujuan redirect (target) tidak boleh kosong".to_string(),
        ));
    }

    if source.contains(['\r', '\n', '\0']) || target.contains(['\r', '\n', '\0']) {
        return Err(CoreError::Validation(
            "Redirect tidak boleh mengandung karakter escape atau baris baru".to_string(),
        ));
    }

    if rule.status_code != 301 && rule.status_code != 302 {
        return Err(CoreError::Validation(
            "Kode status redirect harus 301 (Permanent) atau 302 (Temporary)".to_string(),
        ));
    }

    if source == target {
        return Err(CoreError::Validation(
            "Sumber dan target redirect tidak boleh identik (mencegah loop redirect)".to_string(),
        ));
    }

    if rule.match_type == RedirectMatchType::Regex {
        validate_regex_pattern(source)?;
    } else if !source.starts_with('/') {
        return Err(CoreError::Validation(
            "Path sumber redirect non-regex harus diawali tanda garis miring '/'".to_string(),
        ));
    }

    Ok(())
}

/// Validates basic regex pattern syntax (balanced parentheses, brackets, etc.).
pub fn validate_regex_pattern(pattern: &str) -> Result<(), CoreError> {
    if pattern.is_empty() {
        return Err(CoreError::Validation(
            "Pola regex tidak boleh kosong".to_string(),
        ));
    }
    let mut paren_count = 0i32;
    let mut bracket_count = 0i32;
    let mut prev_char = '\0';

    for c in pattern.chars() {
        if prev_char != '\\' {
            match c {
                '(' => paren_count += 1,
                ')' => {
                    paren_count -= 1;
                    if paren_count < 0 {
                        return Err(CoreError::Validation(
                            "Tanda kurung ')' tidak berimbang dalam regex".to_string(),
                        ));
                    }
                }
                '[' => bracket_count += 1,
                ']' => {
                    bracket_count -= 1;
                    if bracket_count < 0 {
                        return Err(CoreError::Validation(
                            "Tanda kurung siku ']' tidak berimbang dalam regex".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }
        prev_char = c;
    }

    if paren_count != 0 {
        return Err(CoreError::Validation(
            "Tanda kurung '(' tidak tertutup dalam regex".to_string(),
        ));
    }
    if bracket_count != 0 {
        return Err(CoreError::Validation(
            "Tanda kurung siku '[' tidak tertutup dalam regex".to_string(),
        ));
    }

    Ok(())
}

/// Validates an HTTP header name.
pub fn validate_header_name(name: &str) -> Result<(), CoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Nama header tidak boleh kosong".to_string(),
        ));
    }

    let is_valid = trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !is_valid {
        return Err(CoreError::Validation(format!(
            "Nama header '{trimmed}' tidak valid (hanya karakter alfanumerik, tanda hubung, dan underscore)"
        )));
    }

    Ok(())
}

/// Validates an HTTP header value to prevent header injection.
pub fn validate_header_value(val: &str) -> Result<(), CoreError> {
    if val.contains(['\r', '\n', '\0']) {
        return Err(CoreError::Validation(
            "Nilai header tidak boleh mengandung karakter baris baru atau null byte".to_string(),
        ));
    }
    Ok(())
}

/// Validates an IPv4 / IPv6 address or CIDR notation.
pub fn validate_ip_or_cidr(val: &str) -> Result<(), CoreError> {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Alamat IP atau CIDR tidak boleh kosong".to_string(),
        ));
    }

    if let Some((ip_part, mask_part)) = trimmed.split_once('/') {
        let ip = IpAddr::from_str(ip_part)
            .map_err(|_| CoreError::Validation(format!("Format IP '{ip_part}' tidak valid")))?;
        let mask = mask_part
            .parse::<u8>()
            .map_err(|_| CoreError::Validation(format!("Subnet mask '{mask_part}' tidak valid")))?;

        match ip {
            IpAddr::V4(_) => {
                if mask > 32 {
                    return Err(CoreError::Validation(
                        "Subnet mask IPv4 maksimal 32".to_string(),
                    ));
                }
            }
            IpAddr::V6(_) => {
                if mask > 128 {
                    return Err(CoreError::Validation(
                        "Subnet mask IPv6 maksimal 128".to_string(),
                    ));
                }
            }
        }
    } else {
        IpAddr::from_str(trimmed)
            .map_err(|_| CoreError::Validation(format!("Format IP '{trimmed}' tidak valid")))?;
    }

    Ok(())
}

/// Forbidden directives in raw Nginx snippets to prevent breaking container/host isolation.
const FORBIDDEN_SNIPPET_DIRECTIVES: &[&str] = &[
    "user",
    "worker_processes",
    "worker_connections",
    "pid",
    "load_module",
    "include",
    "alias /",
    "root /",
    "access_log /dev",
    "error_log /dev",
    "ssl_certificate_key /etc",
    "perl_",
    "lua_",
    "exec",
    "system",
    "http {",
    "events {",
    "stream {",
    "169.254.",
];

/// Validates raw Nginx snippet for forbidden / host-compromising directives.
pub fn validate_nginx_snippet(content: &str) -> Result<(), CoreError> {
    let lower = content.to_lowercase();
    for forbidden in FORBIDDEN_SNIPPET_DIRECTIVES {
        if lower.contains(forbidden) {
            return Err(CoreError::Validation(format!(
                "Direktif Nginx '{forbidden}' dilarang demi keamanan isolasi server"
            )));
        }
    }
    Ok(())
}

/// Comprehensive validator for the entire `SiteRulesConfig`.
pub fn validate_site_rules(rules: &SiteRulesConfig) -> Result<(), CoreError> {
    // 1. Redirects
    for r in &rules.redirects {
        if r.enabled {
            validate_redirect_rule(r)?;
        }
    }

    // 2. Custom Headers
    for h in &rules.custom_headers {
        if h.enabled {
            validate_header_name(&h.name)?;
            validate_header_value(&h.value)?;
        }
    }

    // 3. CORS
    if rules.cors.enabled {
        for origin in &rules.cors.allowed_origins {
            if origin.contains(['\r', '\n', '\0']) {
                return Err(CoreError::Validation(
                    "Origin CORS mengandung karakter baris baru tidak valid".to_string(),
                ));
            }
        }
    }

    // 4. Hotlink
    if rules.hotlink_protection.enabled {
        for d in &rules.hotlink_protection.allowed_domains {
            if d.contains(['\r', '\n', '\0', ' ', ';']) {
                return Err(CoreError::Validation(format!(
                    "Domain hotlink protection '{d}' tidak valid"
                )));
            }
        }
        if rules
            .hotlink_protection
            .redirect_url
            .as_deref()
            .is_some_and(|r| r.contains(['\r', '\n', '\0']))
        {
            return Err(CoreError::Validation(
                "URL pengalihan hotlink tidak valid".to_string(),
            ));
        }
    }

    // 5. Basic Auth
    if rules.basic_auth.enabled {
        if rules.basic_auth.realm.contains(['\r', '\n', '"']) {
            return Err(CoreError::Validation(
                "Realm Basic Auth tidak boleh mengandung tanda kutip atau baris baru".to_string(),
            ));
        }
        for u in &rules.basic_auth.users {
            if u.username.trim().is_empty() || u.username.contains([':', '\r', '\n']) {
                return Err(CoreError::Validation(format!(
                    "Username basic auth '{}' tidak valid",
                    u.username
                )));
            }
            if u.password_hash.trim().is_empty() {
                return Err(CoreError::Validation(format!(
                    "Hash password untuk user '{}' tidak boleh kosong",
                    u.username
                )));
            }
        }
    }

    // 6. IP Access
    if rules.ip_access.enabled {
        for r in &rules.ip_access.rules {
            validate_ip_or_cidr(&r.cidr_or_ip)?;
        }
    }

    // 7. Maintenance
    if rules.maintenance.enabled {
        for ip in &rules.maintenance.bypass_ips {
            validate_ip_or_cidr(ip)?;
        }
        if rules.maintenance.custom_message.contains('\0') {
            return Err(CoreError::Validation(
                "Pesan maintenance tidak boleh mengandung null byte".to_string(),
            ));
        }
    }

    // 8. Custom Error Pages
    for p in &rules.custom_error_pages {
        if p.status_code < 400 || p.status_code > 599 {
            return Err(CoreError::Validation(format!(
                "Kode status error page {} tidak valid (harus 400-599)",
                p.status_code
            )));
        }
        if p.target.trim().is_empty() || p.target.contains(['\r', '\n', '\0']) {
            return Err(CoreError::Validation(format!(
                "Target error page untuk kode {} tidak valid",
                p.status_code
            )));
        }
    }

    // 9. Nginx Snippet
    if rules.nginx_snippet.enabled {
        validate_nginx_snippet(&rules.nginx_snippet.content)?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_redirect_rule_success_and_errors() {
        let valid_exact = StructuredRedirect {
            id: "r1".to_string(),
            source: "/old".to_string(),
            target: "/new".to_string(),
            status_code: 301,
            match_type: RedirectMatchType::Exact,
            enabled: true,
        };
        assert!(validate_redirect_rule(&valid_exact).is_ok());

        let loop_redirect = StructuredRedirect {
            id: "r2".to_string(),
            source: "/same".to_string(),
            target: "/same".to_string(),
            status_code: 301,
            match_type: RedirectMatchType::Exact,
            enabled: true,
        };
        assert!(validate_redirect_rule(&loop_redirect).is_err());

        let bad_code = StructuredRedirect {
            id: "r3".to_string(),
            source: "/old".to_string(),
            target: "/new".to_string(),
            status_code: 200,
            match_type: RedirectMatchType::Exact,
            enabled: true,
        };
        assert!(validate_redirect_rule(&bad_code).is_err());

        let valid_regex = StructuredRedirect {
            id: "r4".to_string(),
            source: "^/post/([0-9]+)$".to_string(),
            target: "/article/$1".to_string(),
            status_code: 302,
            match_type: RedirectMatchType::Regex,
            enabled: true,
        };
        assert!(validate_redirect_rule(&valid_regex).is_ok());

        let invalid_regex = StructuredRedirect {
            id: "r5".to_string(),
            source: "^/post/([0-9+$".to_string(),
            target: "/article/$1".to_string(),
            status_code: 302,
            match_type: RedirectMatchType::Regex,
            enabled: true,
        };
        assert!(validate_redirect_rule(&invalid_regex).is_err());
    }

    #[test]
    fn test_validate_ip_or_cidr() {
        assert!(validate_ip_or_cidr("192.168.1.1").is_ok());
        assert!(validate_ip_or_cidr("10.0.0.0/8").is_ok());
        assert!(validate_ip_or_cidr("::1").is_ok());
        assert!(validate_ip_or_cidr("2001:db8::/32").is_ok());

        assert!(validate_ip_or_cidr("999.999.999.999").is_err());
        assert!(validate_ip_or_cidr("10.0.0.0/33").is_err());
        assert!(validate_ip_or_cidr("not-an-ip").is_err());
    }

    #[test]
    fn test_validate_nginx_snippet_security() {
        assert!(validate_nginx_snippet("gzip on; client_max_body_size 128m;").is_ok());
        assert!(validate_nginx_snippet("user root;").is_err());
        assert!(validate_nginx_snippet("root /;").is_err());
        assert!(validate_nginx_snippet("include /etc/shadow;").is_err());
        assert!(validate_nginx_snippet("access_log /dev/null;").is_err());
    }
}
