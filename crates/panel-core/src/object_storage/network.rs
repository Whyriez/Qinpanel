//! Pure DNS/endpoint admission. A future HTTP adapter must pin the admitted IPs,
//! preserve the TLS hostname and reject redirects; no lookup or HTTP happens here.
use super::{StorageError, identity::*};
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressClass {
    Public,
    Private,
    Forbidden,
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        _ => ip,
    }
}

fn classify_v4(ip: Ipv4Addr) -> AddressClass {
    let [a, b, c, _] = ip.octets();
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        || a == 192 && (b == 0 && c == 0 || b == 88 && c == 99)
        || a == 198 && (b == 18 || b == 19)
        || ip == Ipv4Addr::new(100, 100, 100, 200)
        || ip == Ipv4Addr::new(168, 63, 129, 16)
    {
        return AddressClass::Forbidden;
    }
    if ip.is_private() || a == 100 && (64..=127).contains(&b) {
        AddressClass::Private
    } else {
        AddressClass::Public
    }
}

fn classify(ip: IpAddr) -> AddressClass {
    match canonical(ip) {
        IpAddr::V4(ip) => classify_v4(ip),
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            if ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (segments[0] & 0xffc0) == 0xfe80
                || segments[0] == 0xfd00 && segments[1] == 0x0ec2
                || segments[0] == 0xfd20 && segments[1] == 0x00ce
                || segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8)
                || segments[0] == 0x2002
            {
                return AddressClass::Forbidden;
            }
            if (segments[0] & 0xfe00) == 0xfc00 {
                return AddressClass::Private;
            }
            if (segments[0] & 0xe000) != 0x2000 {
                return AddressClass::Forbidden;
            }
            AddressClass::Public
        }
    }
}

/// Server-side approval bound to exact connection, revision, origin and addresses.
/// It cannot be deserialized from a caller's allow_private flag or request body.
#[derive(Debug, Clone)]
pub struct PrivateEndpointApproval {
    configuration_fingerprint: String,
    connection_id: ConnectionId,
    connection_revision: u64,
    owner: TenantId,
    endpoint: StorageEndpoint,
    addresses: Vec<IpAddr>,
    expires_at: u64,
    observed_at: u64,
}
impl PrivateEndpointApproval {
    pub fn approve(
        actor: StorageActor,
        connection: &StorageConnection,
        addresses: Vec<IpAddr>,
        now: u64,
        expires_at: u64,
    ) -> Result<Self, StorageError> {
        actor.require_admin()?;
        connection.config.validate(connection.owner)?;
        if connection.revision == 0
            || now == 0
            || expires_at <= now
            || expires_at - now > 300
            || addresses.is_empty()
            || addresses.len() > 16
            || addresses
                .iter()
                .any(|ip| classify(*ip) != AddressClass::Private)
        {
            return Err(StorageError::ForbiddenEndpoint);
        }
        Ok(Self {
            configuration_fingerprint: connection.fingerprint()?,
            connection_id: connection.id,
            connection_revision: connection.revision,
            owner: connection.owner,
            endpoint: connection.config.endpoint.clone(),
            addresses: addresses.into_iter().map(canonical).collect(),
            expires_at,
            observed_at: now,
        })
    }
}

/// Only this guard constructs dial targets. Never re-resolve its hostname when
/// connecting, and re-check after expiry or a connection/credential revision.
#[derive(Debug, Clone)]
pub struct ValidatedDialTargets {
    configuration_fingerprint: String,
    connection_id: ConnectionId,
    endpoint: StorageEndpoint,
    addresses: Vec<IpAddr>,
    expires_at: u64,
    observed_at: u64,
    connection_revision: u64,
    credential_revision: u64,
}
impl ValidatedDialTargets {
    pub fn require_current(
        &self,
        connection: &StorageConnection,
        now: u64,
    ) -> Result<(), StorageError> {
        if now < self.observed_at
            || now >= self.expires_at
            || connection.id != self.connection_id
            || connection.revision != self.connection_revision
            || connection.credential_revision() != self.credential_revision
            || connection.fingerprint()? != self.configuration_fingerprint
        {
            return Err(StorageError::StaleEvidence);
        }
        Ok(())
    }
    pub fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }
    pub fn endpoint(&self) -> &StorageEndpoint {
        &self.endpoint
    }
    pub fn addresses(&self) -> &[IpAddr] {
        &self.addresses
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub fn connection_revision(&self) -> u64 {
        self.connection_revision
    }
    pub fn credential_revision(&self) -> u64 {
        self.credential_revision
    }
}

pub fn validate_dial_targets(
    connection: &StorageConnection,
    panel_endpoint: &StorageEndpoint,
    addresses: &[IpAddr],
    approval: Option<&PrivateEndpointApproval>,
    now: u64,
) -> Result<ValidatedDialTargets, StorageError> {
    connection.config.validate(connection.owner)?;
    connection
        .config
        .endpoint
        .require_separate_origin(panel_endpoint)?;
    if connection.revision == 0 || now == 0 || addresses.is_empty() || addresses.len() > 16 {
        return Err(StorageError::ForbiddenEndpoint);
    }
    let addresses: Vec<_> = addresses.iter().copied().map(canonical).collect();
    if let Ok(literal) = connection.config.endpoint.host.as_str().parse::<IpAddr>()
        && (addresses.len() != 1 || addresses[0] != canonical(literal))
    {
        return Err(StorageError::ForbiddenEndpoint);
    }
    let mut expires_at = now.checked_add(60).ok_or(StorageError::ForbiddenEndpoint)?;
    for ip in &addresses {
        match classify(*ip) {
            AddressClass::Forbidden => return Err(StorageError::ForbiddenEndpoint),
            AddressClass::Private => {
                let approval = approval.ok_or(StorageError::ForbiddenEndpoint)?;
                if approval.connection_id != connection.id
                    || approval.connection_revision != connection.revision
                    || approval.owner != connection.owner
                    || approval.endpoint != connection.config.endpoint
                    || approval.configuration_fingerprint != connection.fingerprint()?
                    || now < approval.observed_at
                    || now >= approval.expires_at
                    || !approval.addresses.contains(ip)
                {
                    return Err(StorageError::ForbiddenEndpoint);
                }
                expires_at = expires_at.min(approval.expires_at);
            }
            AddressClass::Public => {}
        }
    }
    Ok(ValidatedDialTargets {
        configuration_fingerprint: connection.fingerprint()?,
        connection_id: connection.id,
        endpoint: connection.config.endpoint.clone(),
        addresses,
        expires_at,
        observed_at: now,
        connection_revision: connection.revision,
        credential_revision: connection.credential_revision(),
    })
}
