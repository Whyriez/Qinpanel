-- Additive metadata only. Existing network.public_ipv4/ipv6 remain the source;
-- no inferred endpoints, listeners, routes, firewall or credentials are created.
CREATE TABLE IF NOT EXISTS service_endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    service TEXT NOT NULL CHECK(service IN ('panel','website','ssh','database','imap','submission','smtp_inbound')),
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('host','project','engine')),
    site_id INTEGER REFERENCES sites(id) ON DELETE CASCADE,
    engine TEXT REFERENCES database_servers(engine),
    transport TEXT NOT NULL CHECK(transport IN ('direct','cloudflare_http','cloudflare_access','private_network','provider')),
    host TEXT NOT NULL CHECK(length(host) BETWEEN 1 AND 253),
    port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
    tls_mode TEXT NOT NULL CHECK(tls_mode IN ('none','tls','starttls','ssh')),
    certificate_id INTEGER REFERENCES certificates(id),
    -- Reserved opaque bindings, never credentials. Only a future verified binder
    -- may populate these; the public repository does not select or accept them.
    route_id TEXT,
    connection_id TEXT,
    secret_ref TEXT,
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    is_primary INTEGER NOT NULL CHECK(is_primary IN (0,1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK(is_primary = 0 OR enabled = 1),
    CHECK((scope_kind = 'host' AND site_id IS NULL AND engine IS NULL)
       OR (scope_kind = 'project' AND site_id IS NOT NULL AND engine IS NULL)
       OR (scope_kind = 'engine' AND site_id IS NULL AND engine IS NOT NULL)),
    CHECK((service = 'panel' AND scope_kind = 'host')
       OR (service = 'website' AND scope_kind = 'project')
       OR (service = 'ssh' AND scope_kind IN ('host','project'))
       OR (service = 'database' AND scope_kind = 'engine')
       OR (service IN ('imap','submission','smtp_inbound') AND scope_kind = 'project'))
);
CREATE UNIQUE INDEX IF NOT EXISTS service_endpoints_primary
    ON service_endpoints(service, scope_kind, COALESCE(site_id,0), COALESCE(engine,''))
    WHERE is_primary = 1;
CREATE INDEX IF NOT EXISTS service_endpoints_project ON service_endpoints(site_id);

CREATE TABLE IF NOT EXISTS endpoint_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    endpoint_id TEXT NOT NULL REFERENCES service_endpoints(id) ON DELETE CASCADE,
    endpoint_revision INTEGER NOT NULL CHECK(endpoint_revision > 0),
    kind TEXT NOT NULL CHECK(kind IN ('installation','service','route','dns','tls','authentication','external_reachability')),
    observation_point TEXT NOT NULL CHECK(observation_point IN ('local_server','user_client','external_probe')),
    observer_id TEXT NOT NULL CHECK(length(observer_id) BETWEEN 1 AND 128),
    address_family TEXT CHECK(address_family IN ('ipv4','ipv6')),
    observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
    expires_at INTEGER NOT NULL CHECK(expires_at > observed_at),
    outcome TEXT NOT NULL CHECK(outcome IN ('pass','fail','unknown')),
    reason TEXT NOT NULL CHECK(reason IN ('verified','not_tested','unsupported','timeout','service_missing','service_stopped','dns_failure','connector_unavailable','tls_failure','authentication_denied','listener_unavailable','port_blocked','provider_pending')),
    CHECK(kind != 'external_reachability' OR (observation_point != 'local_server' AND address_family IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS endpoint_evidence_revision ON endpoint_evidence(endpoint_id,endpoint_revision,id);
