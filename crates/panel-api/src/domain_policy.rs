//! Panel cookies are host-scoped, not port-scoped. Tenant code must never
//! share the administrator hostname, including on another listener/port.
use crate::public_url::{PanelOrigin, PublicRequest};
use axum::http::StatusCode;
use panel_db::pool::DbPool;

pub(crate) async fn require_tenant_domain(
    pool: &DbPool,
    domain: &str,
    current: Option<&PublicRequest>,
) -> Result<(), (StatusCode, String)> {
    let failure = || {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Reservasi hostname panel tidak dapat diverifikasi".into(),
        )
    };
    let hosts: Vec<String> = sqlx::query_scalar("SELECT host FROM service_endpoints WHERE service='panel' UNION SELECT canonical_url FROM panel_url_policy UNION SELECT origin FROM panel_url_aliases")
        .fetch_all(pool).await.map_err(|_| failure())?;
    if current.is_some_and(|context| context.origin.host.eq_ignore_ascii_case(domain))
        || hosts.iter().any(|value| {
            value.eq_ignore_ascii_case(domain)
                || PanelOrigin::parse(value)
                    .is_ok_and(|origin| origin.host.eq_ignore_ascii_case(domain))
        })
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Hostname panel dicadangkan untuk administrasi dan tidak boleh dipakai project tenant"
                .into(),
        ));
    }
    Ok(())
}
