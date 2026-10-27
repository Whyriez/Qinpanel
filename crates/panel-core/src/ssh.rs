//! SSH public key validation, parsing, and fingerprint calculation.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::tenant::ShellPolicy;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SshKeyError {
    #[error("SSH key cannot be empty")]
    EmptyKey,
    #[error("SSH key cannot contain multiple lines or newline characters")]
    ContainsNewlines,
    #[error("Invalid OpenSSH public key format: expected '<key-type> <base64-blob> [comment]'")]
    InvalidFormat,
    #[error(
        "Unsupported SSH key algorithm: '{0}'. Supported: ssh-ed25519, ssh-rsa, ecdsa-sha2-nistp256, ecdsa-sha2-nistp384, ecdsa-sha2-nistp521, sk-ssh-ed25519@openssh.com, sk-ecdsa-sha2-nistp256@openssh.com"
    )]
    UnsupportedAlgorithm(String),
    #[error("Failed to decode base64 key data: {0}")]
    Base64Decode(String),
    #[error("Invalid binary wire encoding: length prefix exceeds key data or corrupt wire format")]
    InvalidWireEncoding,
    #[error("Key type mismatch: wire format specifies '{wire}' but header is '{header}'")]
    TypeMismatch { wire: String, header: String },
    #[error("Invalid key size: {0}")]
    InvalidKeySize(String),
    #[error(
        "Weak RSA key: modulus size of {bits} bits is insecure (minimum required is 2048 bits)"
    )]
    WeakRsaKey { bits: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParsedSshKey {
    /// Algoritma/tipe kunci (mis. ssh-ed25519, ssh-rsa, ecdsa-sha2-nistp256)
    pub key_type: String,
    /// Baris kunci publik kanonikal tunggal
    pub public_key: String,
    /// Fingerprint SHA-256 standar OpenSSH (mis. SHA256:4t71o/...)
    pub fingerprint_sha256: String,
    /// Fingerprint MD5 legacy format hex dengan titik dua (mis. 2c:aa:...)
    pub fingerprint_md5: String,
    /// Komentar opsional dari kunci publik (mis. user@host)
    pub comment: Option<String>,
    /// Estimasi panjang bit kunci (mis. 256 untuk Ed25519, 2048/4096 untuk RSA)
    pub bit_length: Option<usize>,
}

/// Public server identity only, distinct from tenant authorized keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SshHostKeyFingerprint {
    pub key_type: String,
    pub fingerprint_sha256: String,
}

const SUPPORTED_KEY_TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Helper to read length-prefixed byte slice in SSH wire format (big-endian u32 length + bytes)
fn read_wire_string(data: &[u8], offset: &mut usize) -> Result<Vec<u8>, SshKeyError> {
    if *offset + 4 > data.len() {
        return Err(SshKeyError::InvalidWireEncoding);
    }
    let len = u32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]) as usize;
    *offset += 4;

    if *offset + len > data.len() {
        return Err(SshKeyError::InvalidWireEncoding);
    }
    let slice = data[*offset..*offset + len].to_vec();
    *offset += len;
    Ok(slice)
}

