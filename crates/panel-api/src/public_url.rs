//! One origin model for browser requests, cookies, WebSockets and absolute links.
use crate::routes::AppState;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use panel_core::connectivity::EndpointHost;
use std::{net::SocketAddr, num::NonZeroU16};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelOrigin {
    pub secure: bool,
    pub host: String,
    pub port: NonZeroU16,
    explicit_port: bool,
}
impl PanelOrigin {
    pub fn parse(url: &str) -> Result<Self, &'static str> {
        let (scheme, authority) = url
            .split_once("://")
            .ok_or("URL panel harus HTTP atau HTTPS tanpa path.")?;
        if !matches!(scheme, "http" | "https") {
            return Err("Skema URL panel tidak valid.");
        }
        Self::from_authority(authority, scheme == "https")
    }
    pub fn from_authority(value: &str, secure: bool) -> Result<Self, &'static str> {
        if value.is_empty()
            || value.len() > 300
            || value
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            || value.contains(['@', '/', '\\', '?', '#', '%', ','])
        {
            return Err("Host panel tidak valid.");
        }
        let authority: axum::http::uri::Authority =
            value.parse().map_err(|_| "Host/port panel tidak valid.")?;
        let host = authority.host();
        let host = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host);
        if host.contains(':') && !value.starts_with('[') {
            return Err("IPv6 pada URL harus memakai bracket.");
        }
        // Runtime requests also serve the development UI on localhost. Keep
        // this exact exception outside EndpointHost's public metadata/probe model.
        let host = if host.eq_ignore_ascii_case("localhost") {
            "localhost".to_owned()
        } else {
            EndpointHost::parse(host)
                .map_err(|_| "Hostname/IP panel tidak valid.")?
                .as_str()
                .to_owned()
        };
        let explicit_port = if value.starts_with('[') {
            value
                .split_once(']')
                .is_some_and(|(_, suffix)| suffix.starts_with(':'))
        } else {
            value.contains(':')
        };
        if value.ends_with(':') || (explicit_port && authority.port_u16().is_none()) {
            return Err("Port panel tidak valid.");
        }
        let port = NonZeroU16::new(
            authority
                .port_u16()
                .unwrap_or(if secure { 443 } else { 80 }),
        )
        .ok_or("Port panel tidak valid.")?;
        Ok(Self {
            secure,
            host,
            port,
            explicit_port,
        })
    }
    pub fn authority(&self) -> String {
        if self.explicit_port {
            if self.host.contains(':') {
                format!("[{}]:{}", self.host, self.port)
            } else {
                format!("{}:{}", self.host, self.port)
            }
        } else if self.host.as_str().contains(':') {
            format!("[{}]", self.host.as_str())
        } else {
            self.host.as_str().into()
        }
    }
    pub fn url(&self) -> String {
        format!(
            "{}://{}",
            if self.secure { "https" } else { "http" },
            self.authority()
        )
    }
    pub fn same(&self, other: &Self) -> bool {
        self.secure == other.secure && self.host == other.host && self.port == other.port
    }
    /// Only internal panel paths. No scheme-relative, encoded authority, dot
    /// traversal or control characters; an event can never replace the origin.
    pub fn action_url(&self, path: &str) -> Result<String, &'static str> {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.len() > 2048
            || path.contains(['\\', '%'])
            || path.chars().any(|c| c.is_control() || c.is_whitespace())
            || path
                .split(['?', '#'])
                .next()
                .is_none_or(|p| p.split('/').any(|s| matches!(s, "." | "..")))
        {
            return Err("Tautan aksi panel tidak valid.");
        }
        Ok(format!("{}{path}", self.url()))
    }
    pub fn matches_origin(&self, headers: &HeaderMap, required: bool) -> bool {
        if single(headers, "sec-fetch-site") == Some("cross-site") {
            return false;
        }
        match single(headers, "origin") {
            Some(value) => Self::parse(value).is_ok_and(|origin| self.same(&origin)),
            None => !required && !headers.contains_key(header::ORIGIN),
        }
    }
}
pub fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}
/// Only a known loopback peer can assert upstream TLS. Nginx overwrites XFP
/// and only relays CF-Visitor from a loopback connector under a private name.
pub fn request_https(request: &Request) -> bool {
    if !request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback())
    {
        return false;
    }
    if single(request.headers(), "x-forwarded-proto") == Some("https") {
        return true;
    }
    single(request.headers(), "x-qpanel-cf-visitor")
        .filter(|s| s.len() <= 128)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .is_some_and(|v| {
            v.as_object().is_some_and(|o| o.len() == 1)
                && v.get("scheme").and_then(|s| s.as_str()) == Some("https")
        })
}
#[derive(Debug, Clone)]
pub struct PublicRequest {
    pub origin: PanelOrigin,
    pub cookie_secure: bool,
}

