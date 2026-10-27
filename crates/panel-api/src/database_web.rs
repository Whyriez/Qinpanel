//! Authenticated internal phpMyAdmin/Adminer gateway. Every request rechecks ownership.
use crate::{
    auth::{AuthenticatedUser, RequireAdminCsrf, RequireAuth, RequireCsrf},
    routes::AppState,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use panel_core::hash_token;
use panel_db::repositories::{SqliteDatabaseRepository, SqliteUserRepository};
use panel_proto::{
    DatabaseWebCredentials, DatabaseWebCredentialsRequest, RequestPayload, ResponsePayload,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_RESPONSE: usize = 40 * 1024 * 1024;

/// Production binds to loopback; only the directly connected local proxy can
/// describe upstream TLS. Cookie policy is a minimum, never downgraded by headers.
fn request_secure(request: &Request, _configured_secure: bool) -> bool {
    request
        .extensions()
        .get::<crate::public_url::PublicRequest>()
        .map(|context| context.origin.secure)
        .unwrap_or_else(|| crate::public_url::request_https(request))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/databases/web-tools", get(tool_status))
        .route("/api/v1/databases/web-tools/install", post(install_tools))
        .route("/api/v1/databases/{id}/web-access", post(launch))
        .route(
            "/api/v1/databases/{id}/web/",
            get(proxy_root).post(proxy_root),
        )
        .route(
            "/api/v1/databases/{id}/web/{*asset}",
            get(proxy_asset).post(proxy_asset),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
}

fn error(status: StatusCode, message: &str) -> Response {
    let mut response = (status, Json(serde_json::json!({"error": message}))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn boxed_error(status: StatusCode, message: &str) -> Box<Response> {
    Box::new(error(status, message))
}

#[utoipa::path(get, path = "/api/v1/databases/web-tools", responses((status = 200, body = panel_proto::DatabaseWebStatus), (status = 503, description = "Inventory unavailable")), tag = "databases")]
pub async fn tool_status(State(state): State<AppState>, _user: RequireAuth) -> Response {
    match state
        .agent_client
        .database_web(RequestPayload::DatabaseWebStatus)
        .await
    {
        Ok(ResponsePayload::DatabaseWebStatus(status)) => Json(status).into_response(),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Status pengelola database belum dapat diperiksa",
        ),
    }
}

#[utoipa::path(post, path = "/api/v1/databases/web-tools/install", responses((status = 202, body = crate::jobs::JobItem), (status = 403, description = "Admin and CSRF required")), tag = "databases")]
pub async fn install_tools(
    State(state): State<AppState>,
    RequireAdminCsrf(user): RequireAdminCsrf,
) -> Response {
    match state.job_runner.enqueue_database_web_install(user.id).await {
        Ok((job, _)) => {
            state.audit_writer.log_async(
                Some(user.id),
                &user.username,
                "database.web_install",
                "host",
                "queued",
                None,
                None,
            );
            (StatusCode::ACCEPTED, Json(crate::jobs::JobItem::from(job))).into_response()
        }
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Gagal mengantrekan instalasi pengelola database",
        ),
    }
}

pub struct WebToolsInstallHandler(pub std::sync::Arc<crate::agent_client::AgentClient>);
impl crate::jobs::JobHandler for WebToolsInstallHandler {
    fn run<'a>(
        &'a self,
        job: &'a panel_db::models::Job,
        ctx: &'a crate::jobs::JobContext,
    ) -> crate::jobs::JobFuture<'a> {
        Box::pin(async move {
            let actor = SqliteUserRepository::new(ctx.pool.clone())
                .find_by_id(job.user_id.ok_or("Aktor instalasi tidak tersedia")?)
                .await
                .map_err(|_| "Gagal memeriksa administrator")?
                .ok_or("Administrator tidak ditemukan")?;
            if actor.role != "admin" || actor.status != "active" {
                return Err("Instalasi memerlukan administrator aktif".into());
            }
            ctx.report_progress(
                10,
                Some("Memasang phpMyAdmin, Adminer, dan layanan internal…"),
            )
            .await
            .map_err(|e| e.to_string())?;
            match ctx
                .install(&self.0, panel_proto::InstallOperation::DatabaseWebTools, 10)
                .await?
            {
                ResponsePayload::DatabaseWebStatus(status) if status.ready => {
                    ctx.report_progress(100, Some("Pengelola database siap digunakan"))
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(None)
                }
                _ => Err("Pengelola database belum siap".into()),
            }
        })
    }
}

async fn access_request(
    state: &AppState,
    user: &AuthenticatedUser,
    id: i64,
    provision: bool,
) -> Result<(String, DatabaseWebCredentialsRequest), Box<Response>> {
    let database = SqliteDatabaseRepository::new(state.pool.clone())
        .find_by_id(id)
        .await
        .map_err(|_| boxed_error(StatusCode::INTERNAL_SERVER_ERROR, "Gagal memuat database"))?
        .ok_or_else(|| boxed_error(StatusCode::NOT_FOUND, "Database tidak ditemukan"))?;
    if !user.can_access_user(database.user_id) {
        return Err(boxed_error(StatusCode::FORBIDDEN, "Akses database ditolak"));
    }
    let tenant = SqliteUserRepository::new(state.pool.clone())
        .find_by_id(database.user_id)
        .await
        .map_err(|_| boxed_error(StatusCode::INTERNAL_SERVER_ERROR, "Gagal memuat tenant"))?
        .ok_or_else(|| boxed_error(StatusCode::NOT_FOUND, "Tenant tidak ditemukan"))?;
    let (tenant_username, tenant_uid) = crate::tenant_context::execution_identity(&tenant)
        .map_err(|(status, message)| boxed_error(status, &message))?;
    let engine = panel_core::DatabaseEngine::parse(&database.engine)
        .map_err(|_| boxed_error(StatusCode::BAD_REQUEST, "Engine database tidak didukung"))?;
    Ok((
        engine.as_str().into(),
        DatabaseWebCredentialsRequest {
            engine: engine.as_str().into(),
            name: database.name,
            db_user: database.db_user,
            tenant_username,
            tenant_uid,
            provision,
        },
    ))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct WebLaunchResponse {
    pub url: String,
    pub tool: String,
}

#[utoipa::path(post, path = "/api/v1/databases/{id}/web-access", params(("id" = i64, Path)), responses((status = 200, body = WebLaunchResponse), (status = 403, description = "Ownership and CSRF required"), (status = 503, description = "Tools not ready")), tag = "databases")]
pub async fn launch(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<i64>,
) -> Response {
    let (engine, request) = match access_request(&state, &user, id, true).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    match state
        .agent_client
        .database_web(RequestPayload::DatabaseWebStatus)
        .await
    {
        Ok(ResponsePayload::DatabaseWebStatus(status)) if status.ready => {}
        _ => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Pengelola database belum siap. Minta administrator memasang atau memperbaikinya di Runtime > Database.",
            );
        }
    }
    match state
        .agent_client
        .database_web(RequestPayload::DatabaseWebCredentials(request))
        .await
    {
        Ok(ResponsePayload::DatabaseWebCredentials(_)) => {
            let _ = panel_db::repositories::SqliteAuditLogRepository::new(
                state.operations_pool.clone(),
            )
            .create(&panel_db::models::CreateAuditLog {
                user_id: Some(user.id),
                actor_username: user.username,
                action: "database.web_access".into(),
                target_resource: format!("database:{id}"),
                details_json: None,
                ip_address: None,
                status: "success".into(),
            })
            .await;
            let mut response = Json(serde_json::json!({"url": format!("/api/v1/databases/{id}/web/"), "tool": if engine == "mariadb" { "phpMyAdmin" } else { "Adminer" }})).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        _ => error(
            StatusCode::BAD_GATEWAY,
            "Gagal menyiapkan akses database internal. Periksa layanan database dan coba lagi.",
        ),
    }
}

async fn proxy_root(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<i64>,
    request: Request,
) -> Response {
    proxy(state, user, id, String::new(), request).await
}
async fn proxy_asset(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path((id, asset)): Path<(i64, String)>,
    request: Request,
) -> Response {
    proxy(state, user, id, asset, request).await
}

/// FastCGI may execute only the fixed, root-owned entry script. Paths select assets, never PHP scripts.
pub fn valid_asset(asset: &str) -> bool {
    asset.len() <= 512
        && asset
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.@+".contains(&byte))
        && asset
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        || asset.is_empty()
}

fn same_origin(headers: &HeaderMap, secure: bool) -> bool {
    crate::public_url::single(headers, "host")
        .and_then(|host| crate::public_url::PanelOrigin::from_authority(host, secure).ok())
        .is_some_and(|origin| origin.matches_origin(headers, false))
}

/// phpMyAdmin names HTTPS sessions differently, including a prefix in newer releases.
/// Keep this allowlist aligned with the PHP bridge; panel cookies must never pass.
fn is_tool_cookie(name: &str) -> bool {
    if name.starts_with("adminer_") {
        return true;
    }
    let name = name.strip_prefix("__Secure-").unwrap_or(name);
    name.starts_with("pma") || matches!(name, "phpMyAdmin" | "phpMyAdmin_https")
}

fn tool_cookies(value: &str) -> String {
    value
        .split(';')
        .map(str::trim)
        .filter(|part| {
            part.split_once('=')
                .is_some_and(|(name, _)| is_tool_cookie(name))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

async fn proxy(
    state: AppState,
    user: AuthenticatedUser,
    id: i64,
    asset: String,
    request: Request,
) -> Response {
    if !valid_asset(&asset) {
        return error(StatusCode::BAD_REQUEST, "Jalur pengelola tidak valid");
    }
    let secure = request_secure(&request, state.auth_config.cookie_secure.unwrap_or(true));
    if !same_origin(request.headers(), secure) {
        return error(StatusCode::FORBIDDEN, "Permintaan lintas asal ditolak");
    }
    let (engine, access) = match access_request(&state, &user, id, false).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let database = access.name.clone();
    let credentials = match state
        .agent_client
        .database_web(RequestPayload::DatabaseWebCredentials(access))
        .await
    {
        Ok(ResponsePayload::DatabaseWebCredentials(value)) => value,
        _ => {
            return error(
                StatusCode::CONFLICT,
                "Buka kembali pengelola melalui tombol pada daftar database",
            );
        }
    };
    let prefix = format!("/api/v1/databases/{id}/web/");
    let query = request.uri().query().unwrap_or("").to_string();
    let method = request.method().as_str().to_string();
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host = request
        .extensions()
        .get::<crate::public_url::PublicRequest>()
        .map(|context| context.origin.authority())
        .or_else(|| {
            crate::public_url::single(request.headers(), "host")
                .and_then(|host| crate::public_url::PanelOrigin::from_authority(host, secure).ok())
                .map(|origin| origin.authority())
        })
        .unwrap_or_else(|| "localhost".into());
    // Never forward the panel authentication cookie to third-party PHP tools.
    let cookie = tool_cookies(
        request
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    );
    let body = match axum::body::to_bytes(request.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(_) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Permintaan pengelola maksimal 32 MiB",
            );
        }
    };
    let params = parameters(WebRequest {
        engine: &engine,
        database: &database,
        prefix: &prefix,
        asset: &asset,
        query: &query,
        method: &method,
        content_type: &content_type,
        host: &host,
        cookie: &cookie,
        session: &hash_token(&format!("{}:{id}", user.session_id)),
        credentials: &credentials,
        secure,
        length: body.len(),
    });
    match private_request(&params, &body)
        .await
        .and_then(|output| parse_response(&output, &prefix, secure, &host))
    {
        Ok(response) => response,
        Err(_) => error(
            StatusCode::BAD_GATEWAY,
            "Layanan pengelola database tidak merespons. Periksa panel-db-web.service.",
        ),
    }
}

struct WebRequest<'a> {
    engine: &'a str,
    database: &'a str,
    prefix: &'a str,
    asset: &'a str,
    query: &'a str,
    method: &'a str,
    content_type: &'a str,
    host: &'a str,
    cookie: &'a str,
    session: &'a str,
    credentials: &'a DatabaseWebCredentials,
    secure: bool,
    length: usize,
}
fn parameters(request: WebRequest<'_>) -> Vec<(String, String)> {
    let path = format!(
        "{}{}",
        request.prefix,
        if request.asset.is_empty() {
            "index.php"
        } else {
            request.asset
        }
    );
    [
        ("GATEWAY_INTERFACE", "CGI/1.1".into()),
        ("SERVER_PROTOCOL", "HTTP/1.1".into()),
        ("SERVER_SOFTWARE", "Qinpanel".into()),
        (
            "SERVER_NAME",
            crate::public_url::PanelOrigin::from_authority(request.host, request.secure)
                .map(|origin| origin.host.as_str().to_owned())
                .unwrap_or_else(|_| "localhost".into()),
        ),
        (
            "SERVER_PORT",
            crate::public_url::PanelOrigin::from_authority(request.host, request.secure)
                .map(|origin| origin.port.get().to_string())
                .unwrap_or_else(|_| if request.secure { "443" } else { "80" }.into()),
        ),
        ("REMOTE_ADDR", "127.0.0.1".into()),
        (
            "SCRIPT_FILENAME",
            "/usr/share/panel/db-web/entry.php".into(),
        ),
        ("SCRIPT_NAME", path.clone()),
        (
            "REQUEST_URI",
            format!(
                "{path}{}{}",
                if request.query.is_empty() { "" } else { "?" },
                request.query
            ),
        ),
        ("REQUEST_METHOD", request.method.into()),
        ("QUERY_STRING", request.query.into()),
        ("CONTENT_TYPE", request.content_type.into()),
        ("CONTENT_LENGTH", request.length.to_string()),
        ("HTTP_HOST", request.host.into()),
        ("HTTP_COOKIE", request.cookie.into()),
        ("HTTPS", if request.secure { "on" } else { "off" }.into()),
        ("REDIRECT_STATUS", "200".into()),
        ("PANEL_DB_ENGINE", request.engine.into()),
        ("PANEL_DATABASE", request.database.into()),
        ("PANEL_DB_USER", request.credentials.username.clone()),
        ("PANEL_DB_PASSWORD", request.credentials.password.clone()),
        ("PANEL_WEB_PREFIX", request.prefix.into()),
        ("PANEL_WEB_SESSION", request.session.into()),
        ("PANEL_WEB_ASSET", request.asset.into()),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}

async fn private_request(params: &[(String, String)], body: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        tokio::time::timeout(std::time::Duration::from_secs(65), async {
            let stream = tokio::net::UnixStream::connect("/run/panel-db-web/manager.sock")
                .await
                .map_err(|_| "socket tidak tersedia".to_string())?;
            exchange(stream, params, body).await
        })
        .await
        .map_err(|_| "pengelola timeout".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (params, body);
        Err("Pengelola internal memerlukan server Linux".into())
    }
}

fn record(kind: u8, content: &[u8]) -> Vec<u8> {
    let length = content.len() as u16;
    let mut result = vec![1, kind, 0, 1, (length >> 8) as u8, length as u8, 0, 0];
    result.extend_from_slice(content);
    result
}
fn encode_length(length: usize, out: &mut Vec<u8>) {
    if length < 128 {
        out.push(length as u8);
    } else {
        out.extend_from_slice(&((length as u32) | 0x80000000).to_be_bytes());
    }
}
/// Bounded FastCGI responder transport, also exercised with an in-memory duplex fixture.
pub async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    params: &[(String, String)],
    body: &[u8],
) -> Result<Vec<u8>, String> {
    stream
        .write_all(&record(1, &[0, 1, 0, 0, 0, 0, 0, 0]))
        .await
        .map_err(|_| "begin")?;
    let mut encoded = Vec::new();
    for (name, value) in params {
        encode_length(name.len(), &mut encoded);
        encode_length(value.len(), &mut encoded);
        encoded.extend_from_slice(name.as_bytes());
        encoded.extend_from_slice(value.as_bytes());
    }
    for chunk in encoded.chunks(65535) {
        stream
            .write_all(&record(4, chunk))
            .await
            .map_err(|_| "params")?;
    }
    stream
        .write_all(&record(4, &[]))
        .await
        .map_err(|_| "params end")?;
    for chunk in body.chunks(65535) {
        stream
            .write_all(&record(5, chunk))
            .await
            .map_err(|_| "stdin")?;
    }
    stream
        .write_all(&record(5, &[]))
        .await
        .map_err(|_| "stdin end")?;
    let mut output = Vec::new();
    let mut total = 0usize;
    loop {
        let mut header = [0; 8];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|_| "incomplete response")?;
        if header[0] != 1 || header[2..4] != [0, 1] {
            return Err("invalid response".into());
        }
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        total += length;
        if total > MAX_RESPONSE {
            return Err("response too large".into());
        }
        let mut bytes = vec![0; length + header[6] as usize];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|_| "incomplete record")?;
        match header[1] {
            6 => output.extend_from_slice(&bytes[..length]),
            7 => {} // PHP diagnostics may contain query data; never return or log stderr.
            3 => {
                if length != 8 || bytes[..5] != [0, 0, 0, 0, 0] {
                    return Err("responder failed".into());
                }
                return Ok(output);
            }
            _ => return Err("unexpected record".into()),
        }
    }
}

