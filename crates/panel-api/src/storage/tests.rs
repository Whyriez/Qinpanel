use super::*;
use crate::auth::{CSRF_HEADER_NAME, SESSION_COOKIE_NAME, generate_csrf_token};
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use panel_proto::{StorageSecretAction, StorageSecretRequest};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
};
use tower::ServiceExt;

fn keys() -> S3Credentials {
    S3Credentials {
        access_key: "fixture_access_key".into(),
        secret_key: "fixture_secret_key_not_real".into(),
        session_token: None,
    }
}

#[tokio::test]
async fn administrator_and_impersonated_sessions_cannot_access_owner_content() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let expires = chrono::Utc::now().timestamp() + 60;
    for actor in ["admin", "alice"] {
        if actor == "alice" {
            sqlx::query(
                "UPDATE sessions SET user_agent='impersonated_by:admin' WHERE id='session-alice'",
            )
            .execute(&ctx.pool)
            .await
            .expect("impersonation fixture");
        }
        for (method, suffix, body) in [
            ("GET", "objects".to_string(), json!({})),
            ("POST", "test".to_string(), json!({"expected_revision":1})),
            (
                "POST",
                "upload?expected_revision=1&key=private.txt&overwrite=true".to_string(),
                json!("file"),
            ),
            (
                "POST",
                "delete".to_string(),
                json!({"expected_revision":1,"key":"private.txt","confirmation":"private.txt"}),
            ),
            (
                "POST",
                "download".to_string(),
                json!({"expected_revision":1,"key":"private.txt"}),
            ),
            (
                "POST",
                "link".to_string(),
                json!({"expected_revision":1,"key":"private.txt","expires_in":3600}),
            ),
            (
                "POST",
                "access".to_string(),
                json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"visibility":"public"}),
            ),
            (
                "GET",
                format!(
                    "download?expected_revision=1&key=private.txt&expires_at={expires}&preview=true"
                ),
                json!({}),
            ),
            (
                "POST",
                "application-key".to_string(),
                json!({"expected_revision":1,"expected_generation":0,"confirmation":"project-media","retry":false}),
            ),
            (
                "POST",
                "policy".to_string(),
                json!({"expected_revision":1,"rules":policy::Rules::default()}),
            ),
        ] {
            let (status, body) = ctx
                .call(
                    method,
                    &format!("/api/v1/storage/connections/{id}/{suffix}"),
                    Some(actor),
                    true,
                    body,
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{actor} {suffix}: {body}");
        }
    }
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn storage_rules_are_owner_scoped_revision_bound_and_default_private() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let path = format!("/api/v1/storage/connections/{id}/policy");
    let (status, default) = ctx
        .call("GET", &path, Some("alice"), false, json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(default["visibility"], "owner");
    assert_eq!(default["enforcement"], "panel");
    assert!(default["used_bytes"].is_null());
    assert_eq!(
        ctx.call("GET", &path, Some("bob"), false, json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let rules = policy::Rules {
        read: false,
        delete: false,
        ..policy::Rules::default()
    };
    let input = json!({"expected_revision":1,"rules":rules});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), false, input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input).await.0,
        StatusCode::CONFLICT
    );
    for (method, suffix, body) in [
        ("GET", "objects", json!({})),
        (
            "POST",
            "download",
            json!({"expected_revision":1,"key":"private.txt"}),
        ),
        (
            "POST",
            "delete",
            json!({"expected_revision":1,"key":"private.txt","confirmation":"private.txt"}),
        ),
    ] {
        assert_eq!(
            ctx.call(
                method,
                &format!("/api/v1/storage/connections/{id}/{suffix}"),
                Some("alice"),
                true,
                body
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    // An owner can undo their own deny rule without exposing content to anyone else.
    assert_eq!(
        ctx.call(
            "POST",
            &path,
            Some("alice"),
            true,
            json!({"expected_revision":2,"rules":policy::Rules::default()})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn admin_usage_response_contains_totals_without_object_names_and_preserves_last_measurement_on_failure()
 {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let path = format!("/api/v1/storage/connections/{id}/usage");
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = ctx
        .call("POST", &path, Some("admin"), true, json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["used_bytes"], 3);
    assert_eq!(body["object_count"], 1);
    assert!(!body.to_string().contains("sample.txt"));
    ctx.runtime.fail.store(403, Ordering::SeqCst);
    assert_eq!(
        ctx.call("POST", &path, Some("admin"), true, json!({}))
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let (_, current) = ctx
        .call(
            "GET",
            &format!("/api/v1/storage/connections/{id}/policy"),
            Some("admin"),
            false,
            json!({}),
        )
        .await;
    assert_eq!(current["used_bytes"], 3);
    assert_eq!(current["measured_at"], body["measured_at"]);
}

#[tokio::test]
async fn usage_traverses_folders_and_continuation_pages_without_exposing_paths() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.runtime.paged.store(true, Ordering::SeqCst);
    let (status, body) = ctx
        .call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/usage"),
            Some("admin"),
            true,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["used_bytes"], 13);
    assert_eq!(body["object_count"], 3);
    assert!(!body.to_string().contains("private/"));
    assert_eq!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| **call == "list")
            .count(),
        3
    );
}

#[tokio::test]
async fn empty_folder_markers_respect_owner_csrf_write_and_file_extension_rules() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.verify(&id).await;
    let policy = format!("/api/v1/storage/connections/{id}/policy");
    assert_eq!(ctx.call("POST", &policy, Some("alice"), true,
        json!({"expected_revision":1,"rules":{"read":true,"write":true,"delete":true,"max_file_bytes":1024,"extensions":["png"]}})).await.0, StatusCode::OK);
    let upload = format!(
        "/api/v1/storage/connections/{id}/upload?expected_revision=1&key=folder%20space%2B%2F&overwrite=true"
    );
    for (user, csrf, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some("admin"), true, StatusCode::FORBIDDEN),
        (Some("bob"), true, StatusCode::NOT_FOUND),
        (Some("alice"), false, StatusCode::FORBIDDEN),
        (Some("alice"), true, StatusCode::OK),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri(&upload)
            .header("host", "panel.example.test")
            .header("content-type", "application/octet-stream");
        if let Some(user) = user {
            request = request.header("cookie", format!("{SESSION_COOKIE_NAME}=token-{user}"));
            if csrf {
                request = request.header(
                    CSRF_HEADER_NAME,
                    generate_csrf_token(&format!("session-{user}")),
                );
            }
        }
        let response = ctx
            .app
            .clone()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| **call == "upload")
            .count(),
        1
    );
    // Nonempty objects ending in slash are still files and cannot bypass extensions.
    assert_eq!(
        ctx.call("POST", &upload, Some("alice"), true, json!("bytes"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(ctx.call("POST", &policy, Some("alice"), true,
        json!({"expected_revision":2,"rules":{"read":true,"write":false,"delete":true,"max_file_bytes":1024,"extensions":["png"]}})).await.0, StatusCode::OK);
    let request = Request::builder()
        .method("POST")
        .uri(&upload)
        .header("host", "panel.example.test")
        .header("cookie", format!("{SESSION_COOKIE_NAME}=token-alice"))
        .header(CSRF_HEADER_NAME, generate_csrf_token("session-alice"))
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        ctx.app
            .clone()
            .oneshot(request)
            .await
            .expect("response")
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| **call == "upload")
            .count(),
        1
    );
}

