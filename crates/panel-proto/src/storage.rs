//! Connection/tenant-scoped S3 vault operations. Never accepts a filesystem path.
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StorageSecret(pub String);
impl std::fmt::Debug for StorageSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted storage credential]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageSecretAction {
    Seal,
    Open,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSecretRequest {
    pub connection_id: uuid::Uuid,
    pub owner_id: i64,
    pub action: StorageSecretAction,
    pub value: StorageSecret,
}

/// Fixed single-node control plane. Paths, service names and binary URLs belong
/// to the agent helper, never to an HTTP client. PEM and credentials redact Debug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedStorageRequest {
    // An empty struct variant enforces deny_unknown_fields; a tagged unit
    // variant would silently consume extra payload fields during deserialization.
    Status {},
    ConfigureTunnel {
        hostname: String,
    },
    PrepareHttp {
        hostname: String,
    },
    Configure {
        hostname: String,
        fullchain_pem: StorageSecret,
        private_key_pem: StorageSecret,
    },
    Provision {
        connection_id: uuid::Uuid,
        owner_id: i64,
        bucket: String,
        credentials: StorageSecret,
    },
    ApplicationKey {
        connection_id: uuid::Uuid,
        owner_id: i64,
        bucket: String,
        generation: u32,
        credentials: StorageSecret,
    },
    Decommission {
        connection_id: uuid::Uuid,
        owner_id: i64,
        bucket: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct S3Credentials {
    #[cfg_attr(feature = "openapi", schema(write_only))]
    pub access_key: String,
    #[cfg_attr(feature = "openapi", schema(write_only))]
    pub secret_key: String,
    #[cfg_attr(feature = "openapi", schema(write_only))]
    pub session_token: Option<String>,
}
impl std::fmt::Debug for S3Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted S3 credentials]")
    }
}
impl S3Credentials {
    pub fn valid(&self) -> bool {
        let printable = |v: &str, min, max| {
            (min..=max).contains(&v.len()) && v.bytes().all(|b| (33..=126).contains(&b))
        };
        (3..=256).contains(&self.access_key.len())
            && self
                .access_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            && printable(&self.secret_key, 8, 1024)
            && self
                .session_token
                .as_ref()
                .is_none_or(|v| printable(v, 1, 16384))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_control_redacts_pem_and_rejects_untyped_paths() {
        let request = ManagedStorageRequest::Configure {
            hostname: "storage.example.test".into(),
            fullchain_pem: StorageSecret("fixture-certificate".into()),
            private_key_pem: StorageSecret("fixture-private-key".into()),
        };
        let wire =
            crate::RpcRequest::new("managed", crate::RequestPayload::ManagedStorage(request));
        assert!(!format!("{wire:?}").contains("fixture-private-key"));
        assert!(!format!("{wire:?}").contains("fixture-certificate"));
        assert_eq!(
            serde_json::from_slice::<crate::RpcRequest>(
                &serde_json::to_vec(&wire).expect("encode")
            )
            .expect("decode"),
            wire
        );
        assert!(
            serde_json::from_value::<ManagedStorageRequest>(
                serde_json::json!({"action":"status","path":"/tmp/arbitrary"})
            )
            .is_err()
        );
    }

    #[test]
    fn managed_status_keeps_the_existing_wire_shape() {
        let wire = serde_json::json!({"action":"status"});
        assert_eq!(
            serde_json::to_value(ManagedStorageRequest::Status {}).expect("encode"),
            wire
        );
        assert_eq!(
            serde_json::from_value::<ManagedStorageRequest>(wire).expect("decode"),
            ManagedStorageRequest::Status {}
        );
    }

    #[test]
    fn managed_variants_reject_unknown_fields_directly_and_in_rpc_frames() {
        let connection_id =
            uuid::Uuid::parse_str("7c03851f-60dd-49d1-a3aa-a2100320e7c9").expect("fixture UUID");
        for request in [
            ManagedStorageRequest::Status {},
            ManagedStorageRequest::ConfigureTunnel {
                hostname: "storage.example.test".into(),
            },
            ManagedStorageRequest::PrepareHttp {
                hostname: "storage.example.test".into(),
            },
            ManagedStorageRequest::Configure {
                hostname: "storage.example.test".into(),
                fullchain_pem: StorageSecret("fixture-certificate".into()),
                private_key_pem: StorageSecret("fixture-private-key".into()),
            },
            ManagedStorageRequest::Provision {
                connection_id,
                owner_id: 2,
                bucket: "fixture-media".into(),
                credentials: StorageSecret("fixture-key".into()),
            },
            ManagedStorageRequest::ApplicationKey {
                connection_id,
                owner_id: 2,
                bucket: "fixture-media".into(),
                generation: 1,
                credentials: StorageSecret("fixture-key".into()),
            },
            ManagedStorageRequest::Decommission {
                connection_id,
                owner_id: 2,
                bucket: "fixture-media".into(),
            },
        ] {
            let raw = serde_json::to_value(&request).expect("encode request");
            assert_eq!(
                serde_json::from_value::<ManagedStorageRequest>(raw.clone())
                    .expect("decode request"),
                request
            );
            let wire = crate::RpcRequest::new(
                "managed-fixture",
                crate::RequestPayload::ManagedStorage(request),
            );
            let encoded = serde_json::to_value(&wire).expect("encode frame");
            assert_eq!(
                serde_json::from_value::<crate::RpcRequest>(encoded.clone()).expect("decode frame"),
                wire
            );
            for field in ["path", "command", "binary_url", "service"] {
                let mut untyped = raw.clone();
                untyped[field] = serde_json::json!("untyped-fixture-value");
                let mut frame = encoded.clone();
                frame["payload"]["data"] = untyped.clone();
                assert!(serde_json::from_value::<ManagedStorageRequest>(untyped).is_err());
                assert!(serde_json::from_value::<crate::RpcRequest>(frame).is_err());
            }
        }
    }

    #[test]
    fn wire_roundtrip_redacts_debug_and_does_not_accept_paths_or_header_injection() {
        let request = StorageSecretRequest {
            connection_id: uuid::Uuid::new_v4(),
            owner_id: 2,
            action: StorageSecretAction::Seal,
            value: StorageSecret("fixture_secret_payload".into()),
        };
        let wire = crate::RpcRequest::new(
            "fixture",
            crate::RequestPayload::StorageSecret(request.clone()),
        );
        assert!(!format!("{wire:?}").contains("fixture_secret"));
        assert_eq!(
            serde_json::from_slice::<crate::RpcRequest>(
                &serde_json::to_vec(&wire).expect("encode")
            )
            .expect("decode"),
            wire
        );
        let mut raw = serde_json::to_value(request).expect("json");
        raw["path"] = serde_json::json!("/arbitrary");
        assert!(serde_json::from_value::<StorageSecretRequest>(raw).is_err());
        let keys = S3Credentials {
            access_key: "fixture".into(),
            secret_key: "fixture_secret".into(),
            session_token: Some("injected\r\nheader".into()),
        };
        assert!(!keys.valid());
    }
}
