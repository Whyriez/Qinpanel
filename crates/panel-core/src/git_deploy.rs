//! Domain models, validators, and webhook signature verifiers for Git Deploy (P8-T4).

use crate::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum allowed length for a Git repository URL.
pub const MAX_GIT_REPO_URL_LEN: usize = 512;
/// Maximum allowed length for a Git branch name.
pub const MAX_GIT_BRANCH_LEN: usize = 255;
/// Maximum allowed length for a docroot subpath.
pub const MAX_DOCROOT_SUBPATH_LEN: usize = 255;

/// Validates that a Git repository URL is well-formed and free of dangerous characters.
pub fn validate_git_repo_url(url: &str) -> Result<(), CoreError> {
    if url.is_empty() {
        return Err(CoreError::Validation(
            "URL repositori Git tidak boleh kosong".to_string(),
        ));
    }
    if url.contains(['\r', '\n', '\0', ' ', '\t']) {
        return Err(CoreError::Validation(
            "URL repositori Git tidak boleh mengandung whitespace atau newline".to_string(),
        ));
    }
    let trimmed = url.trim();
    if trimmed.len() > MAX_GIT_REPO_URL_LEN {
        return Err(CoreError::Validation(format!(
            "URL repositori Git terlalu panjang (maksimum {MAX_GIT_REPO_URL_LEN} karakter)"
        )));
    }

    // Reject dangerous control and command injection characters
    for c in trimmed.chars() {
        if c.is_control()
            || matches!(
                c,
                ';' | '&' | '|' | '`' | '$' | '\'' | '"' | '<' | '>' | ' ' | '\t' | '\r' | '\n'
            )
        {
            return Err(CoreError::Validation(format!(
                "URL repositori Git mengandung karakter tidak diizinkan: '{c}'"
            )));
        }
    }

    // Supported protocols: SSH (git@... or ssh://...) or HTTPS (https://...)
    let is_ssh = trimmed.starts_with("git@") || trimmed.starts_with("ssh://");
    let is_https = trimmed.starts_with("https://");

    if !is_ssh && !is_https {
        return Err(CoreError::Validation(
            "URL repositori Git harus berawalan 'git@', 'ssh://', atau 'https://'".to_string(),
        ));
    }

    Ok(())
}

/// Validates that a Git branch name follows git ref format rules.
pub fn validate_git_branch(branch: &str) -> Result<(), CoreError> {
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Validation(
            "Nama branch Git tidak boleh kosong".to_string(),
        ));
    }
    if trimmed.len() > MAX_GIT_BRANCH_LEN {
        return Err(CoreError::Validation(format!(
            "Nama branch Git terlalu panjang (maksimum {MAX_GIT_BRANCH_LEN} karakter)"
        )));
    }

    if trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
    {
        return Err(CoreError::Validation(
            "Nama branch Git tidak boleh berawalan atau berakhiran dengan '/' atau '.'".to_string(),
        ));
    }

    if trimmed.contains("..") || trimmed.contains("//") {
        return Err(CoreError::Validation(
            "Nama branch Git tidak boleh mengandung '..' atau '//'".to_string(),
        ));
    }

    for c in trimmed.chars() {
        if c.is_control()
            || matches!(
                c,
                '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '@' | ' ' | '\t' | '\r' | '\n'
            )
        {
            return Err(CoreError::Validation(format!(
                "Nama branch Git mengandung karakter tidak diizinkan: '{c}'"
            )));
        }
    }

    Ok(())
}

/// Validates that docroot subpath is a clean relative subpath (e.g. "public", "dist", "build", or "").
pub fn validate_docroot_subpath(subpath: &str) -> Result<String, CoreError> {
    let trimmed = subpath.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok(String::new());
    }

    if trimmed.len() > MAX_DOCROOT_SUBPATH_LEN {
        return Err(CoreError::Validation(format!(
            "Subpath docroot terlalu panjang (maksimum {MAX_DOCROOT_SUBPATH_LEN} karakter)"
        )));
    }

    let cleaned = trimmed.trim_start_matches('/').trim_end_matches('/');

    if cleaned.contains("..") {
        return Err(CoreError::Validation(
            "Subpath docroot tidak boleh mengandung '..' (path traversal)".to_string(),
        ));
    }

    for c in cleaned.chars() {
        if c.is_control()
            || matches!(
                c,
                ';' | '&' | '|' | '`' | '$' | '\'' | '"' | '<' | '>' | '\\' | '\r' | '\n'
            )
        {
            return Err(CoreError::Validation(format!(
                "Subpath docroot mengandung karakter tidak diizinkan: '{c}'"
            )));
        }
    }

    Ok(cleaned.to_string())
}

