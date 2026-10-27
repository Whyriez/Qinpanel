//! Fixed-key credential encryption only; AI HTTP calls run in the non-root API.
use crate::{AgentError, crypto, system::SystemRunner};
use panel_proto::{AiSecret, AiSecretAction, AiSecretRequest, valid_ai_key};

pub fn secret(runner: &dyn SystemRunner, request: AiSecretRequest) -> Result<AiSecret, AgentError> {
    let context = format!("qonpanel-ai:v1:{}:", request.provider.as_str());
    let invalid =
        || AgentError::InvalidInput("Secret AI atau konteks provider tidak valid.".into());
    match request.action {
        AiSecretAction::Seal => {
            if !valid_ai_key(&request.value.0) {
                return Err(invalid());
            }
            let key = crypto::get_or_create_master_key(runner, None)?;
            crypto::encrypt_value(&key, &format!("{context}{}", request.value.0)).map(AiSecret)
        }
        AiSecretAction::Open => {
            if !request.value.0.starts_with(crypto::CIPHER_PREFIX)
                || request.value.0.len() > 4096
                || !runner.path_exists(std::path::Path::new(crypto::DEFAULT_MASTER_KEY_PATH))
            {
                return Err(invalid());
            }
            let key = crypto::get_or_create_master_key(runner, None)?;
            let plaintext = crypto::decrypt_value(&key, &request.value.0)?;
            let value = plaintext.strip_prefix(&context).ok_or_else(invalid)?;
            if !valid_ai_key(value) {
                return Err(invalid());
            }
            Ok(AiSecret(value.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    use panel_proto::AiProvider;

    #[test]
    fn credentials_roundtrip_only_in_their_provider_context_without_commands() {
        let runner = MockSystemRunner::new();
        runner.set_file_content(crypto::DEFAULT_MASTER_KEY_PATH, &[42; 32]);
        let value = AiSecret("fixture_ai_key_not_a_real_secret_123".into());
        for provider in [AiProvider::Gemini, AiProvider::Openai] {
            let sealed = secret(
                &runner,
                AiSecretRequest {
                    provider,
                    action: AiSecretAction::Seal,
                    value: value.clone(),
                },
            )
            .expect("seal");
            assert!(!sealed.0.contains(&value.0));
            assert!(!format!("{sealed:?}").contains(&value.0));
            assert_eq!(
                secret(
                    &runner,
                    AiSecretRequest {
                        provider,
                        action: AiSecretAction::Open,
                        value: sealed.clone()
                    }
                )
                .expect("open"),
                value
            );
            let other = if provider == AiProvider::Gemini {
                AiProvider::Openai
            } else {
                AiProvider::Gemini
            };
            assert!(
                secret(
                    &runner,
                    AiSecretRequest {
                        provider: other,
                        action: AiSecretAction::Open,
                        value: sealed
                    }
                )
                .is_err()
            );
        }
        assert!(runner.get_commands().is_empty());
    }

    #[test]
    fn plaintext_open_and_header_injection_are_rejected() {
        let runner = MockSystemRunner::new();
        for (action, value) in [
            (AiSecretAction::Open, "not_encrypted_key_123"),
            (
                AiSecretAction::Seal,
                "key_with_newline\r\nAuthorization_bad",
            ),
            (AiSecretAction::Seal, "enc:v1:fakeciphertext"),
        ] {
            assert!(
                secret(
                    &runner,
                    AiSecretRequest {
                        provider: AiProvider::Gemini,
                        action,
                        value: AiSecret(value.into())
                    }
                )
                .is_err()
            );
        }
        assert!(runner.get_commands().is_empty());
    }

    #[test]
    fn opening_with_a_missing_master_key_never_creates_a_replacement() {
        let runner = MockSystemRunner::new();
        assert!(
            secret(
                &runner,
                AiSecretRequest {
                    provider: AiProvider::Gemini,
                    action: AiSecretAction::Open,
                    value: AiSecret("enc:v1:fixture".into())
                }
            )
            .is_err()
        );
        assert!(!runner.path_exists(std::path::Path::new(crypto::DEFAULT_MASTER_KEY_PATH)));
        assert!(runner.get_commands().is_empty());
    }
}
