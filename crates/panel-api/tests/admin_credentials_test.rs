#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Pure Axum requests and temporary SQLite; no listening server or agent daemon.
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    agent_client::AgentClient,
    auth::{self, AuthenticatedUser, LockoutManager, RequireCsrf},
    config::AuthConfig,
    routes::AppState,
};
use panel_core::auth::{hash_password, verify_password};
use panel_db::{
    models::CreateUser,
    pool::{init_pool, run_migrations},
    repositories::SqliteUserRepository,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

async fn fixture() -> (AppState, tempfile::TempDir, i64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("panel.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let user = SqliteUserRepository::new(pool.clone())
        .create(&CreateUser {
            username: "admin_old".into(),
            password_hash: hash_password("OriginalPassword123!").unwrap(),
            role: "admin".into(),
            email: None,
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .unwrap();
    let state = AppState::new(
        Arc::new(AgentClient::new(dir.path().join("no-agent"))),
        pool,
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 900)),
    );
    (state, dir, user.id)
}
async fn post(
    state: &AppState,
    path: &str,
    payload: Value,
    cookie: &str,
    csrf: &str,
) -> (StatusCode, Value, String) {
    let response = panel_api::create_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .header("x-csrf-token", csrf)
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .unwrap_or_default();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body, cookie)
}
async fn login(state: &AppState) -> (String, String) {
    let (status, body, cookie) = post(
        state,
        "/api/v1/auth/login",
        json!({"username":"admin_old","password":"OriginalPassword123!"}),
        "",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (cookie, body["csrf_token"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn username_only_and_combined_changes_reauthenticate_rotate_and_revoke_other_sessions() {
    for new_password in [None, Some("ChangedPassword456!")] {
        let (state, _dir, id) = fixture().await;
        let (cookie, csrf) = login(&state).await;
        let (second_cookie, _) = login(&state).await;
        sqlx::query("INSERT INTO api_tokens (user_id,name,token_hash,scopes_json,created_at) VALUES (?,'test','api-token','[]',0)").bind(id).execute(&state.pool).await.unwrap();
        let (status,body,new_cookie)=post(&state,"/api/v1/auth/password",json!({"current_password":"OriginalPassword123!","username":"admin_new","new_password":new_password}),&cookie,&csrf).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["user"]["username"], "admin_new");
        assert_ne!(new_cookie, cookie);
        assert_ne!(body["csrf_token"], csrf);
        for old in [&cookie, &second_cookie] {
            let res = panel_api::create_router(state.clone())
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/auth/me")
                        .header("cookie", old)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        let res = panel_api::create_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/me")
                    .header("cookie", new_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id=?")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM api_tokens WHERE user_id=?")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .unwrap(),
            0
        );
        let user = SqliteUserRepository::new(state.pool.clone())
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap();
        assert!(user.linux_username.is_none());
        assert!(
            verify_password(
                new_password.unwrap_or("OriginalPassword123!"),
                &user.password_hash
            )
            .unwrap()
        );
        assert_eq!(
            post(
                &state,
                "/api/v1/auth/login",
                json!({"username":"admin_old","password":"OriginalPassword123!"}),
                "",
                ""
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(post(&state,"/api/v1/auth/login",json!({"username":"admin_new","password":new_password.unwrap_or("OriginalPassword123!")}),"","").await.0,StatusCode::OK);
    }
}

#[tokio::test]
async fn csrf_password_username_conflicts_and_invalid_input_do_not_mutate_credentials() {
    let (state, _dir, id) = fixture().await;
    let (cookie, csrf) = login(&state).await;
    sqlx::query("INSERT INTO users (username,password_hash,role,status,created_at,updated_at) VALUES ('taken','unused','admin','active',0,0)").execute(&state.pool).await.unwrap();
    for (payload, token, expected) in [
        (
            json!({"current_password":"OriginalPassword123!","username":"admin_new"}),
            "",
            StatusCode::FORBIDDEN,
        ),
        (
            json!({"current_password":"wrong","username":"admin_new"}),
            csrf.as_str(),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"current_password":"OriginalPassword123!","username":"taken"}),
            csrf.as_str(),
            StatusCode::CONFLICT,
        ),
        (
            json!({"current_password":"OriginalPassword123!","username":"bad name"}),
            csrf.as_str(),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"current_password":"OriginalPassword123!","new_password":"short"}),
            csrf.as_str(),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"current_password":"OriginalPassword123!"}),
            csrf.as_str(),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            post(&state, "/api/v1/auth/password", payload, &cookie, token)
                .await
                .0,
            expected
        );
        let user = SqliteUserRepository::new(state.pool.clone())
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.username, "admin_old");
        assert!(verify_password("OriginalPassword123!", &user.password_hash).unwrap());
    }
}