pub fn parse_response(
    output: &[u8],
    prefix: &str,
    secure: bool,
    host: &str,
) -> Result<Response, String> {
    let split = output
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("invalid CGI headers")?;
    if split > 64 * 1024 {
        return Err("CGI headers too large".into());
    }
    let headers =
        std::str::from_utf8(&output[..split]).map_err(|_| "invalid CGI header encoding")?;
    let mut response = Response::new(Body::from(Bytes::copy_from_slice(&output[split + 4..])));
    for line in headers.split("\r\n") {
        let (name, value) = line.split_once(':').ok_or("invalid CGI header")?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "status" => {
                *response.status_mut() = StatusCode::from_u16(
                    value
                        .split_whitespace()
                        .next()
                        .ok_or("status")?
                        .parse()
                        .map_err(|_| "status")?,
                )
                .map_err(|_| "status")?
            }
            "location" => {
                let rewritten;
                let value = if value.starts_with("http://") || value.starts_with("https://") {
                    let uri: axum::http::Uri = value.parse().map_err(|_| "unsafe redirect")?;
                    let origin = crate::public_url::PanelOrigin::from_authority(
                        uri.authority().ok_or("unsafe redirect")?.as_str(),
                        uri.scheme_str() == Some("https"),
                    )
                    .map_err(|_| "unsafe redirect")?;
                    let expected = crate::public_url::PanelOrigin::from_authority(host, secure)
                        .map_err(|_| "unsafe redirect")?;
                    if !expected.same(&origin) {
                        return Err("unsafe redirect".into());
                    }
                    rewritten = uri
                        .path_and_query()
                        .ok_or("unsafe redirect")?
                        .as_str()
                        .to_owned();
                    rewritten.as_str()
                } else {
                    value
                };
                let path = value.split(['?', '#']).next().ok_or("unsafe redirect")?;
                // phpMyAdmin emits ./index.php; the leading current-directory
                // marker stays inside its scoped gateway, unlike parent traversal.
                let checked_path = path.strip_prefix("./").unwrap_or(path);
                if path.contains(['\\', '%'])
                    || checked_path
                        .split('/')
                        .any(|part| matches!(part, "." | ".."))
                    || !(value == "/"
                        || path.starts_with(prefix)
                        || (!path.starts_with('/') && !path.contains(':')))
                {
                    return Err("unsafe redirect".into());
                }
                response.headers_mut().insert(
                    header::LOCATION,
                    HeaderValue::from_str(value).map_err(|_| "location")?,
                );
                if response.status() == StatusCode::OK {
                    *response.status_mut() = StatusCode::FOUND;
                }
            }
            "set-cookie" => {
                let pair = value.split(';').next().ok_or("cookie")?;
                if pair
                    .split_once('=')
                    .is_some_and(|(name, _)| is_tool_cookie(name))
                {
                    response.headers_mut().append(
                        header::SET_COOKIE,
                        HeaderValue::from_str(&format!(
                            "{pair}; Path={prefix}; HttpOnly; SameSite=Strict{}",
                            if secure { "; Secure" } else { "" }
                        ))
                        .map_err(|_| "cookie")?,
                    );
                }
            }
            "content-type" | "content-disposition" | "content-security-policy" => {
                response.headers_mut().insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| "header")?,
                    HeaderValue::from_str(value).map_err(|_| "header value")?,
                );
            }
            _ => {}
        }
    }
    // Libraries supply their own nonce/hash CSP; legacy distro Adminer needs inline scripts.
    if !response.headers().contains_key("content-security-policy") {
        response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'"));
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::tool_cookies;

    #[test]
    fn forwards_tool_session_variants_without_panel_cookies() {
        assert_eq!(
            tool_cookies(
                "nusa_session=secret; phpMyAdmin=plain; phpMyAdmin_https=tls; __Secure-phpMyAdmin_https=secure; pma_lang=id; __Secure-pma_lang_https=id; adminer_sid=pg; __Secure-nusa_session=secret; phpMyAdmin_fake=bad; malformed"
            ),
            "phpMyAdmin=plain; phpMyAdmin_https=tls; __Secure-phpMyAdmin_https=secure; pma_lang=id; __Secure-pma_lang_https=id; adminer_sid=pg"
        );
    }
}

#[cfg(test)]
#[path = "database_web_origin_tests.rs"]
mod origin_tests;