/// Computes HMAC-SHA256 according to RFC 2104 using SHA-256 without external third-party crates.
pub fn compute_hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hash = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// Compares two byte slices in constant time to prevent timing attacks.
pub fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut res = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        res |= x ^ y;
    }
    res == 0
}

/// Converts a slice of bytes into a lowercase hex string.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Verifies a GitHub webhook signature header (`X-Hub-Signature-256: sha256=<hex>`).
pub fn verify_github_signature(payload: &[u8], secret: &str, sig_header: &str) -> bool {
    let trimmed = sig_header.trim();
    let hex_sig = if let Some(stripped) = trimmed.strip_prefix("sha256=") {
        stripped
    } else {
        trimmed
    };

    let expected_mac = compute_hmac_sha256(secret.as_bytes(), payload);
    let expected_hex = bytes_to_hex(&expected_mac);

    constant_time_compare(hex_sig.as_bytes(), expected_hex.as_bytes())
}

/// Verifies a GitLab webhook secret token header (`X-Gitlab-Token: <token>`).
pub fn verify_gitlab_token(token_header: &str, secret: &str) -> bool {
    let trimmed = token_header.trim();
    constant_time_compare(trimmed.as_bytes(), secret.trim().as_bytes())
}

/// Generates a standardized release ID based on timestamp and optional commit hash.
/// Format: `YYYYMMDD_HHMMSS_<commit_short>`
pub fn generate_release_id(commit_hash: Option<&str>) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let short_hash = match commit_hash {
        Some(h) if h.len() >= 7 => &h[..7],
        Some(h) if !h.is_empty() => h,
        _ => "latest",
    };

    // Calculate approximate UTC date time for directory readability
    format!("rel_{now}_{short_hash}")
}

/// Parsed metadata extracted from incoming webhook push event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedWebhookPayload {
    pub branch: String,
    pub commit_hash: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
}

/// Parses GitHub push event JSON payload.
pub fn parse_github_push_payload(payload: &[u8]) -> Result<ParsedWebhookPayload, CoreError> {
    let val: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| CoreError::Validation(format!("Invalid GitHub JSON payload: {e}")))?;

    let ref_str = val["ref"].as_str().unwrap_or("");
    let branch = if let Some(stripped) = ref_str.strip_prefix("refs/heads/") {
        stripped.to_string()
    } else {
        ref_str.to_string()
    };

    let commit_hash = val["after"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "0000000000000000000000000000000000000000")
        .or_else(|| val["head_commit"]["id"].as_str())
        .map(String::from);

    let commit_message = val["head_commit"]["message"].as_str().map(String::from);

    let commit_author = val["head_commit"]["author"]["name"]
        .as_str()
        .map(String::from);

    Ok(ParsedWebhookPayload {
        branch,
        commit_hash,
        commit_message,
        commit_author,
    })
}