/// Parse and validate an OpenSSH public key line.
pub fn parse_ssh_public_key(input: &str) -> Result<ParsedSshKey, SshKeyError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(SshKeyError::EmptyKey);
    }

    if raw.contains('\n') || raw.contains('\r') {
        return Err(SshKeyError::ContainsNewlines);
    }

    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.len() < 2 {
        return Err(SshKeyError::InvalidFormat);
    }

    let key_type = tokens[0].to_string();
    if !SUPPORTED_KEY_TYPES.contains(&key_type.as_str()) {
        return Err(SshKeyError::UnsupportedAlgorithm(key_type));
    }

    let b64_data = tokens[1];
    let wire_bytes = STANDARD
        .decode(b64_data)
        .map_err(|e| SshKeyError::Base64Decode(e.to_string()))?;

    if wire_bytes.len() < 8 {
        return Err(SshKeyError::InvalidWireEncoding);
    }

    let mut offset = 0;
    let wire_type_bytes = read_wire_string(&wire_bytes, &mut offset)?;
    let wire_type =
        String::from_utf8(wire_type_bytes).map_err(|_| SshKeyError::InvalidWireEncoding)?;

    if wire_type != key_type {
        return Err(SshKeyError::TypeMismatch {
            wire: wire_type,
            header: key_type,
        });
    }

    let mut bit_length = None;

    match key_type.as_str() {
        "ssh-ed25519" => {
            // Must have 4 + 11 ("ssh-ed25519") + 4 + 32 (pubkey) = 51 bytes
            let pubkey_bytes = read_wire_string(&wire_bytes, &mut offset)?;
            if pubkey_bytes.len() != 32 || wire_bytes.len() != 51 {
                return Err(SshKeyError::InvalidKeySize(format!(
                    "ed25519 public key wire size must be 51 bytes, got {}",
                    wire_bytes.len()
                )));
            }
            bit_length = Some(256);
        }
        "ssh-rsa" => {
            // Exponent e
            let _e = read_wire_string(&wire_bytes, &mut offset)?;
            // Modulus n
            let n = read_wire_string(&wire_bytes, &mut offset)?;
            // Strip leading zero byte if present (sign bit padding in mpint)
            let mut n_slice = n.as_slice();
            while let Some((&0, rest)) = n_slice.split_first() {
                n_slice = rest;
            }
            let bits = n_slice.len() * 8;
            if bits < 2048 {
                return Err(SshKeyError::WeakRsaKey { bits });
            }
            bit_length = Some(bits);
        }
        "ecdsa-sha2-nistp256" | "sk-ecdsa-sha2-nistp256@openssh.com" => {
            bit_length = Some(256);
        }
        "ecdsa-sha2-nistp384" => {
            bit_length = Some(384);
        }
        "ecdsa-sha2-nistp521" => {
            bit_length = Some(521);
        }
        "sk-ssh-ed25519@openssh.com" => {
            bit_length = Some(256);
        }
        _ => {}
    }

    // SHA-256 fingerprint: SHA256:<base64-no-padding>
    let sha_digest = Sha256::digest(&wire_bytes);
    let fingerprint_sha256 = format!("SHA256:{}", STANDARD_NO_PAD.encode(sha_digest));

    // MD5 legacy fingerprint: aa:bb:cc:...
    let md5_digest = Md5::digest(&wire_bytes);
    let fingerprint_md5 = md5_digest
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":");

    let comment = if tokens.len() > 2 {
        Some(tokens[2..].join(" "))
    } else {
        None
    };

    let canonical_key = match &comment {
        Some(c) => format!("{} {} {}", key_type, b64_data, c),
        None => format!("{} {}", key_type, b64_data),
    };

    Ok(ParsedSshKey {
        key_type,
        public_key: canonical_key,
        fingerprint_sha256,
        fingerprint_md5,
        comment,
        bit_length,
    })
}

/// Parameters for generating SSH and SFTP commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectionParams<'a> {
    pub username: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub folder: Option<&'a str>,
    pub policy: ShellPolicy,
}

/// Escape a path for single-quoted shell string `'...'`.
/// In POSIX shells (bash, zsh, sh), a single quote inside single quotes is represented as `'\''`.
pub fn escape_shell_single_quote(path: &str) -> String {
    path.replace('\'', "'\\''")
}

/// Quote one argument for a POSIX client shell. Remote commands need a second layer.
fn quote(value: &str) -> String {
    format!("'{}'", escape_shell_single_quote(value))
}

fn valid_params(params: &SshConnectionParams<'_>) -> bool {
    crate::Username::parse(params.username).is_ok()
        && crate::connectivity::EndpointHost::parse(params.host).is_ok()
        && params.port != 0
        && params
            .folder
            .is_none_or(|p| !p.chars().any(char::is_control))
}

/// Generated text only; never executed by the API or agent. POSIX client syntax.
pub fn generate_ssh_command(params: &SshConnectionParams<'_>) -> Option<String> {
    if params.policy != ShellPolicy::Full || !valid_params(params) {
        return None;
    }
    let prefix = format!("ssh -p {}", params.port);
    let destination = format!("{}@{}", params.username, params.host);
    match params.folder.filter(|p| !p.is_empty()) {
        Some(folder) => {
            let remote = format!("cd -- {} && exec \"$SHELL\" -l", quote(folder));
            Some(format!("{prefix} -t {destination} {}", quote(&remote)))
        }
        None => Some(format!("{prefix} {destination}")),
    }
}