#[tokio::test]
async fn panel_upload_limits_use_fresh_usage_and_block_provider_mutation() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.verify(&id).await;
    let allocation = format!("/api/v1/storage/connections/{id}/allocation");
    let policy = format!("/api/v1/storage/connections/{id}/policy");
    let upload = format!(
        "/api/v1/storage/connections/{id}/upload?expected_revision=1&key=photo.png&overwrite=true"
    );
    assert_eq!(
        ctx.call(
            "POST",
            &allocation,
            Some("alice"),
            true,
            json!({"expected_revision":1,"quota_bytes":4})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call(
            "POST",
            &allocation,
            Some("admin"),
            true,
            json!({"expected_revision":1,"quota_bytes":4})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call("POST", &upload, Some("alice"), true, json!("file"))
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert!(!ctx.runtime.calls.lock().expect("calls").contains(&"upload"));
    assert_eq!(
        ctx.call(
            "POST",
            &allocation,
            Some("admin"),
            true,
            json!({"expected_revision":2,"quota_bytes":100})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call("POST", &upload, Some("alice"), true, json!("file"))
            .await
            .0,
        StatusCode::OK
    );
    let mut rules = policy::Rules {
        extensions: vec!["jpg".into()],
        ..policy::Rules::default()
    };
    assert_eq!(
        ctx.call(
            "POST",
            &policy,
            Some("alice"),
            true,
            json!({"expected_revision":3,"rules":rules})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call("POST", &upload, Some("alice"), true, json!("file"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    rules.extensions.clear();
    rules.max_file_bytes = 2;
    assert_eq!(
        ctx.call(
            "POST",
            &policy,
            Some("alice"),
            true,
            json!({"expected_revision":4,"rules":rules})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call("POST", &upload, Some("alice"), true, json!("file"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|value| **value == "upload")
            .count(),
        1
    );
}

#[tokio::test]
async fn managed_downloads_use_owner_proxy_without_public_s3_presigning() {
    let ctx = Context::new().await;
    let (_, connection) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            managed_allocation(),
        )
        .await;
    let id = connection["id"].as_str().expect("id");
    ctx.verify(id).await;
    let (status, body) = ctx
        .call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/download"),
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"photo.png"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["url"]
            .as_str()
            .expect("url")
            .starts_with("/api/v1/storage/")
    );
    assert!(
        !ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .contains(&"download")
    );
    let url = body["url"].as_str().expect("proxy URL");
    for actor in ["admin", "bob"] {
        let expected = if actor == "admin" {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(
            ctx.call("GET", url, Some(actor), false, json!({})).await.0,
            expected
        );
    }
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(url)
                .header("host", "panel.example.test")
                .header("cookie", format!("{SESSION_COOKIE_NAME}=token-alice"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("disposition")
            .starts_with("attachment;")
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
        axum::body::Bytes::from_static(b"fixture file")
    );
    assert!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .contains(&"read_object")
    );
}

#[tokio::test]
async fn raster_preview_is_sniffed_and_active_documents_are_rejected() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let expires = chrono::Utc::now().timestamp() + 60;
    let base = format!(
        "/api/v1/storage/connections/{id}/download?expected_revision=1&expires_at={expires}&preview=true&key="
    );
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}fixture.png"))
                .header("host", "panel.example.test")
                .header("cookie", format!("{SESSION_COOKIE_NAME}=token-alice"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("disposition")
            .starts_with("inline;")
    );
    assert_eq!(
        ctx.call(
            "GET",
            &format!("{base}document.svg"),
            Some("alice"),
            false,
            json!({})
        )
        .await
        .0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
}

#[tokio::test]
async fn storage_tunnel_and_public_ip_queue_without_dns_tokens_but_keep_admin_csrf_and_consent() {
    for method in ["tunnel", "http01"] {
        let ctx = Context::new().await;
        ctx.runtime.dns_configured.store(false, Ordering::SeqCst);
        ctx.runtime.dns_fail.store(true, Ordering::SeqCst);
        let mut input = json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"hostname":"storage.example.test","method":method,"email":"admin@example.test","confirmed":true});
        if method == "tunnel" {
            input["email"] = json!("");
            input["confirmed"] = json!(false);
            input["zone_id"] = json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
            input["tunnel_id"] = json!("7c03851f-60dd-49d1-a3aa-a2100320e7c9");
        }
        let path = "/api/v1/storage/managed/tls";
        for (actor, csrf, expected) in [
            (None, false, StatusCode::UNAUTHORIZED),
            (Some("alice"), true, StatusCode::FORBIDDEN),
            (Some("admin"), false, StatusCode::FORBIDDEN),
        ] {
            assert_eq!(
                ctx.call("POST", path, actor, csrf, input.clone()).await.0,
                expected
            );
        }
        if method == "http01" {
            let mut denied = input.clone();
            denied["confirmed"] = json!(false);
            assert_eq!(
                ctx.call("POST", path, Some("admin"), true, denied).await.0,
                StatusCode::BAD_REQUEST
            );
        } else {
            let mut denied = input.clone();
            denied["tunnel_id"] = json!("http://untrusted");
            assert_eq!(
                ctx.call("POST", path, Some("admin"), true, denied).await.0,
                StatusCode::BAD_REQUEST
            );
        }
        let (status, body) = ctx
            .call("POST", path, Some("admin"), true, input.clone())
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(body["method"], method);
        assert_eq!(body["auto_renew"], method == "http01");
        assert_eq!(body["state"], "queued");
        assert_eq!(
            ctx.call("POST", path, Some("admin"), true, input.clone())
                .await
                .0,
            StatusCode::ACCEPTED
        );
        input["method"] = json!("dns01");
        input["email"] = json!("admin@example.test");
        input["confirmed"] = json!(true);
        input.as_object_mut().expect("object").remove("zone_id");
        input.as_object_mut().expect("object").remove("tunnel_id");
        assert_eq!(
            ctx.call("POST", path, Some("admin"), true, input).await.0,
            StatusCode::CONFLICT
        );
    }
}

#[tokio::test]
async fn storage_tls_inventory_distinguishes_dns_agent_failure_from_missing_token() {
    let ctx = Context::new().await;
    for (configured, failed) in [(false, false), (true, false), (true, true)] {
        ctx.runtime
            .dns_configured
            .store(configured, Ordering::SeqCst);
        ctx.runtime.dns_fail.store(failed, Ordering::SeqCst);
        let (status, body) = ctx
            .call(
                "GET",
                "/api/v1/storage/managed/tls",
                Some("admin"),
                false,
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["dns"]["available"], !failed);
        assert_eq!(body["dns"]["configured"], configured && !failed);
        assert_eq!(body["certificate"]["state"], "idle");
        assert!(!body.to_string().contains("fixture-dns-revision"));
        assert!(!body.to_string().contains("api_token"));
    }
    assert_eq!(ctx.runtime.configure_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn storage_tls_generation_is_admin_csrf_scoped_and_requires_dns_consent() {
    let ctx = Context::new().await;
    let request = json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"hostname":"storage.example.test","email":"admin@example.test","confirmed":true});
    for (user, csrf) in [(None, false), (Some("alice"), true), (Some("admin"), false)] {
        let (status, _) = ctx
            .call(
                "POST",
                "/api/v1/storage/managed/tls",
                user,
                csrf,
                request.clone(),
            )
            .await;
        assert!(matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/tls",
            Some("admin"),
            true,
            request.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    ctx.runtime.dns_configured.store(true, Ordering::SeqCst);
    for (field, value) in [
        ("confirmed", json!(false)),
        ("hostname", json!("storage.other.test")),
        ("hostname", json!("panel.example.test")),
        ("hostname", json!("192.0.2.1")),
        ("email", json!("admin@example.test\r\nInjected: value")),
    ] {
        let mut invalid = request.clone();
        invalid[field] = value;
        assert_eq!(
            ctx.call(
                "POST",
                "/api/v1/storage/managed/tls",
                Some("admin"),
                true,
                invalid
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM managed_storage_tls")
        .fetch_one(&ctx.pool)
        .await
        .expect("revision");
    assert_eq!(revision, 1);
    assert_eq!(ctx.runtime.configure_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn storage_tls_request_is_durable_idempotent_and_excludes_manual_overwrites() {
    let ctx = Context::new().await;
    ctx.runtime.dns_configured.store(true, Ordering::SeqCst);
    let request = json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"hostname":"storage.example.test","email":"admin@example.test","confirmed":true});
    let (status, accepted) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/tls",
            Some("admin"),
            true,
            request.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    assert_eq!(accepted["state"], "queued");
    assert_eq!(accepted["revision"], 2);
    let (_, duplicate) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/tls",
            Some("admin"),
            true,
            request.clone(),
        )
        .await;
    assert_eq!(accepted["job_id"], duplicate["job_id"]);
    let mut different = request.clone();
    different["email"] = json!("other@example.test");
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/tls",
            Some("admin"),
            true,
            different
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let manual = json!({"hostname":"storage.example.test","fullchain_pem":"fixture-cert","private_key_pem":"fixture-private-key"});
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed",
            Some("admin"),
            true,
            manual
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(ctx.runtime.configure_calls.load(Ordering::SeqCst), 0);
    let (_, inventory) = ctx
        .call(
            "GET",
            "/api/v1/storage/managed/tls",
            Some("admin"),
            false,
            json!({}),
        )
        .await;
    assert_eq!(inventory["certificate"]["state"], "queued");
    assert_eq!(inventory["dns"]["configured"], true);
    assert!(!inventory.to_string().contains("private_key"));
    let payload: String = sqlx::query_scalar("SELECT payload_json FROM jobs WHERE id=?")
        .bind(accepted["job_id"].as_str())
        .fetch_one(&ctx.pool)
        .await
        .expect("job");
    let payload: Value = serde_json::from_str(&payload).expect("payload");
    assert_eq!(payload.as_object().expect("object").len(), 2);
    assert!(payload.get("hostname").is_none());
    assert!(payload.get("api_token").is_none());
}
fn configuration() -> Value {
    json!({"expected_revision":0,"owner_id":2,"site_id":1,"label":"Project media","provider":"compatible","endpoint":"https://objects.example.test","region":"us-east-1","addressing":"path","bucket":"project-media","credentials":keys()})
}
pub(super) fn connection() -> Connection {
    Connection {
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: 2,
        site_id: None,
        label: "Fixture".into(),
        provider: "compatible".into(),
        endpoint: "https://objects.example.test".into(),
        region: "us-east-1".into(),
        addressing: "path".into(),
        bucket: "project-media".into(),
        credentials_enc: "enc:v1:fixture".into(),
        revision: 1,
        verified_revision: Some(1),
        verified_at: Some(chrono::Utc::now().timestamp()),
        created_at: 1,
        updated_at: 1,
        last_operation: None,
        managed: false,
        managed_state: None,
        application_key_generation: 0,
        application_key_state: None,
    }
}
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<&'static str>>,
    object_keys: Mutex<Vec<String>>,
    head_status: AtomicU16,
    fail: AtomicU16,
    session: AtomicBool,
    managed_fail: AtomicBool,
    dns_configured: AtomicBool,
    dns_fail: AtomicBool,
    configure_calls: AtomicUsize,
    paged: AtomicBool,
}
impl StorageRuntime for Fake {
    fn dns_status(&self) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async move {
            if self.dns_fail.load(Ordering::SeqCst) {
                return Err(Failure::Timeout);
            }
            Ok(
                json!({"configured":self.dns_configured.load(Ordering::SeqCst),"zone_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","zone_name":"example.test","revision":"fixture-dns-revision","pending":[]}),
            )
        })
    }
    fn managed(
        &self,
        request: panel_proto::ManagedStorageRequest,
    ) -> BoxFuture<'_, Result<Value, Failure>> {
        Box::pin(async move {
            match request {
                panel_proto::ManagedStorageRequest::Decommission { .. } => {
                    Ok(json!({"removed":true}))
                }
                panel_proto::ManagedStorageRequest::Status {} => Ok(
                    json!({"configured":true,"running":true,"endpoint":"https://storage.example.test"}),
                ),
                panel_proto::ManagedStorageRequest::Configure { .. } => {
                    self.configure_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(
                        json!({"configured":true,"running":true,"endpoint":"https://storage.example.test"}),
                    )
                }
                panel_proto::ManagedStorageRequest::ConfigureTunnel { .. } => Ok(
                    json!({"configured":true,"running":true,"origin_ready":true,"connection_mode":"tunnel","endpoint":"https://storage.example.test"}),
                ),
                panel_proto::ManagedStorageRequest::PrepareHttp { .. } => {
                    Ok(json!({"prepared":true}))
                }
                panel_proto::ManagedStorageRequest::SetBucketAccess {
                    revision, public, ..
                } => {
                    self.calls.lock().expect("lock").push("managed_access");
                    if self.managed_fail.load(Ordering::SeqCst) {
                        Err(Failure::Timeout)
                    } else {
                        Ok(json!({"ready":true,"revision":revision,"public":public}))
                    }
                }
                panel_proto::ManagedStorageRequest::Provision { .. } => {
                    self.calls.lock().expect("lock").push("managed_provision");
                    if self.managed_fail.load(Ordering::SeqCst) {
                        Err(Failure::Timeout)
                    } else {
                        Ok(json!({"ready":true}))
                    }
                }
                panel_proto::ManagedStorageRequest::ApplicationKey { generation, .. } => {
                    self.calls.lock().expect("lock").push("managed_key");
                    if self.managed_fail.load(Ordering::SeqCst) {
                        Err(Failure::Timeout)
                    } else {
                        Ok(json!({"ready":true,"generation":generation}))
                    }
                }
            }
        })
    }

    fn secret(
        &self,
        request: StorageSecretRequest,
    ) -> BoxFuture<'_, Result<StorageSecret, Failure>> {
        Box::pin(async move {
            Ok(StorageSecret(match request.action {
                StorageSecretAction::Seal => "enc:v1:fixture".into(),
                StorageSecretAction::Open => {
                    let mut value = keys();
                    if self.session.load(Ordering::SeqCst) {
                        value.session_token = Some("sensitive.JWT.session".into());
                    }
                    serde_json::to_string(&value).expect("keys")
                }
            }))
        })
    }
    fn execute(
        &self,
        connection: Connection,
        _credentials: S3Credentials,
        operation: Operation,
        _panel: StorageEndpoint,
    ) -> BoxFuture<'_, Result<Outcome, Failure>> {
        Box::pin(async move {
            self.calls.lock().expect("lock").push(match &operation {
                Operation::List { .. } => "list",
                Operation::Upload { .. } => "upload",
                Operation::Delete { .. } => "delete",
                Operation::CreateBucket => "create",
                Operation::Download { .. } => "download",
                Operation::AccessLink { .. } => "access_link",
                Operation::ReadObject { .. } => "read_object",
                Operation::HeadObject { .. } => "head_object",
            });
            if let Operation::Delete { key } | Operation::HeadObject { key } = &operation {
                self.object_keys
                    .lock()
                    .expect("object keys")
                    .push(key.clone());
            }
            match self.fail.load(Ordering::SeqCst) {
                0 => {}
                999 => return Err(Failure::Timeout),
                status => return Err(Failure::Provider(status)),
            }
            Ok(match operation {
                Operation::HeadObject { .. } => match self.head_status.load(Ordering::SeqCst) {
                    200 => Outcome::Done,
                    0 => return Err(Failure::Provider(404)),
                    999 => return Err(Failure::Timeout),
                    status => return Err(Failure::Provider(status)),
                },
                Operation::List { prefix, token } if self.paged.load(Ordering::SeqCst) => {
                    let first = prefix.is_empty() && token.is_none();
                    Outcome::Listing(Listing {
                        objects: vec![Object {
                            key: format!("{prefix}{}.txt", token.as_deref().unwrap_or("private")),
                            size: if prefix.is_empty() { 3 } else { 7 },
                            modified_at: "2026-10-29T00:00:00Z".into(),
                        }],
                        prefixes: if first {
                            vec!["private/".into()]
                        } else {
                            vec![]
                        },
                        next_token: first.then(|| "next".into()),
                    })
                }
                Operation::List { prefix, .. } => Outcome::Listing(Listing {
                    objects: vec![Object {
                        key: format!("{prefix}sample.txt"),
                        size: 3,
                        modified_at: "2026-10-26T00:00:00Z".into(),
                    }],
                    prefixes: vec![],
                    next_token: None,
                }),
                Operation::Download { key } => Outcome::Download {
                    url: signing::download(&connection, &keys(), &key, chrono::Utc::now())?,
                    expires_at: chrono::Utc::now().timestamp() + 60,
                },
                Operation::AccessLink { key, expires_in } => Outcome::Download {
                    url: signing::access(
                        &connection,
                        &keys(),
                        &key,
                        chrono::Utc::now(),
                        expires_in,
                        false,
                    )?,
                    expires_at: chrono::Utc::now().timestamp() + i64::from(expires_in),
                },
                Operation::ReadObject { key } => {
                    Outcome::Content(if key.ends_with("fixture.png") {
                        axum::body::Bytes::from_static(b"\x89PNG\r\n\x1a\nfixture")
                    } else {
                        axum::body::Bytes::from_static(b"fixture file")
                    })
                }
                _ => Outcome::Done,
            })
        })
    }
}
struct Context {
    app: Router,
    pool: panel_db::DbPool,
    runtime: Arc<Fake>,
}
impl Context {
    async fn new() -> Self {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("db");
        panel_db::pool::run_migrations(&pool)
            .await
            .expect("migrate");
        let now = chrono::Utc::now().timestamp();
        for (id, name, role, uid) in [
            (1, "admin", "admin", None),
            (2, "alice", "user", Some(10001)),
            (3, "bob", "user", Some(10002)),
        ] {
            sqlx::query("INSERT INTO users(id,username,password_hash,role,status,linux_username,linux_uid,created_at,updated_at) VALUES(?,?,'unused',?,'active',?,?,?,?)")
                .bind(id).bind(name).bind(role).bind(uid.map(|_|format!("p_{name}"))).bind(uid).bind(now).bind(now).execute(&pool).await.expect("user");
            sqlx::query("INSERT INTO sessions(id,user_id,token_hash,expires_at,created_at) VALUES(?,?,?,?,?)").bind(format!("session-{name}")).bind(id).bind(panel_core::auth::hash_token(&format!("token-{name}"))).bind(now+3600).bind(now).execute(&pool).await.expect("session");
        }
        for (id, user, name) in [(1, 2, "alice.example.test"), (2, 3, "bob.example.test")] {
            sqlx::query("INSERT INTO sites(id,user_id,kind,docroot,status,created_at,updated_at) VALUES(?,?,'static','fixture','active',?,?)").bind(id).bind(user).bind(now).bind(now).execute(&pool).await.expect("site");
            sqlx::query("INSERT INTO domains(site_id,name,unicode_name,is_primary,created_at,updated_at) VALUES(?,?,?,1,?,?)").bind(id).bind(name).bind(name).bind(now).bind(now).execute(&pool).await.expect("domain");
        }
        let runtime = Arc::new(Fake::default());
        let state = AppState::new(
            Arc::new(AgentClient::new("unused-s3-fixture")),
            pool.clone(),
            "testing".into(),
            crate::config::AuthConfig::default(),
            Arc::new(crate::auth::LockoutManager::new(5, 900)),
        );
        Self {
            app: router_with_runtime(runtime.clone())
                .route(
                    "/api/v1/tenants/{id}",
                    axum::routing::delete(crate::tenants::delete_tenant_handler),
                )
                .with_state(state),
            pool,
            runtime,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        user: Option<&str>,
        csrf: bool,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "panel.example.test")
            .header("content-type", "application/json");
        if let Some(user) = user {
            request = request.header("cookie", format!("{SESSION_COOKIE_NAME}=token-{user}"));
            if csrf {
                request = request.header(
                    CSRF_HEADER_NAME,
                    generate_csrf_token(&format!("session-{user}")),
                );
            }
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        if status.is_success() {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (status, serde_json::from_slice(&bytes).expect("json"))
    }
    async fn create(&self) -> String {
        let (status, value) = self
            .call(
                "POST",
                "/api/v1/storage/connections",
                Some("admin"),
                true,
                configuration(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["id"].as_str().expect("id").into()
    }
    async fn verify(&self, id: &str) {
        assert_eq!(
            self.call(
                "POST",
                &format!("/api/v1/storage/connections/{id}/test"),
                Some("alice"),
                true,
                json!({"expected_revision":1})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn context_is_authoritative_and_lists_real_primary_domains() {
    let ctx = Context::new().await;
    let (status, body) = ctx
        .call(
            "GET",
            "/api/v1/storage/context",
            Some("alice"),
            false,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["owners"].as_array().expect("owners").len(), 1);
    assert_eq!(body["sites"][0]["domain"], "alice.example.test");
    assert!(!body.to_string().contains("bob"));
}
#[tokio::test]
async fn authentication_admin_creation_and_csrf_precede_any_secret_or_provider_call() {
    let ctx = Context::new().await;
    assert_eq!(
        ctx.call("GET", "/api/v1/storage/connections", None, false, json!({}))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("alice"),
            true,
            configuration()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("admin"),
            false,
            configuration()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn encrypted_metadata_and_list_never_return_key_or_ciphertext() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let (_, body) = ctx
        .call(
            "GET",
            "/api/v1/storage/connections",
            Some("alice"),
            false,
            json!({}),
        )
        .await;
    assert_eq!(body["connections"][0]["id"], id);
    assert!(!body.to_string().contains("fixture_access_key"));
    assert!(!body.to_string().contains("enc:v1:"));
    let cipher: String = sqlx::query_scalar("SELECT credentials_enc FROM storage_connections")
        .fetch_one(&ctx.pool)
        .await
        .expect("cipher");
    assert!(cipher.starts_with("enc:v1:"));
}
#[tokio::test]
async fn cross_tenant_listing_objects_binding_and_download_are_rejected_before_network() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let (_, body) = ctx
        .call(
            "GET",
            "/api/v1/storage/connections",
            Some("bob"),
            false,
            json!({}),
        )
        .await;
    assert!(body["connections"].as_array().expect("list").is_empty());
    for (method, suffix, body) in [
        ("GET", "objects", json!({})),
        (
            "POST",
            "download",
            json!({"expected_revision":1,"key":"secret.txt"}),
        ),
        (
            "POST",
            "binding",
            json!({"expected_revision":1,"site_id":2}),
        ),
    ] {
        assert_eq!(
            ctx.call(
                method,
                &format!("/api/v1/storage/connections/{id}/{suffix}"),
                Some("bob"),
                true,
                body
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn admin_cannot_attach_another_tenants_project_or_use_admin_as_owner() {
    let ctx = Context::new().await;
    let mut value = configuration();
    value["site_id"] = json!(2);
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("admin"),
            true,
            value
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut value = configuration();
    value["owner_id"] = json!(1);
    value["site_id"] = Value::Null;
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("admin"),
            true,
            value
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn stale_confirmation_and_cross_owner_binding_never_mutate_provider() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    assert_eq!(
        ctx.call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/delete"),
            Some("alice"),
            true,
            json!({"expected_revision":9,"key":"file.txt","confirmation":"file.txt"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        ctx.call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/binding"),
            Some("alice"),
            true,
            json!({"expected_revision":1,"site_id":2})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn verification_gates_mutations_and_delete_requires_exact_confirmation() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let path = format!("/api/v1/storage/connections/{id}/delete");
    assert_eq!(
        ctx.call(
            "POST",
            &path,
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"file.txt","confirmation":"file.txt"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    ctx.verify(&id).await;
    assert_eq!(
        ctx.call(
            "POST",
            &path,
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"file.txt","confirmation":"other"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        ctx.call(
            "POST",
            &path,
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"file.txt","confirmation":"file.txt"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let status: String = sqlx::query_scalar("SELECT state FROM storage_operations")
        .fetch_one(&ctx.pool)
        .await
        .expect("operation");
    assert_eq!(status, "succeeded");
}
#[tokio::test]
async fn disconnected_metadata_preserves_provider_and_other_tenant_data() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    assert_eq!(
        ctx.call(
            "DELETE",
            &format!("/api/v1/storage/connections/{id}"),
            Some("admin"),
            true,
            json!({"expected_revision":1,"confirmation":"project-media"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(ctx.runtime.calls.lock().expect("calls").is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sites")
        .fetch_one(&ctx.pool)
        .await
        .expect("sites");
    assert_eq!(count, 2);
}
#[tokio::test]
async fn operation_lease_is_atomic_and_timeout_is_unknown_not_success() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.verify(&id).await;
    let value: Connection = sqlx::query_as("SELECT * FROM storage_connections WHERE id=?")
        .bind(&id)
        .fetch_one(&ctx.pool)
        .await
        .expect("connection");
    let (a, b) = tokio::join!(
        store::reserve(&ctx.pool, &value, 2, "upload"),
        store::reserve(&ctx.pool, &value, 2, "delete")
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let lease = a.or(b).expect("one lease");
    let error = Err(Failure::Timeout);
    store::finish(&ctx.pool, &lease, &error)
        .await
        .expect("finish");
    let state: String = sqlx::query_scalar("SELECT state FROM storage_operations WHERE id=?")
        .bind(&lease)
        .fetch_one(&ctx.pool)
        .await
        .expect("state");
    assert_eq!(state, "unknown");
}
#[tokio::test]
async fn destination_change_requires_new_keys_and_rotation_preserves_identity() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let mut value = configuration();
    value["expected_revision"] = json!(1);
    value["endpoint"] = json!("https://other.example.test");
    value.as_object_mut().expect("object").remove("credentials");
    assert_eq!(
        ctx.call(
            "POST",
            &format!("/api/v1/storage/connections/{id}"),
            Some("admin"),
            true,
            value
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut value = configuration();
    value["expected_revision"] = json!(1);
    value["label"] = json!("Renamed");
    let (status, result) = ctx
        .call(
            "POST",
            &format!("/api/v1/storage/connections/{id}"),
            Some("admin"),
            true,
            value,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["id"], id);
    assert_eq!(result["revision"], 2);
    assert!(result["verified_at"].is_null());
}
#[test]
fn endpoint_and_object_contracts_reject_ambiguous_or_insecure_inputs() {
    for endpoint in [
        "http://objects.example.test",
        "https://user:pass@objects.example.test",
        "https://objects.example.test?token=x",
        "https://objects.example.test/admin",
        "https://127.0.0.1",
        "https://objects.example.test/#x",
    ] {
        let mut c = connection();
        c.endpoint = endpoint.into();
        assert!(c.validate().is_err(), "{endpoint}");
    }
    for value in ["../file", "folder/./file", "a\\b", "a\nb"] {
        assert!(domain::key(value).is_err());
    }
    let mut c = connection();
    c.provider = "supabase".into();
    c.endpoint = "https://project.storage.supabase.co/storage/v1/s3".into();
    assert!(c.validate().is_ok());
}
#[test]
fn xml_listing_decodes_keys_and_rejects_bad_xml_entities_and_out_of_scope_prefix() {
    let xml=b"<ListBucketResult><IsTruncated>false</IsTruncated><EncodingType>url</EncodingType><Contents><Key>images%2Fhello%20%2B.png</Key><Size>42</Size><LastModified>2026-10-26T00:00:00Z</LastModified></Contents></ListBucketResult>";
    let list = transport::parse_listing(xml, "images/").expect("listing");
    assert_eq!(list.objects[0].key, "images/hello +.png");
    assert_eq!(list.objects[0].size, 42);
    assert!(transport::parse_listing(xml, "other/").is_err());
    assert!(transport::parse_listing(b"<!DOCTYPE a><ListBucketResult/>", "").is_err());
    assert!(transport::parse_listing(b"<malformed>", "").is_err());
    assert!(transport::parse_listing(b"<ListBucketResult><IsTruncated>false</IsTruncated><NextContinuationToken/></ListBucketResult>","").is_ok());
    assert!(
        transport::parse_listing(b"<Other><IsTruncated>false</IsTruncated></Other>", "").is_err()
    );
}

#[test]
fn namespaced_s3_xml_preserves_listing_and_resource_limits() {
    let xml = br#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
        <IsTruncated>true</IsTruncated><EncodingType>url</EncodingType>
        <NextContinuationToken>fixture+token==</NextContinuationToken>
        <Contents><Key>images%2Freport%20%2B.txt</Key><Size>42</Size>
        <LastModified>2026-10-26T00:00:00Z</LastModified></Contents>
        <CommonPrefixes><Prefix>images%2Farchive%2F</Prefix></CommonPrefixes>
    </ListBucketResult>"#;
    let listing = transport::parse_listing(xml, "images/").expect("namespaced S3 listing");
    assert_eq!(listing.objects.len(), 1);
    assert_eq!(listing.objects[0].key, "images/report +.txt");
    assert_eq!(listing.objects[0].size, 42);
    assert_eq!(listing.prefixes.len(), 1);
    assert_eq!(listing.prefixes[0], "images/archive/");
    assert_eq!(listing.next_token.as_deref(), Some("fixture+token=="));
    assert!(transport::parse_listing(&vec![b' '; domain::LIST_LIMIT + 1], "").is_err());
    let deep = format!(
        "<ListBucketResult>{}<IsTruncated>false</IsTruncated>{}</ListBucketResult>",
        "<nested>".repeat(17),
        "</nested>".repeat(17)
    );
    assert!(transport::parse_listing(deep.as_bytes(), "").is_err());
}

#[test]
fn listing_query_encoding_preserves_spaces_literal_pluses_and_percent_once() {
    let name = "WhatsApp Image 2026-10-02 at 20.58.22.jpeg";
    let xml = br#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
        <IsTruncated>true</IsTruncated><EncodingType>url</EncodingType>
        <NextContinuationToken>opaque+token%2B==</NextContinuationToken>
        <Contents><Key>WhatsApp+Image+2026-10-02+at+20.58.22.jpeg</Key><Size>42</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents>
        <Contents><Key>WhatsApp%2BImage%2B2026-10-02%2Bat%2B20.58.22.jpeg</Key><Size>43</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents>
        <Contents><Key>literal%252B%2520.txt</Key><Size>44</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents>
        <CommonPrefixes><Prefix>folder+space%2Bplus%2F</Prefix></CommonPrefixes>
    </ListBucketResult>"#;
    let listing = transport::parse_listing(xml, "").expect("query-escaped list");
    assert_eq!(listing.objects[0].key, name);
    assert_eq!(
        listing.objects[1].key,
        "WhatsApp+Image+2026-10-02+at+20.58.22.jpeg"
    );
    assert_eq!(listing.objects[2].key, "literal%2B%20.txt");
    assert_eq!(listing.prefixes, ["folder space+plus/"]);
    assert_eq!(listing.next_token.as_deref(), Some("opaque+token%2B=="));
    assert!(
        transport::parse_listing(xml, name).is_err(),
        "Other keys in this fixture are outside the exact prefix"
    );
    let exact = String::from(
        "<ListBucketResult><IsTruncated>false</IsTruncated><EncodingType>url</EncodingType><Contents><Key>WhatsApp+Image+2026-10-02+at+20.58.22.jpeg</Key><Size>42</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents></ListBucketResult>",
    );
    let exact = transport::parse_listing(exact.as_bytes(), name)
        .expect("exact prefix no longer rejects spaces");
    assert_eq!(exact.objects[0].key, name);
    assert!(
        signing::public_url(&connection(), &exact.objects[0].key)
            .expect("URL")
            .contains("WhatsApp%20Image%202026-10-02%20at%2020.58.22.jpeg")
    );
    let plain = br#"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>plain+plus%20.txt</Key><Size>1</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents></ListBucketResult>"#;
    assert_eq!(
        transport::parse_listing(plain, "")
            .expect("plain XML")
            .objects[0]
            .key,
        "plain+plus%20.txt"
    );
    assert_eq!(
        signing::decode("path+plus%20space").expect("path decode"),
        "path+plus space"
    );
}

#[tokio::test]
async fn delete_checks_the_exact_listed_key_and_only_confirms_a_missing_object() {
    for head in [0, 200, 403, 500, 999] {
        let ctx = Context::new().await;
        let id = ctx.create().await;
        ctx.verify(&id).await;
        ctx.runtime.head_status.store(head, Ordering::SeqCst);
        let xml = b"<ListBucketResult><IsTruncated>false</IsTruncated><EncodingType>url</EncodingType><Contents><Key>photos%2FWhatsApp+Image%2Boriginal.jpeg</Key><Size>42</Size><LastModified>2026-10-30T00:00:00Z</LastModified></Contents></ListBucketResult>";
        let key = &transport::parse_listing(xml, "photos/")
            .expect("list")
            .objects[0]
            .key;
        assert_eq!(key, "photos/WhatsApp Image+original.jpeg");
        let (status, body) = ctx
            .call(
                "POST",
                &format!("/api/v1/storage/connections/{id}/delete"),
                Some("alice"),
                true,
                json!({"expected_revision":1,"key":key,"confirmation":key}),
            )
            .await;
        assert_eq!(
            *ctx.runtime.object_keys.lock().expect("keys"),
            [key.clone(), key.clone()],
            "DELETE and HEAD must address the same unchanged key"
        );
        let operation: String =
            sqlx::query_scalar("SELECT state FROM storage_operations WHERE connection_id=?")
                .bind(&id)
                .fetch_one(&ctx.pool)
                .await
                .expect("operation");
        if head == 0 {
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(operation, "succeeded");
        } else {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
            assert!(
                body["error"]
                    .as_str()
                    .expect("error")
                    .contains("belum dipastikan hilang")
            );
            assert_eq!(operation, "unknown");
            let verified: Option<i64> =
                sqlx::query_scalar("SELECT verified_at FROM storage_connections WHERE id=?")
                    .bind(&id)
                    .fetch_one(&ctx.pool)
                    .await
                    .expect("verification");
            assert!(
                verified.is_none(),
                "Unconfirmed delete requires readback before another mutation"
            );
        }
    }
}

#[tokio::test]
async fn legacy_invalid_linux_identity_is_preserved_but_cannot_gain_storage_access() {
    let ctx = Context::new().await;
    sqlx::query("UPDATE users SET linux_uid=1001 WHERE id=2")
        .execute(&ctx.pool)
        .await
        .expect("legacy fixture");
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("admin"),
            true,
            configuration()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let uid: i64 = sqlx::query_scalar("SELECT linux_uid FROM users WHERE id=2")
        .fetch_one(&ctx.pool)
        .await
        .expect("uid");
    assert_eq!(uid, 1001);
}
#[test]
fn sigv4_encoding_payload_and_conditions_are_bound_and_presigned_urls_expire() {
    let c = connection();
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-26T00:00:00Z")
        .expect("time")
        .with_timezone(&chrono::Utc);
    assert_eq!(
        signing::path(&c, Some("images/hello +.png")),
        "/project-media/images/hello%20%2B.png"
    );
    assert_eq!(signing::decode("a%2Bb").expect("decode"), "a+b");
    let headers = signing::headers(
        &c,
        &keys(),
        signing::SignRequest {
            method: "PUT",
            path: "/project-media/a",
            query: "",
            body: b"data",
            overwrite: false,
        },
        now,
    )
    .expect("headers");
    assert_eq!(headers["if-none-match"], "*");
    assert!(headers["authorization"].contains("if-none-match"));
    assert!(!format!("{:?}", keys()).contains("fixture_secret"));
    let url = signing::download(&c, &keys(), "file.txt", now).expect("url");
    assert!(url.contains("X-Amz-Expires=60"));
    assert!(url.contains("response-content-disposition=attachment"));
    assert!(!url.contains(&keys().secret_key));
    let headers = signing::headers(
        &c,
        &keys(),
        signing::SignRequest {
            method: "GET",
            path: "/project-media/file.txt",
            query: "",
            body: b"",
            overwrite: true,
        },
        now,
    )
    .expect("golden signature");
    // Independent Python hashlib/hmac fixture for the canonical request above.
    assert!(
        headers["authorization"].ends_with(
            "Signature=5d4a6d0d10073e11bfb6c3df16449581cf40ca2fc39ed106bcc9aecfd7cbfdfd"
        )
    );
    let mut temporary = keys();
    temporary.session_token = Some("sensitive.JWT.session".into());
    assert!(signing::download(&c, &temporary, "file.txt", now).is_err());
}

#[tokio::test]
async fn authenticated_proxy_download_is_attachment_and_cross_tenant_or_expired_requests_fail() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let expires = chrono::Utc::now().timestamp() + 60;
    let path = format!(
        "/api/v1/storage/connections/{id}/download?expected_revision=1&key=file.txt&expires_at={expires}"
    );
    assert_eq!(
        ctx.call("GET", &path, Some("bob"), false, json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("host", "panel.example.test")
                .header("cookie", format!("{SESSION_COOKIE_NAME}=token-alice"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("header")
            .starts_with("attachment;")
    );
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
        axum::body::Bytes::from_static(b"fixture file")
    );
    assert_eq!(ctx.call("GET",&format!("/api/v1/storage/connections/{id}/download?expected_revision=1&key=file.txt&expires_at=1"),Some("alice"),false,json!({})).await.0,StatusCode::GONE);
    for expires in [i64::MIN, i64::MAX] {
        assert_eq!(ctx.call("GET", &format!("/api/v1/storage/connections/{id}/download?expected_revision=1&key=file.txt&expires_at={expires}"), Some("alice"), false, json!({})).await.0, StatusCode::GONE);
    }
}

#[tokio::test]
async fn unknown_upload_requires_readback_and_unknown_conditional_put_requires_explicit_consent() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.verify(&id).await;
    let prefix = format!(
        "/api/v1/storage/connections/{id}/upload?expected_revision=1&key=file.txt&overwrite="
    );
    assert_eq!(
        ctx.call(
            "POST",
            &format!("{prefix}false"),
            Some("alice"),
            true,
            json!("fixture")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    ctx.runtime.fail.store(999, Ordering::SeqCst);
    assert_eq!(
        ctx.call(
            "POST",
            &format!("{prefix}true"),
            Some("alice"),
            true,
            json!("fixture")
        )
        .await
        .0,
        StatusCode::GATEWAY_TIMEOUT
    );
    let row:(Option<i64>,String)=sqlx::query_as("SELECT c.verified_revision,o.state FROM storage_connections c JOIN storage_operations o ON o.connection_id=c.id WHERE c.id=?").bind(&id).fetch_one(&ctx.pool).await.expect("state");
    assert_eq!(row, (None, "unknown".into()));
    ctx.runtime.fail.store(0, Ordering::SeqCst);
    assert_eq!(
        ctx.call(
            "POST",
            &format!("{prefix}true"),
            Some("alice"),
            true,
            json!("fixture")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    ctx.verify(&id).await;
    assert_eq!(
        ctx.call(
            "POST",
            &format!("{prefix}true"),
            Some("alice"),
            true,
            json!("fixture")
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn session_credentials_never_appear_in_a_browser_presigned_url() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    ctx.verify(&id).await;
    ctx.runtime.session.store(true, Ordering::SeqCst);
    let (status, value) = ctx
        .call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/download"),
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"file.txt"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        value["url"]
            .as_str()
            .expect("url")
            .starts_with("/api/v1/storage/connections/")
    );
    assert!(!value.to_string().contains("sensitive.JWT.session"));
    assert!(
        !ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .contains(&"download")
    );
}

fn managed_allocation() -> Value {
    json!({"request_id":uuid::Uuid::new_v4(),"owner_id":2,"site_id":1,"label":"Alice media","bucket":"alice-media"})
}

#[tokio::test]
async fn bucket_access_is_explicit_revision_bound_and_reconciles_unknown_results() {
    let ctx = Context::new().await;
    let (status, allocated) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            managed_allocation(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{allocated}");
    let id = allocated["id"].as_str().expect("id");
    let path = format!("/api/v1/storage/connections/{id}/access");
    let link = format!("/api/v1/storage/connections/{id}/link");
    let (_, initial) = ctx
        .call("GET", &path, Some("alice"), false, json!({}))
        .await;
    assert_eq!(initial["visibility"], "private");
    let key = "photos/foto + café.png";
    let link_input = json!({"expected_revision":1,"key":key,"expires_in":3600});
    let (status, signed) = ctx
        .call("POST", &link, Some("alice"), true, link_input.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{signed}");
    assert_eq!(signed["kind"], "signed");
    assert!(
        signed["url"]
            .as_str()
            .expect("url")
            .contains("X-Amz-Expires=3600")
    );
    assert!(
        signed["url"]
            .as_str()
            .expect("url")
            .contains("response-content-disposition=inline")
    );
    let input =
        json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"visibility":"public"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), false, input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call("POST", &path, Some("bob"), true, input.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    ctx.runtime.managed_fail.store(true, Ordering::SeqCst);
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input.clone())
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (_, unknown) = ctx
        .call("GET", &path, Some("alice"), false, json!({}))
        .await;
    assert_eq!(unknown["state"], "unknown");
    assert_eq!(unknown["visibility"], "private");
    assert_eq!(unknown["desired_visibility"], "public");
    assert_eq!(
        ctx.call("POST", &link, Some("alice"), true, link_input.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let other =
        json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":2,"visibility":"private"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, other).await.0,
        StatusCode::CONFLICT
    );
    ctx.runtime.managed_fail.store(false, Ordering::SeqCst);
    let (status, public) = ctx
        .call("POST", &path, Some("alice"), true, input.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["visibility"], "public");
    assert_eq!(public["revision"], 2);
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input).await.0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.runtime
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| **call == "managed_access")
            .count(),
        2,
        "Ready retries must not restart the engine"
    );
    let (status, url) = ctx
        .call("POST", &link, Some("alice"), true, link_input.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{url}");
    assert_eq!(
        url["url"],
        "https://storage.example.test/alice-media/photos/foto%20%2B%20caf%C3%A9.png"
    );
    assert!(url["expires_at"].is_null());
    assert_eq!(url["kind"], "public");
    let stale =
        json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"visibility":"private"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, stale).await.0,
        StatusCode::CONFLICT
    );
    let revoke =
        json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":2,"visibility":"private"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, revoke).await.1["visibility"],
        "private"
    );
    assert_eq!(
        ctx.call("POST", &link, Some("alice"), true, link_input)
            .await
            .1["kind"],
        "signed"
    );
    for body in [
        json!({"expected_revision":1,"key":"../secret","expires_in":3600}),
        json!({"expected_revision":1,"key":"file.txt","expires_in":86401}),
    ] {
        assert!(
            !ctx.call("POST", &link, Some("alice"), true, body)
                .await
                .0
                .is_success()
        );
    }
}

#[tokio::test]
async fn external_bucket_access_remains_provider_managed_and_read_rules_cover_links() {
    let ctx = Context::new().await;
    let id = ctx.create().await;
    let path = format!("/api/v1/storage/connections/{id}/access");
    let (_, access) = ctx
        .call("GET", &path, Some("alice"), false, json!({}))
        .await;
    assert_eq!(access["visibility"], "provider");
    assert_eq!(access["can_change"], false);
    let body =
        json!({"request_id":uuid::Uuid::new_v4(),"expected_revision":1,"visibility":"public"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, body).await.0,
        StatusCode::BAD_REQUEST
    );
    let rules = policy::Rules {
        read: false,
        ..policy::Rules::default()
    };
    assert_eq!(
        ctx.call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/policy"),
            Some("alice"),
            true,
            json!({"expected_revision":1,"rules":rules})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        ctx.call(
            "POST",
            &format!("/api/v1/storage/connections/{id}/link"),
            Some("alice"),
            true,
            json!({"expected_revision":1,"key":"file.txt","expires_in":3600})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn managed_service_requires_admin_and_separate_origin() {
    let ctx = Context::new().await;
    assert_eq!(
        ctx.call(
            "GET",
            "/api/v1/storage/managed",
            Some("alice"),
            false,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call(
            "GET",
            "/api/v1/storage/managed",
            Some("admin"),
            false,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let config = json!({"hostname":"panel.example.test","fullchain_pem":"fixture","private_key_pem":"fixture"});
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed",
            Some("admin"),
            false,
            config.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed",
            Some("admin"),
            true,
            config
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn managed_allocation_obeys_tenant_context_and_cannot_become_an_external_connection() {
    let ctx = Context::new().await;
    let allocation = managed_allocation();
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("alice"),
            true,
            allocation.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut cross = allocation.clone();
    cross["site_id"] = json!(2);
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            cross
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, result) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            allocation.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["managed"], true);
    assert_eq!(result["managed_state"], "ready");
    let id = result["id"].as_str().expect("id");
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            allocation
        )
        .await
        .1["id"],
        id
    );
    let list = ctx
        .call(
            "GET",
            "/api/v1/storage/connections",
            Some("alice"),
            false,
            json!({}),
        )
        .await
        .1;
    assert_eq!(list["connections"].as_array().expect("list").len(), 1);
    assert!(!list.to_string().contains("enc:v1"));
    assert_eq!(
        ctx.call(
            "GET",
            "/api/v1/storage/connections",
            Some("bob"),
            false,
            json!({})
        )
        .await
        .1["connections"],
        json!([])
    );
    let path = format!("/api/v1/storage/connections/{id}");
    assert_eq!(
        ctx.call("POST", &path, Some("admin"), true, configuration())
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        ctx.call(
            "DELETE",
            &path,
            Some("admin"),
            true,
            json!({"expected_revision":1,"confirmation":"alice-media"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn managed_unknown_provision_is_durable_and_reconciles_the_same_allocation() {
    let ctx = Context::new().await;
    let allocation = managed_allocation();
    ctx.runtime.managed_fail.store(true, Ordering::SeqCst);
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            allocation.clone()
        )
        .await
        .0,
        StatusCode::GATEWAY_TIMEOUT
    );
    let stored: (String, String) =
        sqlx::query_as("SELECT id,managed_state FROM storage_connections WHERE managed=1")
            .fetch_one(&ctx.pool)
            .await
            .expect("allocation");
    assert_eq!(stored.0, allocation["request_id"].as_str().expect("id"));
    assert_eq!(stored.1, "unknown");
    ctx.runtime.managed_fail.store(false, Ordering::SeqCst);
    let response = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            allocation,
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["managed_state"], "ready");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_connections")
        .fetch_one(&ctx.pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn application_key_is_owner_only_generation_bound_and_never_returned_by_get() {
    let ctx = Context::new().await;
    let (_, connection) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            managed_allocation(),
        )
        .await;
    let path = format!(
        "/api/v1/storage/connections/{}/application-key",
        connection["id"].as_str().expect("id")
    );
    let input = json!({"expected_revision":1,"expected_generation":0,"confirmation":"alice-media","retry":false});
    assert_eq!(
        ctx.call("POST", &path, Some("admin"), true, input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let result = ctx
        .call("POST", &path, Some("alice"), true, input.clone())
        .await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    assert_eq!(result.1["generation"], 1);
    assert!(result.1["credentials"]["secret_key"].is_string());
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input).await.0,
        StatusCode::CONFLICT
    );
    let listing = ctx
        .call(
            "GET",
            "/api/v1/storage/connections",
            Some("admin"),
            false,
            json!({}),
        )
        .await
        .1;
    assert!(!listing.to_string().contains("secret_key"));
    assert!(!listing.to_string().contains("credentials_enc"));
    assert_eq!(listing["connections"][0]["application_key_generation"], 1);
}

#[tokio::test]
async fn managed_core_target_is_trusted_and_does_not_weaken_external_endpoint_policy() {
    let mut value = connection();
    value.managed = true;
    value.managed_state = Some("ready".into());
    let core = value.core().expect("core");
    assert!(matches!(
        core.config.target,
        panel_core::object_storage::identity::StorageTarget::Managed { .. }
    ));
    assert!(core.config.credential.is_none());
    let mut input = configuration();
    input["managed"] = json!(true);
    let ctx = Context::new().await;
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/connections",
            Some("admin"),
            true,
            input
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn managed_removal_requires_admin_confirmation_and_native_ack_before_metadata_deletion() {
    let ctx = Context::new().await;
    let (_, connection) = ctx
        .call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            managed_allocation(),
        )
        .await;
    let path = format!(
        "/api/v1/storage/connections/{}/decommission",
        connection["id"].as_str().expect("id")
    );
    let input = json!({"expected_revision":1,"confirmation":"alice-media"});
    assert_eq!(
        ctx.call("POST", &path, Some("alice"), true, input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.call(
            "POST",
            &path,
            Some("admin"),
            true,
            json!({"expected_revision":1,"confirmation":"wrong"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        ctx.call("POST", &path, Some("admin"), true, input).await.0,
        StatusCode::OK
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_connections WHERE managed=1")
        .fetch_one(&ctx.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn tenant_deletion_with_managed_storage_is_rejected_before_agent_operations() {
    let ctx = Context::new().await;
    assert_eq!(
        ctx.call(
            "POST",
            "/api/v1/storage/managed/buckets",
            Some("admin"),
            true,
            managed_allocation()
        )
        .await
        .0,
        StatusCode::OK
    );
    let result = ctx
        .call(
            "DELETE",
            "/api/v1/tenants/2?purge=true",
            Some("admin"),
            true,
            json!({}),
        )
        .await;
    assert_eq!(result.0, StatusCode::CONFLICT, "{}", result.1);
    let identity: (String, i64) = sqlx::query_as("SELECT status,linux_uid FROM users WHERE id=2")
        .fetch_one(&ctx.pool)
        .await
        .expect("tenant");
    assert_eq!(identity, ("active".to_owned(), 10001));
}