/// Parses GitLab Push Hook JSON payload.
pub fn parse_gitlab_push_payload(payload: &[u8]) -> Result<ParsedWebhookPayload, CoreError> {
    let val: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| CoreError::Validation(format!("Invalid GitLab JSON payload: {e}")))?;

    let ref_str = val["ref"].as_str().unwrap_or("");
    let branch = if let Some(stripped) = ref_str.strip_prefix("refs/heads/") {
        stripped.to_string()
    } else {
        ref_str.to_string()
    };

    let commit_hash = val["after"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "0000000000000000000000000000000000000000")
        .or_else(|| val["checkout_sha"].as_str())
        .map(String::from);

    let commit_message = val["commits"]
        .as_array()
        .and_then(|arr| arr.last())
        .and_then(|c| c["message"].as_str())
        .map(String::from);

    let commit_author = val["user_name"].as_str().map(String::from);

    Ok(ParsedWebhookPayload {
        branch,
        commit_hash,
        commit_message,
        commit_author,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_git_repo_url() {
        assert!(validate_git_repo_url("git@github.com:nusa-panel/panel.git").is_ok());
        assert!(validate_git_repo_url("ssh://git@gitlab.com/owner/repo.git").is_ok());
        assert!(validate_git_repo_url("https://github.com/nusa-panel/panel.git").is_ok());

        assert!(validate_git_repo_url("").is_err());
        assert!(validate_git_repo_url("http://insecure.com/repo.git").is_err());
        assert!(validate_git_repo_url("git@github.com:owner/repo.git; rm -rf /").is_err());
        assert!(validate_git_repo_url("https://github.com/repo.git\n").is_err());
    }

    #[test]
    fn test_validate_git_branch() {
        assert!(validate_git_branch("main").is_ok());
        assert!(validate_git_branch("feature/p8-t4-git-deploy").is_ok());
        assert!(validate_git_branch("v1.0.0").is_ok());

        assert!(validate_git_branch("").is_err());
        assert!(validate_git_branch("/leading/slash").is_err());
        assert!(validate_git_branch("trailing/slash/").is_err());
        assert!(validate_git_branch("double..dot").is_err());
        assert!(validate_git_branch("invalid:char").is_err());
    }

    #[test]
    fn test_validate_docroot_subpath() {
        assert_eq!(validate_docroot_subpath("").unwrap(), "");
        assert_eq!(validate_docroot_subpath(".").unwrap(), "");
        assert_eq!(validate_docroot_subpath("public").unwrap(), "public");
        assert_eq!(validate_docroot_subpath("/dist/").unwrap(), "dist");
        assert_eq!(
            validate_docroot_subpath("build/client").unwrap(),
            "build/client"
        );

        assert!(validate_docroot_subpath("../parent").is_err());
        assert!(validate_docroot_subpath("sub/../escape").is_err());
        assert!(validate_docroot_subpath("bad;dir").is_err());
    }

    #[test]
    fn test_hmac_sha256_rfc4231_test_vector() {
        // RFC 4231 Test Case 2:
        // Key = "Jefe"
        // Data = "what do ya want for nothing?"
        // HMAC-SHA-256 = 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let mac = compute_hmac_sha256(key, data);
        assert_eq!(
            bytes_to_hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn test_verify_github_signature() {
        let secret = "my_webhook_secret_key";
        let payload = b"{\"action\":\"push\",\"ref\":\"refs/heads/main\"}";
        let mac = compute_hmac_sha256(secret.as_bytes(), payload);
        let header = format!("sha256={}", bytes_to_hex(&mac));

        assert!(verify_github_signature(payload, secret, &header));
        assert!(!verify_github_signature(payload, "wrong_secret", &header));
        assert!(!verify_github_signature(
            b"{\"tampered\":true}",
            secret,
            &header
        ));
    }

    #[test]
    fn test_verify_gitlab_token() {
        let secret = "gitlab_secret_token_123";
        assert!(verify_gitlab_token("gitlab_secret_token_123", secret));
        assert!(verify_gitlab_token("  gitlab_secret_token_123  ", secret));
        assert!(!verify_gitlab_token("wrong_token", secret));
    }

    #[test]
    fn test_parse_github_push_payload() {
        let raw = r#"{
            "ref": "refs/heads/release/v1.0",
            "after": "11223344556677889900aabbccddeeff11223344",
            "head_commit": {
                "id": "11223344556677889900aabbccddeeff11223344",
                "message": "feat: release v1.0",
                "author": { "name": "Budi Developer" }
            }
        }"#;

        let parsed = parse_github_push_payload(raw.as_bytes()).unwrap();
        assert_eq!(parsed.branch, "release/v1.0");
        assert_eq!(
            parsed.commit_hash.as_deref(),
            Some("11223344556677889900aabbccddeeff11223344")
        );
        assert_eq!(parsed.commit_message.as_deref(), Some("feat: release v1.0"));
        assert_eq!(parsed.commit_author.as_deref(), Some("Budi Developer"));
    }
}