fn sftp_host(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// Connect first, then paste the separately quoted cd command at the SFTP
/// prompt. Startup destination paths undergo an additional OpenSSH parser and
/// may trigger implicit downloads, so arbitrary folders never go in this argv.
pub fn generate_sftp_command(params: &SshConnectionParams<'_>) -> Option<String> {
    if params.policy == ShellPolicy::None || !valid_params(params) {
        return None;
    }
    let destination = format!("{}@{}", params.username, sftp_host(params.host));
    Some(format!("sftp -P {} {}", params.port, quote(&destination)))
}

/// OpenSSH interactive cd parser: double quotes escape quote/backslash. Quoted
/// glob characters are treated literally by cd (makeargv + undo_glob_escape).
pub fn generate_sftp_cd_command(params: &SshConnectionParams<'_>) -> Option<String> {
    if params.policy == ShellPolicy::None || !valid_params(params) {
        return None;
    }
    let folder = params.folder.filter(|p| !p.is_empty())?;
    let escaped = folder.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("cd \"{escaped}\""))
}

pub fn generate_sftp_url(params: &SshConnectionParams<'_>) -> Option<String> {
    if params.policy == ShellPolicy::None || !valid_params(params) {
        return None;
    }
    let mut path = String::new();
    if let Some(folder) = params.folder.filter(|p| !p.is_empty()) {
        if !folder.starts_with('/') {
            path.push('/');
        }
        for b in folder.bytes() {
            if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
                path.push(char::from(b));
            } else {
                path.push_str(&format!("%{b:02X}"));
            }
        }
        if !path.ends_with('/') {
            path.push('/');
        }
    }
    Some(format!(
        "sftp://{}@{}:{}{}",
        params.username,
        sftp_host(params.host),
        params.port,
        path
    ))
}