#[tokio::test]
async fn session_creation_failure_rolls_back_username_password_and_revocations() {
    let (state, _dir, id) = fixture().await;
    let (cookie, csrf) = login(&state).await;
    sqlx::query("CREATE TRIGGER injected BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&state.pool).await.unwrap();
    assert_eq!(post(&state,"/api/v1/auth/password",json!({"current_password":"OriginalPassword123!","username":"admin_new","new_password":"ChangedPassword456!"}),&cookie,&csrf).await.0,StatusCode::INTERNAL_SERVER_ERROR);
    let user = SqliteUserRepository::new(state.pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.username, "admin_old");
    assert!(verify_password("OriginalPassword123!", &user.password_hash).unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id=?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn impersonation_and_stale_sessions_cannot_change_credentials() {
    let (state, _dir, id) = fixture().await;
    for impersonated in [Some("operator".to_string()), None] {
        let actor = AuthenticatedUser {
            id,
            username: "admin_old".into(),
            role: "admin".into(),
            email: None,
            totp_enabled: false,
            session_id: "revoked-session".into(),
            csrf_token: "unused".into(),
            impersonated_by: impersonated.clone(),
        };
        let request = serde_json::from_value(
            json!({"current_password":"OriginalPassword123!","username":"admin_new"}),
        )
        .unwrap();
        let response =
            auth::change_password_handler(State(state.clone()), RequireCsrf(actor), Json(request))
                .await;
        assert_eq!(
            response.status(),
            if impersonated.is_some() {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::CONFLICT
            }
        );
    }
}

#[tokio::test]
async fn in_flight_login_cannot_create_a_session_after_credentials_or_2fa_are_changed() {
    use panel_db::{models::CreateSession, repositories::SqliteSessionRepository};
    for change in [
        "UPDATE users SET username='renamed'",
        "UPDATE users SET password_hash='replacement-hash'",
        "UPDATE users SET totp_enabled=1,totp_secret='replacement-secret'",
        "UPDATE users SET status='suspended'",
    ] {
        let (state, _dir, id) = fixture().await;
        let verified = SqliteUserRepository::new(state.pool.clone())
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap();
        let sessions = SqliteSessionRepository::new(state.pool.clone());
        let input = CreateSession {
            id: "in-flight".into(),
            user_id: id,
            token_hash: "hash".into(),
            ip_address: None,
            user_agent: None,
            expires_at: 9999999999,
        };
        assert!(
            sessions
                .create_if_credentials_match(&input, &verified)
                .await
                .unwrap()
        );
        sessions.delete_by_user_id(id).await.unwrap();
        sqlx::query(change).execute(&state.pool).await.unwrap();
        assert!(
            !sessions
                .create_if_credentials_match(&input, &verified)
                .await
                .unwrap()
        );
        assert!(sessions.list_by_user_id(id).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn credential_reauthentication_is_rate_limited() {
    let (state, _dir, _id) = fixture().await;
    let (cookie, csrf) = login(&state).await;
    for attempt in 0..5 {
        let (status, _, _) = post(
            &state,
            "/api/v1/auth/password",
            json!({"current_password":"wrong","username":"admin_new"}),
            &cookie,
            &csrf,
        )
        .await;
        assert_eq!(
            status,
            if attempt == 4 {
                StatusCode::LOCKED
            } else {
                StatusCode::BAD_REQUEST
            }
        );
    }
    assert_eq!(
        post(
            &state,
            "/api/v1/auth/password",
            json!({"current_password":"OriginalPassword123!","username":"admin_new"}),
            &cookie,
            &csrf
        )
        .await
        .0,
        StatusCode::LOCKED
    );
}

#[tokio::test]
async fn username_change_preserves_2fa_and_tenant_cannot_rename_linux_identity() {
    let (state, _dir, id) = fixture().await;
    let (cookie, csrf) = login(&state).await;
    sqlx::query("UPDATE users SET totp_enabled=1,totp_secret='JBSWY3DPEHPK3PXP' WHERE id=?")
        .bind(id)
        .execute(&state.pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            &state,
            "/api/v1/auth/password",
            json!({"current_password":"OriginalPassword123!","username":"admin_new"}),
            &cookie,
            &csrf
        )
        .await
        .0,
        StatusCode::OK
    );
    let user = SqliteUserRepository::new(state.pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert!(user.totp_enabled);
    assert_eq!(user.totp_secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));

    let (state, _dir, id) = fixture().await;
    sqlx::query(
        "UPDATE users SET role='user',linux_username='p_actual',linux_uid=10042 WHERE id=?",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .unwrap();
    let (cookie, csrf) = login(&state).await;
    assert_eq!(
        post(
            &state,
            "/api/v1/auth/password",
            json!({"current_password":"OriginalPassword123!","username":"tenant_new"}),
            &cookie,
            &csrf
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &state,
            "/api/v1/auth/password",
            json!({"current_password":"OriginalPassword123!","new_password":"ChangedPassword456!"}),
            &cookie,
            &csrf
        )
        .await
        .0,
        StatusCode::OK
    );
    let user = SqliteUserRepository::new(state.pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.username, "admin_old");
    assert_eq!(user.linux_username.as_deref(), Some("p_actual"));
    assert_eq!(user.linux_uid, Some(10042));
}
