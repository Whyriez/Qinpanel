//! Provider-scoped AI credentials; no free paths, endpoints or commands.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Gemini,
    Openai,
}
impl AiProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Openai => "openai",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiSecret(pub String);
impl std::fmt::Debug for AiSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted AI credential]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiSecretAction {
    Seal,
    Open,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSecretRequest {
    pub provider: AiProvider,
    pub action: AiSecretAction,
    pub value: AiSecret,
}

pub fn valid_ai_key(value: &str) -> bool {
    // Service-account-bound Gemini auth keys contain dots (for example AQ.…).
    // Keep credentials opaque while excluding wrappers and header injection.
    (16..=512).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_formats_accept_dotted_auth_keys_and_preserve_length_limits() {
        for key in [
            "AIza_fixture_not_a_real_key_123",
            "AQ.fixture_not_a_real_auth_key-123.segment_456",
            "sk-proj-fixture_not_a_real_key_123",
        ] {
            assert!(valid_ai_key(key));
        }
        assert!(valid_ai_key(&"a".repeat(16)));
        assert!(valid_ai_key(&format!("AQ.{}", "a".repeat(509))));
        assert!(!valid_ai_key(&"a".repeat(15)));
        assert!(!valid_ai_key(&format!("AQ.{}", "a".repeat(510))));
    }

    #[test]
    fn key_formats_reject_wrappers_whitespace_controls_and_non_ascii() {
        for key in [
            "",
            "Bearer AQ.fixture_not_a_real_key_123",
            "\"AQ.fixture_not_a_real_key_123\"",
            "'AQ.fixture_not_a_real_key_123'",
            "AQ.fixture key_not_real_123",
            "AQ.fixture\tkey_not_real_123",
            "AQ.fixture\r\nInjected: header",
            "AQ.fixture\0key_not_real_123",
            "AQ.fixture\u{200b}key_not_real_123",
            "AQ.fixture\u{a0}key_not_real_123",
            "AQ.fixture/key_not_real_123",
            "enc:v1:fixture_not_a_real_cipher",
        ] {
            assert!(!valid_ai_key(key));
        }
    }

    #[test]
    fn rpc_roundtrip_keeps_secret_out_of_debug_and_rejects_free_paths() {
        let key = "fixture_ai_key_not_a_real_secret_123";
        let request = AiSecretRequest {
            provider: AiProvider::Openai,
            action: AiSecretAction::Seal,
            value: AiSecret(key.into()),
        };
        let rpc = crate::RpcRequest::new(
            "ai-fixture",
            crate::RequestPayload::AiSecret(request.clone()),
        );
        assert!(!format!("{rpc:?}").contains(key));
        let encoded = serde_json::to_vec(&rpc).expect("wire");
        assert_eq!(
            serde_json::from_slice::<crate::RpcRequest>(&encoded).expect("roundtrip"),
            rpc
        );
        let mut raw = serde_json::to_value(request).expect("json");
        raw["path"] = serde_json::json!("/arbitrary/secret");
        assert!(serde_json::from_value::<AiSecretRequest>(raw).is_err());
    }
}