/// Config fields are single validated tokens, never arbitrary config directives.
pub fn generate_ssh_config_snippet(
    host_alias: &str,
    host: &str,
    user: &str,
    port: u16,
    identity_file: Option<&str>,
) -> String {
    let identity = identity_file.unwrap_or("~/.ssh/id_ed25519");
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-./~".contains(&b))
    };
    if !token(host_alias)
        || !token(identity)
        || port == 0
        || crate::Username::parse(user).is_err()
        || crate::connectivity::EndpointHost::parse(host).is_err()
    {
        return String::new();
    }
    format!(
        "Host {host_alias}\n    HostName {host}\n    User {user}\n    Port {port}\n    IdentityFile {identity}\n    StrictHostKeyChecking ask\n"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const VALID_ED25519: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl developer@nusapanel";
    const VALID_ED25519_NO_COMMENT: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    #[test]
    fn test_valid_ed25519() {
        let parsed = parse_ssh_public_key(VALID_ED25519).expect("valid ed25519 key");
        assert_eq!(parsed.key_type, "ssh-ed25519");
        assert_eq!(parsed.comment.as_deref(), Some("developer@nusapanel"));
        assert!(parsed.fingerprint_sha256.starts_with("SHA256:"));
        assert_eq!(parsed.fingerprint_md5.split(':').count(), 16);
        assert_eq!(parsed.bit_length, Some(256));
        assert_eq!(parsed.public_key, VALID_ED25519);
    }

    #[test]
    fn test_valid_ed25519_without_comment() {
        let parsed = parse_ssh_public_key(VALID_ED25519_NO_COMMENT).expect("valid ed25519 key");
        assert_eq!(parsed.key_type, "ssh-ed25519");
        assert_eq!(parsed.comment, None);
        assert!(parsed.fingerprint_sha256.starts_with("SHA256:"));
        assert_eq!(parsed.bit_length, Some(256));
    }

    #[test]
    fn test_reject_multiline_and_newlines() {
        let multiline = format!(
            "{}\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... test",
            VALID_ED25519
        );
        assert_eq!(
            parse_ssh_public_key(&multiline),
            Err(SshKeyError::ContainsNewlines)
        );

        let _carriage = format!("{}\r", VALID_ED25519);
        // Note: raw.trim() removes trailing \r, but internal \r must fail:
        let internal_carriage = "ssh-ed25519 AAAAC3NzaC1l\rZDI1NTE5AAAAI... test";
        assert_eq!(
            parse_ssh_public_key(internal_carriage),
            Err(SshKeyError::ContainsNewlines)
        );
    }

    #[test]
    fn test_reject_unsupported_or_garbage() {
        assert_eq!(parse_ssh_public_key(""), Err(SshKeyError::EmptyKey));
        assert_eq!(
            parse_ssh_public_key("not-a-key"),
            Err(SshKeyError::InvalidFormat)
        );
        assert_eq!(
            parse_ssh_public_key("ssh-dss AAAAB3NzaC1kc3MAAACB... user@host"),
            Err(SshKeyError::UnsupportedAlgorithm("ssh-dss".to_string()))
        );
    }

    #[test]
    fn test_reject_mismatched_wire_type() {
        // Wire says ssh-ed25519, but header claims ssh-rsa
        let corrupted = "ssh-rsa AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl user@host";
        let err = parse_ssh_public_key(corrupted).unwrap_err();
        assert!(matches!(err, SshKeyError::TypeMismatch { .. }));
    }

    #[test]
    fn commands_quote_two_shell_layers_and_preserve_path() {
        let path = "/home/p_alice/bob's \"$HOME\" `id` $(id) \\ [x]*?";
        let p = SshConnectionParams {
            username: "p_alice",
            host: "2001:db8::1",
            port: 2222,
            folder: Some(path),
            policy: ShellPolicy::Full,
        };
        let cmd = generate_ssh_command(&p).unwrap();
        assert_eq!(
            cmd,
            format!(
                "ssh -p 2222 -t p_alice@2001:db8::1 {}",
                quote(&format!("cd -- {} && exec \"$SHELL\" -l", quote(path)))
            )
        );
        assert!(
            generate_sftp_command(&p)
                .unwrap()
                .contains("p_alice@[2001:db8::1]")
        );
        assert_eq!(
            generate_sftp_command(&p).unwrap(),
            "sftp -P 2222 'p_alice@[2001:db8::1]'"
        );
        assert!(
            generate_sftp_cd_command(&p)
                .unwrap()
                .starts_with("cd \"/home/p_alice/bob's \\\"$HOME")
        );
        let url = generate_sftp_url(&p).unwrap();
        assert!(url.starts_with("sftp://p_alice@[2001:db8::1]:2222/"));
        assert!(url.contains("%27s%20%22%24HOME%22"));
        assert!(url.contains("%5Bx%5D%2A%3F/"));
    }

    #[test]
    fn permissions_and_untrusted_parameters_fail_closed() {
        let p = SshConnectionParams {
            username: "p_alice",
            host: "ssh.example.com",
            port: 22,
            folder: None,
            policy: ShellPolicy::Full,
        };
        assert_eq!(
            generate_ssh_command(&p).unwrap(),
            "ssh -p 22 p_alice@ssh.example.com"
        );
        let sftp = SshConnectionParams {
            policy: ShellPolicy::Sftp,
            ..p.clone()
        };
        assert!(generate_ssh_command(&sftp).is_none());
        assert!(generate_sftp_command(&sftp).is_some());
        let none = SshConnectionParams {
            policy: ShellPolicy::None,
            ..p.clone()
        };
        assert!(generate_ssh_command(&none).is_none());
        assert!(generate_sftp_command(&none).is_none());
        assert!(generate_sftp_url(&none).is_none());
        for bad in [
            SshConnectionParams {
                host: "-oProxyCommand=id",
                ..p.clone()
            },
            SshConnectionParams {
                host: "host\nProxyCommand id",
                ..p.clone()
            },
            SshConnectionParams {
                username: "root",
                ..p.clone()
            },
            SshConnectionParams {
                port: 0,
                ..p.clone()
            },
            SshConnectionParams {
                folder: Some("/a\nb"),
                ..p
            },
        ] {
            assert!(generate_ssh_command(&bad).is_none());
            assert!(generate_sftp_command(&bad).is_none());
        }
    }

    #[test]
    fn config_uses_raw_ipv6_and_cannot_inject_directives() {
        let cfg =
            generate_ssh_config_snippet("p_alice-server", "2001:db8::1", "p_alice", 2222, None);
        assert!(cfg.contains("HostName 2001:db8::1\n"));
        assert!(cfg.contains("Port 2222\n"));
        assert!(cfg.contains("StrictHostKeyChecking ask\n"));
        assert!(
            generate_ssh_config_snippet("*\nProxyCommand id", "host.test", "p_alice", 22, None)
                .is_empty()
        );
    }
}
