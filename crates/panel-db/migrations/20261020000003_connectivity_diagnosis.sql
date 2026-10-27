-- Preserve evidence and identifiers while extending the closed vocabulary.
CREATE TEMP TABLE connectivity_previous_sequence AS SELECT seq FROM sqlite_sequence WHERE name='endpoint_evidence';
CREATE TABLE endpoint_evidence_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    endpoint_id TEXT NOT NULL REFERENCES service_endpoints(id) ON DELETE CASCADE,
    endpoint_revision INTEGER NOT NULL CHECK(endpoint_revision > 0),
    kind TEXT NOT NULL CHECK(kind IN ('installation','service','route','listener','firewall','provider','dns','tls','authentication','external_reachability')),
    observation_point TEXT NOT NULL CHECK(observation_point IN ('local_server','user_client','external_probe')),
    observer_id TEXT NOT NULL CHECK(length(observer_id) BETWEEN 1 AND 128),
    address_family TEXT CHECK(address_family IN ('ipv4','ipv6')),
    observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
    expires_at INTEGER NOT NULL CHECK(expires_at > observed_at),
    outcome TEXT NOT NULL CHECK(outcome IN ('pass','fail','unknown')),
    reason TEXT NOT NULL CHECK(reason IN ('verified','not_tested','unsupported','timeout','service_missing','service_stopped','dns_failure','connector_unavailable','tls_failure','authentication_denied','listener_unavailable','port_blocked','provider_pending','policy_blocked','family_unavailable','connection_refused','agent_unavailable','local_guard_only','network_unavailable')),
    CHECK(kind != 'external_reachability' OR (observation_point != 'local_server' AND address_family IS NOT NULL))
);
INSERT INTO endpoint_evidence_next SELECT * FROM endpoint_evidence;
DROP TABLE endpoint_evidence;
ALTER TABLE endpoint_evidence_next RENAME TO endpoint_evidence;
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT MAX(seq) FROM connectivity_previous_sequence),0)) WHERE name='endpoint_evidence';
DROP TABLE connectivity_previous_sequence;
CREATE INDEX endpoint_evidence_revision ON endpoint_evidence(endpoint_id,endpoint_revision,id);
CREATE TABLE connectivity_diagnosis_leases (
 endpoint_id TEXT PRIMARY KEY REFERENCES service_endpoints(id) ON DELETE CASCADE,
 token TEXT NOT NULL, lease_until INTEGER NOT NULL, cooldown_until INTEGER NOT NULL
);
