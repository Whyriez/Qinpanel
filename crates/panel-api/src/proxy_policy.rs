//! Authorize the destination that Nginx/the root probe would connect to.
use axum::http::{StatusCode, Uri};
use panel_db::pool::DbPool;
use std::net::{IpAddr, Ipv4Addr};

type Error = (StatusCode, String);
fn denied() -> Error {
    (StatusCode::FORBIDDEN, "Upstream lokal harus runtime aktif milik tenant yang sama; upstream eksternal harus memiliki IP publik yang aman.".into())
}

pub(crate) struct AdmittedUpstream {
    pub url: String,
    pub authority: Option<String>,
    pub tls_name: Option<String>,
}

impl AdmittedUpstream {
    pub fn probe_url(&self) -> String {
        self.url
            .strip_prefix("http://unix:")
            .map_or_else(|| self.url.clone(), |socket| format!("unix:{socket}"))
    }
}

pub(crate) async fn admit(pool: &DbPool, owner: i64, url: &str) -> Result<AdmittedUpstream, Error> {
    panel_webserver::template::validate_reverse_proxy_upstream_url(url, true)
        .map_err(|_| denied())?;
    if let Some(socket) = url
        .strip_prefix("unix:")
        .or_else(|| url.strip_prefix("http://unix:"))
    {
        let socket = socket.trim_end_matches(':');
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM sites WHERE user_id=? AND status='active' AND (kind='python' OR runtime='python')")
            .bind(owner).fetch_all(pool).await.map_err(|_| denied())?;
        if !ids
            .iter()
            .any(|id| socket == format!("/run/panel/python/site_{id}/app.sock"))
        {
            return Err(denied());
        }
        return Ok(AdmittedUpstream {
            url: format!("http://unix:{socket}"),
            authority: None,
            tls_name: None,
        });
    }
    let uri: Uri = url.parse().map_err(|_| denied())?;
    let authority = uri.authority().ok_or_else(denied)?;
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = authority
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        });
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else if host.eq_ignore_ascii_case("localhost") {
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
    } else {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| denied())?
        .map_err(|_| denied())?
        .map(|address| address.ip())
        .collect::<Vec<_>>()
    };
    admit_addresses(pool, owner, &addresses, port).await?;
    let ip = addresses.first().ok_or_else(denied)?;
    let target = if ip.is_ipv6() {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    };
    Ok(AdmittedUpstream {
        url: format!(
            "{}://{}{}",
            uri.scheme_str().ok_or_else(denied)?,
            target,
            uri.path_and_query().map_or("/", |p| p.as_str())
        ),
        authority: Some(authority.as_str().into()),
        tls_name: Some(host.into()),
    })
}

async fn admit_addresses(
    pool: &DbPool,
    owner: i64,
    addresses: &[IpAddr],
    port: u16,
) -> Result<(), Error> {
    if addresses.is_empty() || addresses.len() > 64 {
        return Err(denied());
    }
    let loopback = |ip: IpAddr| match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or(ip.is_loopback(), |ip| ip.is_loopback()),
        IpAddr::V4(ip) => ip.is_loopback(),
    };
    if addresses.iter().any(|ip| loopback(*ip)) {
        if !(20000..=29999).contains(&port) {
            return Err(denied());
        }
        if !addresses.iter().all(|ip| loopback(*ip)) {
            return Err(denied());
        }
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT user_id,settings_json FROM sites WHERE status='active' AND (kind IN ('nodejs','python') OR runtime IN ('nodejs','python'))")
            .fetch_all(pool).await.map_err(|_| denied())?;
        let owners = rows
            .iter()
            .filter_map(|(id, settings)| {
                let settings: serde_json::Value = serde_json::from_str(settings).ok()?;
                (settings["port"].as_u64() == Some(u64::from(port))).then_some(*id)
            })
            .collect::<Vec<_>>();
        if owners.is_empty() || owners.iter().any(|id| *id != owner) {
            return Err(denied());
        }
    } else if !addresses
        .iter()
        .all(|ip| panel_core::object_storage::network::is_public_destination(*ip))
    {
        return Err(denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn proxy_targets_require_exact_ownership_and_public_dns() {
        let pool = panel_db::pool::init_pool("sqlite::memory:").await.unwrap();
        panel_db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username,password_hash,role,status,created_at,updated_at) VALUES(1,'one','fixture','user','active',1,1),(2,'two','fixture','user','active',1,1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sites(id,user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(10,1,'nodejs','/fixture','active','{\"port\":20050}',1,1),(20,2,'python','/fixture','active','{\"port\":20051}',1,1)").execute(&pool).await.unwrap();
        assert!(admit(&pool, 1, "http://127.0.0.1:20050").await.is_ok());
        assert!(
            crate::node_site::validate_runtime_port(&pool, Some(10), 20050)
                .await
                .is_ok()
        );
        assert!(
            crate::node_site::validate_runtime_port(&pool, None, 20050)
                .await
                .is_err()
        );
        for port in [6379, 7700, 17890, 19999, 30000] {
            assert!(
                crate::node_site::validate_runtime_port(&pool, Some(10), port)
                    .await
                    .is_err()
            );
        }
        for url in [
            "http://127.0.0.1:20051",
            "http://localhost:3000",
            "unix:/run/panel/agent.sock",
            "http://unix:/run/panel/python/site_20/app.sock",
            "http://[::ffff:169.254.169.254]/",
            "http://100.100.100.200/",
        ] {
            assert!(admit(&pool, 1, url).await.is_err(), "{url}");
        }
        assert!(
            admit(&pool, 2, "http://unix:/run/panel/python/site_20/app.sock")
                .await
                .is_ok()
        );
        assert_eq!(
            admit(&pool, 2, "unix:/run/panel/python/site_20/app.sock")
                .await
                .unwrap()
                .probe_url(),
            "unix:/run/panel/python/site_20/app.sock"
        );
        let public = "93.184.216.34".parse().unwrap();
        assert!(admit_addresses(&pool, 1, &[public], 443).await.is_ok());
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "::ffff:169.254.169.254",
        ] {
            assert!(
                admit_addresses(&pool, 1, &[public, ip.parse().unwrap()], 443)
                    .await
                    .is_err()
            );
        }
        sqlx::query("INSERT INTO sites(id,user_id,kind,docroot,status,settings_json,created_at,updated_at) VALUES(30,1,'proxy','/fixture','active','{\"upstream_url\":\"http://localhost:20050\"}',1,1)")
            .execute(&pool).await.unwrap();
        assert!(
            crate::node_site::validate_runtime_port(&pool, Some(10), 20050)
                .await
                .is_ok()
        );
        sqlx::query("DELETE FROM sites WHERE id=10")
            .execute(&pool)
            .await
            .unwrap();
        // The proxy survives its target. Neither a new runtime nor another
        // tenant changing an existing runtime may inherit that connection.
        assert!(
            crate::node_site::validate_runtime_port(&pool, None, 20050)
                .await
                .is_err()
        );
        assert!(
            crate::node_site::validate_runtime_port(&pool, Some(20), 20050)
                .await
                .is_err()
        );
        sqlx::query("UPDATE sites SET settings_json='{\"upstream_url\":\"http://localhost:20000\"}' WHERE id=30")
            .execute(&pool).await.unwrap();
        assert_ne!(
            crate::node_site::allocate_node_site_port(&pool)
                .await
                .unwrap(),
            20000
        );
    }
}