fn failure(status: StatusCode, detail: &'static str) -> Response {
    (status,[(header::CONTENT_TYPE,"application/problem+json")],axum::Json(serde_json::json!({"type":"about:blank","title":"Alamat panel tidak valid","status":status.as_u16(),"detail":detail}))).into_response()
}
pub async fn middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let required = request.uri().path().starts_with("/ws/");
    let Some(host) = single(request.headers(), "host") else {
        // Existing non-browser in-process clients do not supply Host. Actual
        // HTTP/1.1 servers require it; never allow ambiguous or browser origins.
        if request.headers().contains_key(header::HOST)
            || request.headers().contains_key(header::ORIGIN)
            || required
        {
            return failure(StatusCode::BAD_REQUEST, "Host tunggal diperlukan.");
        }
        return next.run(request).await;
    };
    let origin = match PanelOrigin::from_authority(host, request_https(&request)) {
        Ok(origin) => origin,
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "Hostname/IP atau port panel tidak valid.",
            );
        }
    };
    if !((matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) && !required
        && !request.headers().contains_key(header::ORIGIN))
        || origin.matches_origin(request.headers(), required))
    {
        return failure(StatusCode::FORBIDDEN, "Permintaan lintas asal ditolak.");
    }
    if request.uri().path() == "/connectivity/verify" && request.method() == axum::http::Method::GET
    {
        let rows: Result<Vec<(String,i64,String)>,_>=sqlx::query_as("SELECT host,port,tls_mode FROM service_endpoints WHERE service='panel' AND scope_kind='host' AND enabled=1").fetch_all(&state.pool).await;
        let registered = rows.is_ok_and(|rows| {
            rows.iter().any(|(host, port, tls)| {
                if *port == i64::from(panel_core::production::INTERNAL_API_PORT) {
                    return false;
                }
                let authority = if host.contains(':') {
                    format!("[{host}]:{port}")
                } else {
                    format!("{host}:{port}")
                };
                PanelOrigin::from_authority(&authority, tls == "tls")
                    .is_ok_and(|candidate| candidate.same(&origin))
            })
        });
        if !registered {
            return failure(
                StatusCode::NOT_FOUND,
                "Alamat Panel aktif belum terdaftar untuk pemeriksaan ini.",
            );
        }
        return axum::response::Html("<!doctype html><html lang=\"id\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Pemeriksaan alamat Qinpanel</title><link rel=\"stylesheet\" href=\"/panel-url-check.css\"><main><p>Qinpanel</p><h1>Alamat panel dapat dibuka</h1><p>Browser Anda berhasil mencapai panel melalui alamat ini. Kembali ke pengaturan Konektivitas untuk mengonfirmasikan aktivasi URL.</p><p>Pemeriksaan ini tidak mengubah layanan atau mengaktifkan alamat. Akses SSH, database dan email memerlukan pemeriksaan masing-masing.</p></main></html>").into_response();
    }
    if !matches!(
        request.uri().path(),
        "/healthz" | "/panel-url-check.css" | "/fonts/plus-jakarta-sans-latin.woff2"
    ) {
        match allowed(&state.pool, &origin).await {
            Ok(true) => {}
            Ok(false) => {
                return failure(
                    StatusCode::MISDIRECTED_REQUEST,
                    "Alamat belum diaktifkan. Gunakan URL panel yang telah diatur.",
                );
            }
            Err(_) => {
                return failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Kebijakan URL panel belum dapat dimuat.",
                );
            }
        }
    }
    let cookie_secure = origin.secure || state.auth_config.cookie_secure == Some(true);
    request.extensions_mut().insert(PublicRequest {
        origin,
        cookie_secure,
    });
    let mut response = next.run(request).await;
    // Covers login, logout, password rotation and impersonation without letting
    // an old install-wide cookie setting downgrade HTTPS offload.
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .cloned()
        .collect();
    response.headers_mut().remove(header::SET_COOKIE);
    for cookie in cookies {
        let rewritten = cookie
            .to_str()
            .ok()
            .filter(|s| {
                s.split_once('=')
                    .is_some_and(|(name, _)| name == crate::auth::SESSION_COOKIE_NAME)
            })
            .and_then(|s| {
                let mut parts: Vec<_> = s
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.eq_ignore_ascii_case("secure"))
                    .collect();
                if cookie_secure {
                    parts.push("Secure");
                }
                HeaderValue::from_str(&parts.join("; ")).ok()
            })
            .unwrap_or(cookie);
        response.headers_mut().append(header::SET_COOKIE, rewritten);
    }
    response
}
pub async fn canonical(pool: &panel_db::DbPool) -> Result<Option<PanelOrigin>, panel_db::DbError> {
    let url: Option<String> =
        sqlx::query_scalar("SELECT canonical_url FROM panel_url_policy WHERE id=1")
            .fetch_optional(pool)
            .await?;
    url.map(|s| {
        PanelOrigin::parse(&s)
            .map_err(|_| panel_db::DbError::Validation("URL panel tersimpan tidak valid.".into()))
    })
    .transpose()
}
async fn allowed(pool: &panel_db::DbPool, origin: &PanelOrigin) -> Result<bool, panel_db::DbError> {
    let Some(main) = canonical(pool).await? else {
        return Ok(true);
    };
    if main.same(origin) {
        return Ok(true);
    }
    let aliases: Vec<String> = sqlx::query_scalar("SELECT origin FROM panel_url_aliases")
        .fetch_all(pool)
        .await?;
    Ok(aliases
        .iter()
        .any(|s| PanelOrigin::parse(s).is_ok_and(|alias| alias.same(origin))))
}
