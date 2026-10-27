//! Encrypt/decrypt S3 credentials using the existing master key and fixed context.
use crate::{AgentError, crypto, system::SystemRunner};
use panel_proto::{S3Credentials, StorageSecret, StorageSecretAction, StorageSecretRequest};
use std::path::Path;

pub fn execute(
    runner: &dyn SystemRunner,
    request: StorageSecretRequest,
) -> Result<StorageSecret, AgentError> {
    let invalid = || AgentError::InvalidInput("Credential storage atau scope tidak valid.".into());
    if request.owner_id <= 0 || request.connection_id.is_nil() || request.value.0.len() > 32768 {
        return Err(invalid());
    }
    let context = format!(
        "qonpanel-s3:v1:{}:{}:",
        request.owner_id, request.connection_id
    );
    match request.action {
        StorageSecretAction::Seal => {
            let credentials: S3Credentials =
                serde_json::from_str(&request.value.0).map_err(|_| invalid())?;
            if !credentials.valid() {
                return Err(invalid());
            }
            let key = crypto::get_or_create_master_key(runner, None)?;
            crypto::encrypt_value(&key, &format!("{context}{}", request.value.0)).map(StorageSecret)
        }
        StorageSecretAction::Open => {
            if !request.value.0.starts_with(crypto::CIPHER_PREFIX)
                || !runner.path_exists(Path::new(crypto::DEFAULT_MASTER_KEY_PATH))
            {
                return Err(invalid());
            }
            let key = crypto::get_or_create_master_key(runner, None)?;
            let plaintext = crypto::decrypt_value(&key, &request.value.0)?;
            let value = plaintext.strip_prefix(&context).ok_or_else(invalid)?;
            let credentials: S3Credentials = serde_json::from_str(value).map_err(|_| invalid())?;
            if !credentials.valid() {
                return Err(invalid());
            }
            Ok(StorageSecret(value.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    fn request(id: uuid::Uuid) -> StorageSecretRequest {
        StorageSecretRequest {
            connection_id: id,
            owner_id: 2,
            action: StorageSecretAction::Seal,
            value: StorageSecret(
                serde_json::to_string(&S3Credentials {
                    access_key: "fixture_access".into(),
                    secret_key: "fixture_secret_not_real".into(),
                    session_token: Some("fixture_token".into()),
                })
                .expect("json"),
            ),
        }
    }
    #[test]
    fn credential_context_is_bound_to_both_connection_and_owner() {
        let runner = MockSystemRunner::new();
        runner.set_file_content(crypto::DEFAULT_MASTER_KEY_PATH, &[42; 32]);
        let original = request(uuid::Uuid::new_v4());
        let sealed = execute(&runner, original.clone()).expect("seal");
        assert!(!sealed.0.contains("fixture_secret"));
        assert!(!format!("{original:?}").contains("fixture_secret"));
        let mut open = original.clone();
        open.action = StorageSecretAction::Open;
        open.value = sealed.clone();
        assert_eq!(
            execute(&runner, open.clone()).expect("open"),
            original.value
        );
        open.owner_id = 3;
        assert!(execute(&runner, open).is_err());
        let mut other = request(uuid::Uuid::new_v4());
        other.action = StorageSecretAction::Open;
        other.value = sealed;
        assert!(execute(&runner, other).is_err());
        assert!(runner.get_commands().is_empty());
    }
    #[test]
    fn missing_master_and_malformed_credentials_never_create_new_master_or_run_commands() {
        let runner = MockSystemRunner::new();
        let mut open = request(uuid::Uuid::new_v4());
        open.action = StorageSecretAction::Open;
        open.value = StorageSecret("enc:v1:fixture".into());
        assert!(execute(&runner, open).is_err());
        assert!(!runner.path_exists(Path::new(crypto::DEFAULT_MASTER_KEY_PATH)));
        let mut invalid = request(uuid::Uuid::new_v4());
        invalid.value=StorageSecret("{\"access_key\":\"key\\r\\nInjected\",\"secret_key\":\"fixture_secret\",\"session_token\":null}".into());
        assert!(execute(&runner, invalid).is_err());
        assert!(runner.get_commands().is_empty());
    }
}
