//! Cloudflare API access and scoped local connectors. Credentials never leave the agent.
use crate::{AgentError, crypto, system::SystemRunner};
use panel_core::DomainName;
use panel_proto::{CloudflareDnsRecord, CloudflareRequest, TunnelAction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const DIR: &str = "/etc/panel/cloudflare";
const CREDENTIALS: &str = "/etc/panel/cloudflare/credentials.enc";
const MANIFEST: &str = "/etc/panel/cloudflare/managed.json";

#[derive(Serialize, Deserialize)]
pub(crate) struct Credentials {
    pub(crate) account_id: String,
    pub(crate) api_token: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    account_id: String,
    tunnels: BTreeMap<String, ManagedTunnel>,
}
#[derive(Default, Serialize, Deserialize)]
struct ManagedTunnel {
    routes: BTreeMap<String, Route>,
}
#[derive(Serialize, Deserialize)]
struct Route {
    zone_id: String,
    record_id: String,
    #[serde(default)]
    tls: bool,
}

fn invalid(message: &str) -> AgentError {
    AgentError::InvalidInput(message.into())
}
fn conflict(message: &str) -> AgentError {
    AgentError::SecurityViolation(message.into())
}
fn id(value: &str) -> Result<(), AgentError> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("ID Cloudflare harus 32 karakter heksadesimal."));
    }
    Ok(())
}
fn tunnel_id(value: &str) -> Result<(), AgentError> {
    if !uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        return Err(invalid("ID tunnel tidak valid."));
    }
    Ok(())
}
fn domain(value: &str) -> Result<String, AgentError> {
    DomainName::parse(value)
        .map(|d| d.as_ascii().to_owned())
        .map_err(|_| invalid("Nama domain tidak valid."))
}
fn directory(runner: &dyn SystemRunner) -> Result<(), AgentError> {
    runner.create_dir_all(Path::new(DIR), 0o700, 0, 0)
}
fn manifest(runner: &dyn SystemRunner) -> Result<Manifest, AgentError> {
    if !runner.path_exists(Path::new(MANIFEST)) {
        return Ok(Manifest::default());
    }
    serde_json::from_slice(&runner.read_file(Path::new(MANIFEST))?).map_err(|_| {
        invalid("Inventaris tunnel lokal rusak; pulihkan cadangan sebelum melanjutkan.")
    })
}
fn save_manifest(runner: &dyn SystemRunner, value: &Manifest) -> Result<(), AgentError> {
    write_atomic(
        runner,
        MANIFEST,
        &serde_json::to_vec(value).map_err(|_| invalid("Inventaris gagal diserialisasi."))?,
    )
}
fn write_atomic(runner: &dyn SystemRunner, path: &str, bytes: &[u8]) -> Result<(), AgentError> {
    let temp = format!("{path}.new");
    runner.write_file(Path::new(&temp), bytes, 0o600)?;
    runner.move_path(Path::new(&temp), Path::new(path))
}
fn credentials(runner: &dyn SystemRunner) -> Result<Credentials, AgentError> {
    if !runner.path_exists(Path::new(CREDENTIALS)) {
        return Err(invalid("Hubungkan akun Cloudflare terlebih dahulu."));
    }
    let ciphertext = String::from_utf8(runner.read_file(Path::new(CREDENTIALS))?)
        .map_err(|_| invalid("Kredensial tersimpan tidak valid."))?;
    if !ciphertext.starts_with(crypto::CIPHER_PREFIX) {
        return Err(conflict("Kredensial wajib terenkripsi."));
    }
    let key = crypto::get_or_create_master_key(runner, None)?;
    serde_json::from_str(&crypto::decrypt_value(&key, &ciphertext)?)
        .map_err(|_| invalid("Kredensial tersimpan tidak valid."))
}

// Private files keep bearer tokens out of process arguments, unit files, and logs.
struct Exchange<'a> {
    runner: &'a dyn SystemRunner,
    config: String,
    body: String,
}
impl Drop for Exchange<'_> {
    fn drop(&mut self) {
        let _ = self.runner.remove_file(Path::new(&self.config));
        let _ = self.runner.remove_file(Path::new(&self.body));
    }
}
pub(crate) fn api(
    runner: &dyn SystemRunner,
    credentials: &Credentials,
    method: &str,
    resource: &str,
    body: Option<Value>,
) -> Result<Value, AgentError> {
    directory(runner)?;
    let nonce = uuid::Uuid::new_v4();
    let exchange = Exchange {
        runner,
        config: format!("{DIR}/exchange-{nonce}.conf"),
        body: format!("{DIR}/exchange-{nonce}.json"),
    };
    runner.write_file(
        Path::new(&exchange.config),
        format!(
            "header = \"Authorization: Bearer {}\"\n",
            credentials.api_token
        )
        .as_bytes(),
        0o600,
    )?;
    let url = format!("https://api.cloudflare.com/client/v4/{resource}");
    let mut args = vec![
        "--disable",
        "--silent",
        "--show-error",
        "--proto",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        "30",
        "--max-filesize",
        "2097152",
        "--config",
        &exchange.config,
        "--request",
        method,
        "--header",
        "Content-Type: application/json",
        "--write-out",
        "\n%{http_code}",
        &url,
    ];
    let body_arg = format!("@{}", exchange.body);
    if let Some(body) = body {
        runner.write_file(
            Path::new(&exchange.body),
            body.to_string().as_bytes(),
            0o600,
        )?;
        args.extend(["--data-binary", &body_arg]);
    }
    let output = runner.run_command("curl", &args).map_err(|_| {
        invalid("Cloudflare tidak dapat dihubungi. Periksa koneksi server lalu coba lagi.")
    })?;
    if !output.status.success() {
        return Err(invalid(
            "Permintaan Cloudflare gagal atau melewati batas waktu. Muat ulang status sebelum mencoba kembali.",
        ));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| invalid("Respons Cloudflare bukan UTF-8."))?;
    let (body, http) = text
        .rsplit_once('\n')
        .ok_or_else(|| invalid("Respons Cloudflare tidak lengkap."))?;
    let response: Value =
        serde_json::from_str(body).map_err(|_| invalid("Respons Cloudflare tidak valid."))?;
    if method == "DELETE" && http == "404" {
        return Ok(json!({"success":true,"result":null}));
    }
    if response["success"] != true || !http.starts_with('2') {
        let codes: Vec<i64> = response["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["code"].as_i64())
            .collect();
        return Err(invalid(&format!(
            "Cloudflare menolak permintaan (HTTP {http}, kode {codes:?}). Periksa izin token, akses akun/domain, dan konflik DNS."
        )));
    }
    Ok(response)
}
fn page(value: u32) -> Result<u32, AgentError> {
    if !(1..=10000).contains(&value) {
        return Err(invalid("Halaman tidak valid."));
    }
    Ok(value)
}
fn filtered(value: &Value, keys: &[&str]) -> Value {
    let mut result = serde_json::Map::new();
    for key in keys {
        if let Some(value) = value.get(*key) {
            result.insert((*key).into(), value.clone());
        }
    }
    Value::Object(result)
}
fn list(response: Value, keys: &[&str]) -> Value {
    let items = response["result"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| filtered(v, keys))
        .collect::<Vec<_>>();
    let info = &response["result_info"];
    let page = info["page"].as_u64().unwrap_or(1);
    let total = info["total_count"].as_u64().unwrap_or(items.len() as u64);
    let size = info["per_page"]
        .as_u64()
        .unwrap_or(items.len() as u64)
        .max(1);
    let pages = info["total_pages"]
        .as_u64()
        .unwrap_or(total.div_ceil(size))
        .max(1);
    json!({"items":items,"pagination":{"page":page,"total_pages":pages,"total_count":total}})
}

fn ingress_rule(hostname: &str, tls: bool) -> Value {
    json!({"hostname":hostname,"service": if tls {"https://127.0.0.1:443"} else {"http://127.0.0.1:80"},"originRequest":{"httpHostHeader":hostname,"originServerName":hostname,"noTLSVerify":false}})
}

fn validate_ingress_boundary(ingress: &[Value]) -> Result<(), AgentError> {
    let rejects = |rule: &Value| {
        rule["service"]
            .as_str()
            .and_then(|service| service.strip_prefix("http_status:"))
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| (400..=599).contains(&status))
    };
    let last = ingress
        .last()
        .ok_or_else(|| invalid("Konfigurasi ingress kosong."))?;
    if last.get("hostname").is_some() || last.get("path").is_some() || !rejects(last) {
        return Err(conflict(
            "Catch-all tunnel harus menolak hostname tidak dikenal, misalnya http_status:404. Konfigurasi tunnel dipertahankan; jangan arahkan catch-all ke origin web.",
        ));
    }
    for rule in &ingress[..ingress.len() - 1] {
        let exact_host = rule["hostname"]
            .as_str()
            .is_some_and(|host| !host.is_empty() && !host.contains('*'));
        if !exact_host && !rejects(rule) {
            return Err(conflict(
                "Tunnel memuat wildcard atau rute tanpa hostname yang dapat mengekspos origin. Gunakan rute hostname eksplisit; konfigurasi tidak diubah.",
            ));
        }
    }
    Ok(())
}
fn zone(
    runner: &dyn SystemRunner,
    cred: &Credentials,
    zone_id: &str,
) -> Result<String, AgentError> {
    id(zone_id)?;
    let response = api(runner, cred, "GET", &format!("zones/{zone_id}"), None)?;
    if response["result"]["account"]["id"] != cred.account_id {
        return Err(conflict(
            "Domain bukan milik akun Cloudflare yang terhubung.",
        ));
    }
    response["result"]["name"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("Nama zona tidak ditemukan."))
}
fn name_in_zone(name: &str, zone: &str) -> Result<String, AgentError> {
    let name = if name == "@" {
        zone.to_owned()
    } else {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if name.contains('_') || name.starts_with("*.") {
            if name.len() > 253
                || name.split('.').enumerate().any(|(i, label)| {
                    label.is_empty()
                        || label.len() > 63
                        || (label != "*" || i != 0)
                            && !label
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                })
            {
                return Err(invalid("Nama DNS record tidak valid."));
            }
            name
        } else {
            domain(&name)?
        }
    };
    if name != zone && !name.ends_with(&format!(".{zone}")) {
        return Err(invalid("Nama record harus berada di domain yang dipilih."));
    }
    Ok(name)
}
fn record_body(record: &CloudflareDnsRecord, zone: &str) -> Result<Value, AgentError> {
    let name = name_in_zone(&record.name, zone)?;
    let kind = record.kind.as_str();
    if !["A", "AAAA", "CNAME", "TXT", "MX", "NS"].contains(&kind) {
        return Err(invalid("Jenis record belum didukung."));
    }
    if record.ttl != 1 && !(60..=86400).contains(&record.ttl) {
        return Err(invalid("TTL harus otomatis (1) atau 60–86400 detik."));
    }
    if record.proxied && !["A", "AAAA", "CNAME"].contains(&kind) {
        return Err(invalid("Proxy hanya tersedia untuk A, AAAA, atau CNAME."));
    }
    if record.content.is_empty()
        || record.content.len() > 4096
        || record.content.chars().any(char::is_control)
    {
        return Err(invalid("Isi record tidak valid."));
    }
    match kind {
        "A" => {
            record
                .content
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| invalid("Alamat IPv4 tidak valid."))?;
        }
        "AAAA" => {
            record
                .content
                .parse::<std::net::Ipv6Addr>()
                .map_err(|_| invalid("Alamat IPv6 tidak valid."))?;
        }
        "CNAME" | "MX" | "NS" => {
            domain(&record.content)?;
        }
        _ => {}
    }
    let mut body = json!({"type": kind, "name": name, "content": record.content, "ttl": if record.proxied {1} else {record.ttl}, "proxied": record.proxied});
    if kind == "MX" {
        body["priority"] = json!(
            record
                .priority
                .ok_or_else(|| invalid("Prioritas wajib untuk MX."))?
        );
    }
    Ok(body)
}
fn owned_record(manifest: &Manifest, record_id: &str) -> bool {
    manifest
        .tunnels
        .values()
        .any(|t| t.routes.values().any(|r| r.record_id == record_id))
}
fn unit(tunnel: &str) -> String {
    format!("qpanel-cloudflared-{tunnel}.service")
}
fn token_path(tunnel: &str) -> String {
    format!("{DIR}/{tunnel}.token")
}
fn unit_path(tunnel: &str) -> String {
    format!("/etc/systemd/system/{}", unit(tunnel))
}
fn command(runner: &dyn SystemRunner, binary: &str, args: &[&str]) -> Result<(), AgentError> {
    let output = runner.run_command(binary, args)?;
    if !output.status.success() {
        return Err(invalid(&format!(
            "{binary} gagal. Periksa layanan dan coba kembali."
        )));
    }
    Ok(())
}
fn service_state(runner: &dyn SystemRunner, tunnel: &str) -> String {
    runner
        .run_command(
            "systemctl",
            &["show", "--property=ActiveState", "--value", &unit(tunnel)],
        )
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

pub fn install_connector(runner: &dyn SystemRunner) -> Result<Value, AgentError> {
    if runner.operating_system()?.is_rpm() {
        runner.write_file(Path::new("/etc/yum.repos.d/qonpanel-cloudflared.repo"), b"[qonpanel-cloudflared]\nname=Cloudflare cloudflared\nbaseurl=https://pkg.cloudflare.com/cloudflared/rpm\nenabled=1\ngpgcheck=1\ngpgkey=https://pkg.cloudflare.com/cloudflare-ascii-pubkey.gpg\nsslverify=1\n", 0o644)?;
        crate::platform::install(runner, &["cloudflared"])?;
        if !runner.path_exists(Path::new("/usr/bin/cloudflared")) {
            return Err(invalid(
                "Binary cloudflared belum ditemukan setelah instalasi.",
            ));
        }
        return Ok(json!({"installed":true}));
    }
    // Official signed repository; fixed URLs, no shell, no trusted=yes.
    let key = runner.run_command(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--max-filesize",
            "65536",
            "https://pkg.cloudflare.com/cloudflare-main.gpg",
        ],
    )?;
    if !key.status.success() || key.stdout.len() < 100 || key.stdout.len() > 65536 {
        return Err(invalid("Kunci repositori Cloudflare gagal diunduh."));
    }
    runner.write_file(
        Path::new("/usr/share/keyrings/qpanel-cloudflare.gpg"),
        &key.stdout,
        0o644,
    )?;
    runner.write_file(Path::new("/etc/apt/sources.list.d/qpanel-cloudflared.list"), b"deb [signed-by=/usr/share/keyrings/qpanel-cloudflare.gpg] https://pkg.cloudflare.com/cloudflared any main\n", 0o644)?;
    command(runner, "apt-get", &["update"])?;
    command(
        runner,
        "apt-get",
        &["install", "-y", "--no-install-recommends", "cloudflared"],
    )?;
    if !runner.path_exists(Path::new("/usr/bin/cloudflared")) {
        return Err(invalid(
            "Binary cloudflared belum ditemukan setelah instalasi.",
        ));
    }
    Ok(json!({"installed": true}))
}

pub fn execute(runner: &dyn SystemRunner, request: CloudflareRequest) -> Result<Value, AgentError> {
    directory(runner)?;
    let mut local = manifest(runner)?;
    match request {
        CloudflareRequest::Status => {
            let configured = runner.path_exists(Path::new(CREDENTIALS));
            let account = if configured {
                Some(credentials(runner)?.account_id)
            } else {
                None
            };
            return Ok(
                json!({"configured": configured, "account_id": account, "connector_installed": runner.path_exists(Path::new("/usr/bin/cloudflared")), "managed_tunnels": local.tunnels.iter().map(|(key,t)| json!({"id":key,"local_status":service_state(runner,key),"routes":t.routes.iter().map(|(name,r)|json!({"hostname":name,"zone_id":r.zone_id})).collect::<Vec<_>>()})).collect::<Vec<_>>() }),
            );
        }
        CloudflareRequest::Configure {
            account_id,
            api_token,
        } => {
            id(&account_id)?;
            if api_token.0.len() < 20
                || api_token.0.len() > 256
                || !api_token
                    .0
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(invalid("Format API token Cloudflare tidak valid."));
            }
            if !local.tunnels.is_empty() && local.account_id != account_id {
                return Err(conflict(
                    "Hapus tunnel yang dikelola sebelum mengganti akun Cloudflare.",
                ));
            }
            let cred = Credentials {
                account_id,
                api_token: api_token.0,
            };
            let account = api(
                runner,
                &cred,
                "GET",
                &format!("accounts/{}", cred.account_id),
                None,
            )?;
            if account["result"]["id"] != cred.account_id {
                return Err(conflict(
                    "Account ID tidak cocok dengan akun yang diverifikasi.",
                ));
            }
            api(
                runner,
                &cred,
                "GET",
                &format!("zones?account.id={}&per_page=1", cred.account_id),
                None,
            )?;
            let key = crypto::get_or_create_master_key(runner, None)?;
            let plain =
                serde_json::to_string(&cred).map_err(|_| invalid("Kredensial tidak valid."))?;
            write_atomic(
                runner,
                CREDENTIALS,
                crypto::encrypt_value(&key, &plain)?.as_bytes(),
            )?;
            return Ok(json!({"configured":true,"account_id":cred.account_id}));
        }
        _ => {}
    }
    let cred = credentials(runner)?;
    local.account_id.clone_from(&cred.account_id);
    let base = format!("accounts/{}/cfd_tunnel", cred.account_id);
    match request {
        CloudflareRequest::Zones { page: number } => Ok(list(
            api(
                runner,
                &cred,
                "GET",
                &format!(
                    "zones?account.id={}&page={}&per_page=50",
                    cred.account_id,
                    page(number)?
                ),
                None,
            )?,
            &["id", "name", "status", "name_servers"],
        )),
        CloudflareRequest::CreateZone { name } => {
            let name = domain(&name)?;
            let value = api(
                runner,
                &cred,
                "POST",
                "zones",
                Some(json!({"name":name,"account":{"id":cred.account_id},"type":"full"})),
            )?;
            Ok(filtered(
                &value["result"],
                &["id", "name", "status", "name_servers"],
            ))
        }
        CloudflareRequest::Records {
            zone_id,
            page: number,
        } => {
            zone(runner, &cred, &zone_id)?;
            Ok(list(
                api(
                    runner,
                    &cred,
                    "GET",
                    &format!(
                        "zones/{zone_id}/dns_records?page={}&per_page=100",
                        page(number)?
                    ),
                    None,
                )?,
                &[
                    "id", "type", "name", "content", "ttl", "proxied", "priority",
                ],
            ))
        }
        CloudflareRequest::SaveRecord {
            zone_id,
            record_id,
            record,
        } => {
            let zone_name = zone(runner, &cred, &zone_id)?;
            let body = record_body(&record, &zone_name)?;
            let path = if let Some(record_id) = &record_id {
                id(record_id)?;
                if owned_record(&local, record_id) {
                    return Err(conflict(
                        "Record dipakai tunnel. Lepaskan hostname melalui pengaturan tunnel.",
                    ));
                }
                format!("zones/{zone_id}/dns_records/{record_id}")
            } else {
                format!("zones/{zone_id}/dns_records")
            };
            let value = api(
                runner,
                &cred,
                if record_id.is_some() { "PUT" } else { "POST" },
                &path,
                Some(body),
            )?;
            Ok(filtered(
                &value["result"],
                &[
                    "id", "type", "name", "content", "ttl", "proxied", "priority",
                ],
            ))
        }
        CloudflareRequest::DeleteRecord { zone_id, record_id } => {
            zone(runner, &cred, &zone_id)?;
            id(&record_id)?;
            if owned_record(&local, &record_id) {
                return Err(conflict(
                    "Record dipakai tunnel. Lepaskan hostname melalui pengaturan tunnel.",
                ));
            }
            api(
                runner,
                &cred,
                "DELETE",
                &format!("zones/{zone_id}/dns_records/{record_id}"),
                None,
            )?;
            Ok(json!({"deleted":true}))
        }
        CloudflareRequest::Tunnels { page: number } => {
            let mut result = list(
                api(
                    runner,
                    &cred,
                    "GET",
                    &format!("{base}?is_deleted=false&page={}&per_page=50", page(number)?),
                    None,
                )?,
                &["id", "name", "status", "remote_config", "created_at"],
            );
            if let Some(items) = result["items"].as_array_mut() {
                for item in items {
                    let key = item["id"].as_str().unwrap_or_default().to_owned();
                    tunnel_id(&key)?;
                    item["managed"] = json!(local.tunnels.contains_key(&key));
                }
            }
            Ok(result)
        }
        CloudflareRequest::CreateTunnel { name } => {
            if !(1..=80).contains(&name.len())
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid(
                    "Nama tunnel harus 1–80 karakter huruf, angka, - atau _.",
                ));
            }
            let value = api(
                runner,
                &cred,
                "POST",
                &base,
                Some(json!({"name":name,"config_src":"cloudflare"})),
            )?;
            let key = value["result"]["id"]
                .as_str()
                .ok_or_else(|| invalid("ID tunnel tidak ditemukan."))?;
            tunnel_id(key)?;
            local.tunnels.insert(key.into(), ManagedTunnel::default());
            save_manifest(runner, &local)?;
            Ok(filtered(&value["result"], &["id", "name", "status"]))
        }
        CloudflareRequest::TunnelAction {
            tunnel_id: key,
            operation,
        } => {
            tunnel_id(&key)?;
            if matches!(operation, TunnelAction::Import) {
                let details = api(runner, &cred, "GET", &format!("{base}/{key}"), None)?;
                if details["result"]["remote_config"] != true {
                    return Err(invalid(
                        "Hanya tunnel remotely managed yang bisa dihubungkan.",
                    ));
                }
                local.tunnels.entry(key.clone()).or_default();
                save_manifest(runner, &local)?;
            } else if !local.tunnels.contains_key(&key) {
                return Err(conflict("Hubungkan tunnel ke panel terlebih dahulu."));
            }
            match operation {
                TunnelAction::Start => {
                    if !runner.path_exists(Path::new("/usr/bin/cloudflared")) {
                        return Err(invalid("Pasang konektor cloudflared terlebih dahulu."));
                    }
                    let remote = api(
                        runner,
                        &cred,
                        "GET",
                        &format!("{base}/{key}/configurations"),
                        None,
                    )?;
                    let config = remote["result"].get("config").ok_or_else(|| {
                        invalid(
                            "Respons konfigurasi tunnel tidak lengkap; konektor tidak dijalankan.",
                        )
                    })?;
                    if let Some(rules) = config["ingress"].as_array() {
                        if !rules.is_empty() {
                            validate_ingress_boundary(rules)?;
                        }
                    } else if !config.is_null()
                        && config.as_object().is_none_or(|object| !object.is_empty())
                    {
                        return Err(invalid(
                            "Konfigurasi tunnel tidak dapat diverifikasi; konektor tidak dijalankan.",
                        ));
                    }
                    let response = api(runner, &cred, "GET", &format!("{base}/{key}/token"), None)?;
                    let token = response["result"]
                        .as_str()
                        .ok_or_else(|| invalid("Token tunnel tidak ditemukan."))?;
                    if token.len() > 16384
                        || !token
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-=+/".contains(&b))
                    {
                        return Err(invalid("Token tunnel tidak valid."));
                    }
                    runner.write_file(Path::new(&token_path(&key)), token.as_bytes(), 0o600)?;
                    let contents = format!(
                        "[Unit]\nDescription=Qinpanel Cloudflare Tunnel {key}\nAfter=network-online.target nginx.service\nWants=network-online.target\n\n[Service]\nType=simple\nDynamicUser=yes\nUser=qpanel-cloudflared\nLoadCredential=token:{}\nExecStart=/usr/bin/cloudflared --no-autoupdate tunnel run --token-file %d/token\nRestart=on-failure\nRestartSec=5s\nNoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nPrivateDevices=yes\nProtectKernelTunables=yes\nProtectControlGroups=yes\nCapabilityBoundingSet=\nRestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n\n[Install]\nWantedBy=multi-user.target\n",
                        token_path(&key)
                    );
                    runner.write_file(Path::new(&unit_path(&key)), contents.as_bytes(), 0o644)?;
                    command(runner, "systemctl", &["daemon-reload"])?;
                    command(runner, "systemctl", &["enable", "--now", &unit(&key)])?;
                }
                TunnelAction::Stop => {
                    command(runner, "systemctl", &["disable", "--now", &unit(&key)])?;
                }
                TunnelAction::Remove => {
                    if local
                        .tunnels
                        .get(&key)
                        .is_some_and(|t| !t.routes.is_empty())
                    {
                        return Err(conflict(
                            "Lepaskan seluruh hostname sebelum menghapus tunnel.",
                        ));
                    }
                    if runner.path_exists(Path::new(&unit_path(&key))) {
                        command(runner, "systemctl", &["disable", "--now", &unit(&key)])?;
                    }
                    api(runner, &cred, "DELETE", &format!("{base}/{key}"), None)?;
                    for path in [unit_path(&key), token_path(&key)] {
                        if runner.path_exists(Path::new(&path)) {
                            runner.remove_file(Path::new(&path))?;
                        }
                    }
                    command(runner, "systemctl", &["daemon-reload"])?;
                    local.tunnels.remove(&key);
                    save_manifest(runner, &local)?;
                }
                TunnelAction::Import => {}
            }
            Ok(json!({"id":key,"local_status":service_state(runner,&key)}))
        }
        CloudflareRequest::Publish {
            zone_id,
            tunnel_id,
            hostname,
            tls,
        } => publish(
            runner, &cred, &mut local, &base, &zone_id, &tunnel_id, &hostname, tls,
        ),
        CloudflareRequest::Unpublish {
            zone_id,
            tunnel_id,
            hostname,
        } => unpublish(
            runner, &cred, &mut local, &base, &zone_id, &tunnel_id, &hostname,
        ),
        CloudflareRequest::Status | CloudflareRequest::Configure { .. } => {
            Err(invalid("Operasi tidak valid."))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn publish(
    runner: &dyn SystemRunner,
    cred: &Credentials,
    local: &mut Manifest,
    base: &str,
    zone_id: &str,
    key: &str,
    hostname: &str,
    tls: bool,
) -> Result<Value, AgentError> {
    tunnel_id(key)?;
    let zone_name = zone(runner, cred, zone_id)?;
    let hostname = name_in_zone(hostname, &zone_name)?;
    if !local.tunnels.contains_key(key) {
        return Err(conflict("Hubungkan tunnel ke panel terlebih dahulu."));
    }
    if local
        .tunnels
        .values()
        .any(|t| t.routes.contains_key(&hostname))
    {
        return Err(conflict(
            "Hostname sudah diterbitkan. Lepaskan dahulu untuk mengganti rute.",
        ));
    }
    let target = format!("{key}.cfargotunnel.com");
    let existing = api(
        runner,
        cred,
        "GET",
        &format!("zones/{zone_id}/dns_records?name={hostname}&per_page=100"),
        None,
    )?;
    if !existing["result"]
        .as_array()
        .ok_or_else(|| invalid("Daftar DNS tidak valid; publikasi dibatalkan."))?
        .is_empty()
    {
        return Err(conflict(
            "Hostname sudah memiliki record DNS. Hapus atau ubah record tersebut secara eksplisit sebelum menerbitkan tunnel.",
        ));
    }
    let path = format!("{base}/{key}/configurations");
    let previous = api(runner, cred, "GET", &path, None)?;
    let old = previous["result"]["config"].clone();
    let mut config = if old.is_object() {
        old.clone()
    } else {
        json!({})
    };
    let mut ingress = config["ingress"].as_array().cloned().unwrap_or_default();
    if ingress.iter().any(|r| r["hostname"] == hostname) {
        return Err(conflict(
            "Hostname sudah dipakai konfigurasi tunnel di luar panel.",
        ));
    }
    if ingress.is_empty() {
        ingress.push(json!({"service":"http_status:404"}));
    }
    validate_ingress_boundary(&ingress)?;
    ingress.insert(0, ingress_rule(&hostname, tls));
    config["ingress"] = json!(ingress);
    api(runner, cred, "PUT", &path, Some(json!({"config":config})))?;
    let record = match api(
        runner,
        cred,
        "POST",
        &format!("zones/{zone_id}/dns_records"),
        Some(json!({"type":"CNAME","name":hostname,"content":target,"proxied":true,"ttl":1})),
    ) {
        Ok(value) => value,
        Err(error) => {
            if api(runner, cred, "PUT", &path, Some(json!({"config":old}))).is_err() {
                return Err(invalid(
                    "DNS gagal dibuat dan konfigurasi tunnel gagal dipulihkan. Periksa rute tunnel sebelum mencoba kembali.",
                ));
            }
            return Err(error);
        }
    };
    let record_id = record["result"]["id"].as_str().ok_or_else(|| invalid("DNS sudah dibuat, tetapi ID record tidak ditemukan. Periksa DNS sebelum mencoba kembali."))?.to_owned();
    id(&record_id)?;
    if let Some(tunnel) = local.tunnels.get_mut(key) {
        tunnel.routes.insert(
            hostname.clone(),
            Route {
                zone_id: zone_id.into(),
                record_id: record_id.clone(),
                tls,
            },
        );
    }
    if save_manifest(runner, local).is_err() {
        let dns = api(
            runner,
            cred,
            "DELETE",
            &format!("zones/{zone_id}/dns_records/{record_id}"),
            None,
        );
        let cfg = api(runner, cred, "PUT", &path, Some(json!({"config":old})));
        return Err(invalid(if dns.is_ok() && cfg.is_ok() {
            "Inventaris gagal disimpan; perubahan DNS dan tunnel telah dibatalkan."
        } else {
            "Inventaris gagal disimpan dan rollback tidak lengkap. Periksa DNS serta rute tunnel sebelum mencoba kembali."
        }));
    }
    Ok(json!({"hostname":hostname,"published":true}))
}
fn unpublish(
    runner: &dyn SystemRunner,
    cred: &Credentials,
    local: &mut Manifest,
    base: &str,
    zone_id: &str,
    key: &str,
    hostname: &str,
) -> Result<Value, AgentError> {
    tunnel_id(key)?;
    id(zone_id)?;
    let hostname = domain(hostname)?;
    let route = local
        .tunnels
        .get(key)
        .and_then(|t| t.routes.get(&hostname))
        .ok_or_else(|| conflict("Rute tidak dikelola panel."))?;
    if route.zone_id != zone_id {
        return Err(conflict("Zona tidak sesuai dengan rute."));
    }
    let path = format!("{base}/{key}/configurations");
    let previous = api(runner, cred, "GET", &path, None)?;
    let mut config = previous["result"]["config"].clone();
    let ingress = config["ingress"]
        .as_array_mut()
        .ok_or_else(|| invalid("Konfigurasi ingress tidak valid."))?;
    let expected = ingress_rule(&hostname, route.tls);
    if ingress
        .iter()
        .any(|r| r["hostname"] == hostname && r != &expected)
    {
        return Err(conflict(
            "Rute hostname berubah di luar panel. Periksa ingress sebelum melepas DNS.",
        ));
    }
    ingress.retain(|r| r != &expected);
    // Delete only the exact record still owned by this route, preserving operator changes.
    let record = api(
        runner,
        cred,
        "GET",
        &format!("zones/{zone_id}/dns_records?name={hostname}&per_page=100"),
        None,
    )?;
    let records = record["result"]
        .as_array()
        .ok_or_else(|| invalid("Daftar record tidak valid."))?;
    if records.iter().any(|r| {
        r["id"] != route.record_id
            || r["type"] != "CNAME"
            || r["name"] != hostname
            || r["content"] != format!("{key}.cfargotunnel.com")
    }) {
        return Err(conflict(
            "Record DNS berubah di luar panel. Periksa konflik sebelum melepas rute.",
        ));
    }
    api(runner, cred, "PUT", &path, Some(json!({"config":config})))?;
    if !records.is_empty()
        && let Err(error) = api(
            runner,
            cred,
            "DELETE",
            &format!("zones/{zone_id}/dns_records/{}", route.record_id),
            None,
        )
    {
        if api(
            runner,
            cred,
            "PUT",
            &path,
            Some(json!({"config":previous["result"]["config"]})),
        )
        .is_err()
        {
            return Err(invalid(
                "DNS gagal dihapus dan rute gagal dipulihkan. Periksa status tunnel.",
            ));
        }
        return Err(error);
    }
    if let Some(tunnel) = local.tunnels.get_mut(key) {
        tunnel.routes.remove(&hostname);
    }
    save_manifest(runner, local)?;
    Ok(json!({"hostname":hostname,"published":false}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};
    use std::{collections::VecDeque, path::PathBuf, sync::Mutex};
    const ACCOUNT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ZONE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const RECORD: &str = "cccccccccccccccccccccccccccccccc";
    const TUNNEL: &str = "01234567-89ab-4cde-8fab-0123456789ab";
    const TOKEN: &str = "fixture_cloudflare_token_123456789";
    struct Reply {
        method: String,
        path: String,
        body: Value,
        code: u16,
    }
    #[derive(Default)]
    struct Fixture {
        inner: MockSystemRunner,
        replies: Mutex<VecDeque<Reply>>,
        requests: Mutex<Vec<(String, String, Value)>>,
    }
    impl Fixture {
        fn new() -> Self {
            let this = Self::default();
            this.inner
                .set_file_content(crypto::DEFAULT_MASTER_KEY_PATH, &[7; 32]);
            let cred = Credentials {
                account_id: ACCOUNT.into(),
                api_token: TOKEN.into(),
            };
            this.inner.set_file_content(
                CREDENTIALS,
                crypto::encrypt_value(&[7; 32], &serde_json::to_string(&cred).unwrap())
                    .unwrap()
                    .as_bytes(),
            );
            let mut local = Manifest {
                account_id: ACCOUNT.into(),
                ..Default::default()
            };
            local
                .tunnels
                .insert(TUNNEL.into(), ManagedTunnel::default());
            this.inner
                .set_file_content(MANIFEST, &serde_json::to_vec(&local).unwrap());
            this
        }
        fn reply(&self, method: &str, path: &str, result: Value) {
            self.replies.lock().unwrap().push_back(Reply {method:method.into(),path:path.into(),body:json!({"success":true,"result":result,"result_info":{"page":1,"total_pages":1}}),code:200});
        }
        fn reject(&self, method: &str, path: &str) {
            self.replies.lock().unwrap().push_back(Reply {
                method: method.into(),
                path: path.into(),
                body: json!({"success":false,"errors":[{"code":9109,"message":TOKEN}]}),
                code: 403,
            });
        }
        fn zone(&self) {
            self.reply(
                "GET",
                &format!("zones/{ZONE}"),
                json!({"name":"example.com","account":{"id":ACCOUNT}}),
            );
        }
        fn pending(&self) {
            assert!(self.replies.lock().unwrap().is_empty());
        }
        fn cf(&self, command: CloudflareRequest) -> Result<Value, AgentError> {
            execute(self, command)
        }
    }
    impl SystemRunner for Fixture {
        fn operating_system(&self) -> Result<panel_core::platform::LinuxPlatform, AgentError> {
            self.inner.operating_system()
        }
        fn run_command(
            &self,
            binary: &str,
            args: &[&str],
        ) -> Result<std::process::Output, AgentError> {
            let mut output = self.inner.run_command(binary, args)?;
            if binary == "curl" {
                assert!(!args.join(" ").contains(TOKEN));
                let method = args[args.iter().position(|a| *a == "--request").unwrap() + 1];
                let url = args
                    .iter()
                    .find(|s| s.starts_with("https://api.cloudflare.com/"))
                    .unwrap();
                let path = url
                    .strip_prefix("https://api.cloudflare.com/client/v4/")
                    .unwrap();
                let next = self
                    .replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected Cloudflare call");
                assert_eq!((method, path), (next.method.as_str(), next.path.as_str()));
                let config = args[args.iter().position(|a| *a == "--config").unwrap() + 1];
                assert!(
                    String::from_utf8(self.inner.read_file(Path::new(config)).unwrap())
                        .unwrap()
                        .contains(TOKEN)
                );
                let body = args
                    .iter()
                    .position(|a| *a == "--data-binary")
                    .map(|index| {
                        serde_json::from_slice(
                            &self
                                .inner
                                .read_file(Path::new(&args[index + 1][1..]))
                                .unwrap(),
                        )
                        .unwrap()
                    })
                    .unwrap_or(Value::Null);
                self.requests
                    .lock()
                    .unwrap()
                    .push((method.into(), path.into(), body));
                output.stdout = format!("{}\n{}", next.body, next.code).into_bytes();
            }
            Ok(output)
        }
        fn create_dir_all(&self, p: &Path, m: u32, u: u32, g: u32) -> Result<(), AgentError> {
            self.inner.create_dir_all(p, m, u, g)
        }
        fn set_permissions(&self, p: &Path, m: u32) -> Result<(), AgentError> {
            self.inner.set_permissions(p, m)
        }
        fn set_ownership(&self, p: &Path, u: u32, g: u32) -> Result<(), AgentError> {
            self.inner.set_ownership(p, u, g)
        }
        fn set_facl(&self, p: &Path, u: &str, m: &str) -> Result<(), AgentError> {
            self.inner.set_facl(p, u, m)
        }
        fn move_path(&self, a: &Path, b: &Path) -> Result<(), AgentError> {
            self.inner.move_path(a, b)
        }
        fn create_symlink(&self, a: &Path, b: &Path) -> Result<(), AgentError> {
            self.inner.create_symlink(a, b)
        }
        fn read_link(&self, p: &Path) -> Result<PathBuf, AgentError> {
            self.inner.read_link(p)
        }
        fn remove_dir_all(&self, p: &Path) -> Result<(), AgentError> {
            self.inner.remove_dir_all(p)
        }
        fn path_exists(&self, p: &Path) -> bool {
            self.inner.path_exists(p)
        }
        fn write_file(&self, p: &Path, b: &[u8], m: u32) -> Result<(), AgentError> {
            self.inner.write_file(p, b, m)
        }
        fn read_file(&self, p: &Path) -> Result<Vec<u8>, AgentError> {
            self.inner.read_file(p)
        }
        fn remove_file(&self, p: &Path) -> Result<(), AgentError> {
            self.inner.remove_file(p)
        }
        fn read_dir(&self, p: &Path) -> Result<Vec<PathBuf>, AgentError> {
            self.inner.read_dir(p)
        }
    }
    #[test]
    fn credentials_are_encrypted_redacted_and_exchange_files_are_removed() {
        let fixture = Fixture::new();
        fixture.reply("GET", &format!("accounts/{ACCOUNT}"), json!({"id":ACCOUNT}));
        fixture.reply(
            "GET",
            &format!("zones?account.id={ACCOUNT}&per_page=1"),
            json!([]),
        );
        let command = CloudflareRequest::Configure {
            account_id: ACCOUNT.into(),
            api_token: panel_proto::CloudflareSecret(TOKEN.into()),
        };
        assert!(!format!("{command:?}").contains(TOKEN));
        let result = fixture.cf(command).unwrap();
        assert!(!result.to_string().contains(TOKEN));
        let encrypted =
            String::from_utf8(fixture.inner.read_file(Path::new(CREDENTIALS)).unwrap()).unwrap();
        assert!(encrypted.starts_with(crypto::CIPHER_PREFIX));
        assert!(!encrypted.contains(TOKEN));
        assert_eq!(credentials(&fixture).unwrap().api_token, TOKEN);
        assert!(
            !fixture
                .inner
                .read_dir(Path::new(DIR))
                .unwrap()
                .iter()
                .any(|p| p.to_string_lossy().contains("exchange-"))
        );
        fixture.pending();
    }
    #[test]
    fn invalid_ids_and_cross_account_zones_are_denied() {
        let fixture = Fixture::new();
        assert!(
            fixture
                .cf(CloudflareRequest::Records {
                    zone_id: "../../etc".into(),
                    page: 1
                })
                .is_err()
        );
        fixture.reply(
            "GET",
            &format!("zones/{ZONE}"),
            json!({"name":"example.com","account":{"id":"other-account"}}),
        );
        assert!(
            fixture
                .cf(CloudflareRequest::Records {
                    zone_id: ZONE.into(),
                    page: 1
                })
                .is_err()
        );
        assert!(tunnel_id("../../etc/passwd").is_err());
        assert!(tunnel_id(&format!("{{{TUNNEL}}}")).is_err());
        fixture.pending();
    }
    #[test]
    fn connector_install_uses_only_official_signed_apt_repository() {
        let runner = MockSystemRunner::new();
        runner.set_response("curl", 0, &[42; 256], b"");
        runner.add_existing_path("/usr/bin/cloudflared");
        assert_eq!(install_connector(&runner).unwrap()["installed"], true);
        let source = String::from_utf8(
            runner
                .read_file(Path::new("/etc/apt/sources.list.d/qpanel-cloudflared.list"))
                .unwrap(),
        )
        .unwrap();
        assert!(source.contains("signed-by=/usr/share/keyrings/qpanel-cloudflare.gpg"));
        assert!(!source.contains("trusted=yes"));
        let commands = runner.get_commands().join("\n");
        assert!(commands.contains("https://pkg.cloudflare.com/cloudflare-main.gpg"));
        assert!(commands.contains("apt-get install -y --no-install-recommends cloudflared"));
        assert!(!commands.contains("bash"));
    }
    #[test]
    fn records_validate_type_address_scope_and_support_dkim_names() {
        let mut record = CloudflareDnsRecord {
            kind: "TXT".into(),
            name: "_dmarc.example.com".into(),
            content: "v=DMARC1; p=none".into(),
            ttl: 300,
            proxied: false,
            priority: None,
        };
        assert_eq!(
            record_body(&record, "example.com").unwrap()["name"],
            "_dmarc.example.com"
        );
        record.name = "otherexample.com".into();
        assert!(record_body(&record, "example.com").is_err());
        record.name = "@".into();
        record.kind = "A".into();
        assert!(record_body(&record, "example.com").is_err());
        record.content = "203.0.113.10".into();
        record.proxied = true;
        assert_eq!(record_body(&record, "example.com").unwrap()["ttl"], 1);
        record.kind = "MX".into();
        assert!(record_body(&record, "example.com").is_err());
        record.proxied = false;
        record.content = "mail.example.com".into();
        assert!(record_body(&record, "example.com").is_err());
        record.priority = Some(0);
        assert!(record_body(&record, "example.com").is_ok());
        record.kind = "TXT".into();
        record.name = "*.example.com".into();
        assert!(record_body(&record, "example.com").is_ok());
    }
    #[test]
    fn provider_errors_do_not_echo_tokens_and_zone_lists_are_paginated_and_filtered() {
        let fixture = Fixture::new();
        fixture.reject(
            "GET",
            &format!("zones?account.id={ACCOUNT}&page=1&per_page=50"),
        );
        let error = fixture
            .cf(CloudflareRequest::Zones { page: 1 })
            .unwrap_err()
            .to_string();
        assert!(error.contains("9109"));
        assert!(!error.contains(TOKEN));
        fixture.reply("GET",&format!("zones?account.id={ACCOUNT}&page=2&per_page=50"),json!([{"id":ZONE,"name":"example.com","token":TOKEN,"name_servers":["one.ns.cloudflare.com"]}]));
        let value = fixture.cf(CloudflareRequest::Zones { page: 2 }).unwrap();
        assert!(!value.to_string().contains(TOKEN));
        assert_eq!(value["items"][0]["id"], ZONE);
        fixture.pending();
    }
    fn old_config() -> Value {
        json!({"originRequest":{"connectTimeout":20},"ingress":[{"hostname":"existing.example.net","service":"http://localhost:5000"},{"service":"http_status:404"}]})
    }
    fn publish_request() -> CloudflareRequest {
        CloudflareRequest::Publish {
            zone_id: ZONE.into(),
            tunnel_id: TUNNEL.into(),
            hostname: "app.example.com".into(),
            tls: true,
        }
    }
    fn prepare_publish(fixture: &Fixture) {
        fixture.zone();
        fixture.reply(
            "GET",
            &format!("zones/{ZONE}/dns_records?name=app.example.com&per_page=100"),
            json!([]),
        );
        fixture.reply(
            "GET",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({"config":old_config()}),
        );
        fixture.reply(
            "PUT",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({}),
        );
    }

    #[test]
    fn publication_rejects_routes_exposing_unregistered_hosts_without_remote_writes() {
        for ingress in [
            json!([{"service":"http://127.0.0.1:80"}]),
            json!([{"hostname":"*.example.com","service":"http://127.0.0.1:80","originRequest":{"httpHostHeader":"known.example.com"}},{"service":"http_status:404"}]),
            json!([{"path":"/.*","service":"http://localhost:80"},{"service":"http_status:404"}]),
            json!([{"service":"hello_world"}]),
            json!([{"service":"http_status:200"}]),
        ] {
            let fixture = Fixture::new();
            fixture.zone();
            fixture.reply(
                "GET",
                &format!("zones/{ZONE}/dns_records?name=app.example.com&per_page=100"),
                json!([]),
            );
            fixture.reply(
                "GET",
                &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
                json!({"config":{"ingress":ingress}}),
            );
            assert!(fixture.cf(publish_request()).is_err());
            assert!(
                fixture
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|request| request.0 == "GET")
            );
            assert!(
                manifest(&fixture).unwrap().tunnels[TUNNEL]
                    .routes
                    .is_empty()
            );
            fixture.pending();
        }
    }

    #[test]
    fn ingress_boundary_accepts_explicit_hosts_and_denial_only_catchalls() {
        for status in [400, 403, 404, 410, 503] {
            validate_ingress_boundary(&[
                json!({"hostname":"app.example.com","service":"http://127.0.0.1:80"}),
                json!({"service":format!("http_status:{status}")}),
            ])
            .unwrap();
        }
        assert!(validate_ingress_boundary(&[]).is_err());
        assert!(
            validate_ingress_boundary(&[json!({"hostname":"*","service":"http_status:404"})])
                .is_err()
        );
    }
    #[test]
    fn connector_refuses_unsafe_ingress_before_fetching_secrets_or_starting_service() {
        for configuration in [
            json!({"config":{"ingress":[{"service":"http://127.0.0.1:80"}]}}),
            json!({}),
            json!({"config":{"ingress":"invalid"}}),
            json!({"config":{"originRequest":{"httpHostHeader":"known.example.com"}}}),
        ] {
            let fixture = Fixture::new();
            fixture.inner.add_existing_path("/usr/bin/cloudflared");
            fixture.reply(
                "GET",
                &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
                configuration,
            );
            assert!(
                fixture
                    .cf(CloudflareRequest::TunnelAction {
                        tunnel_id: TUNNEL.into(),
                        operation: TunnelAction::Start
                    })
                    .is_err()
            );
            assert!(!fixture.inner.path_exists(Path::new(&token_path(TUNNEL))));
            assert!(!fixture.inner.path_exists(Path::new(&unit_path(TUNNEL))));
            assert!(
                !fixture
                    .inner
                    .get_commands()
                    .iter()
                    .any(|command| command.starts_with("systemctl"))
            );
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            fixture.pending();
        }
    }

    #[test]
    fn publication_preserves_existing_ingress_and_uses_only_local_nginx_with_verified_tls() {
        let fixture = Fixture::new();
        prepare_publish(&fixture);
        fixture.reply(
            "POST",
            &format!("zones/{ZONE}/dns_records"),
            json!({"id":RECORD}),
        );
        assert_eq!(fixture.cf(publish_request()).unwrap()["published"], true);
        let requests = fixture.requests.lock().unwrap();
        let config = &requests[3].2["config"];
        assert_eq!(config["originRequest"], old_config()["originRequest"]);
        assert_eq!(config["ingress"][1], old_config()["ingress"][0]);
        assert_eq!(config["ingress"][2]["service"], "http_status:404");
        assert_eq!(config["ingress"][0]["service"], "https://127.0.0.1:443");
        assert_eq!(config["ingress"][0]["originRequest"]["noTLSVerify"], false);
        assert!(
            manifest(&fixture).unwrap().tunnels[TUNNEL]
                .routes
                .contains_key("app.example.com")
        );
        fixture.pending();
    }
    #[test]
    fn publication_rejects_existing_dns_without_writing_and_rolls_back_on_dns_failure() {
        let fixture = Fixture::new();
        fixture.zone();
        fixture.reply(
            "GET",
            &format!("zones/{ZONE}/dns_records?name=app.example.com&per_page=100"),
            json!([{"type":"A"}]),
        );
        assert!(
            fixture
                .cf(publish_request())
                .unwrap_err()
                .to_string()
                .contains("sudah memiliki")
        );
        assert!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.0 == "GET")
        );
        prepare_publish(&fixture);
        fixture.reject("POST", &format!("zones/{ZONE}/dns_records"));
        fixture.reply(
            "PUT",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({}),
        );
        assert!(fixture.cf(publish_request()).is_err());
        assert_eq!(
            fixture.requests.lock().unwrap().last().unwrap().2["config"],
            old_config()
        );
        assert!(
            manifest(&fixture).unwrap().tunnels[TUNNEL]
                .routes
                .is_empty()
        );
        fixture.pending();
    }
    #[test]
    fn connector_uses_credential_file_and_hardened_unit_without_token_arguments() {
        let fixture = Fixture::new();
        fixture.inner.add_existing_path("/usr/bin/cloudflared");
        fixture.inner.set_response("systemctl", 0, b"active\n", b"");
        fixture.reply(
            "GET",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({"config":old_config()}),
        );
        fixture.reply(
            "GET",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/token"),
            json!("fixtureTunnelToken123="),
        );
        let result = fixture
            .cf(CloudflareRequest::TunnelAction {
                tunnel_id: TUNNEL.into(),
                operation: TunnelAction::Start,
            })
            .unwrap();
        assert_eq!(result["local_status"], "active");
        let unit = String::from_utf8(
            fixture
                .inner
                .read_file(Path::new(&unit_path(TUNNEL)))
                .unwrap(),
        )
        .unwrap();
        assert!(unit.contains("DynamicUser=yes"));
        assert!(unit.contains("--token-file %d/token"));
        assert!(!unit.contains("fixtureTunnelToken"));
        assert!(
            !fixture
                .inner
                .get_commands()
                .join("\n")
                .contains("fixtureTunnelToken")
        );
        assert!(fixture.inner.recorded_actions().iter().any(|a|matches!(a,MockAction::WriteFile{path,mode,..} if path==Path::new(&token_path(TUNNEL)) && *mode==0o600)));
        fixture.pending();
    }
    #[test]
    fn unpublish_recovers_when_dns_was_already_deleted_but_local_save_failed() {
        let fixture = Fixture::new();
        let mut local = manifest(&fixture).unwrap();
        local.tunnels.get_mut(TUNNEL).unwrap().routes.insert(
            "app.example.com".into(),
            Route {
                zone_id: ZONE.into(),
                record_id: RECORD.into(),
                tls: true,
            },
        );
        save_manifest(&fixture, &local).unwrap();
        fixture.reply(
            "GET",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({"config":old_config()}),
        );
        fixture.reply(
            "GET",
            &format!("zones/{ZONE}/dns_records?name=app.example.com&per_page=100"),
            json!([]),
        );
        fixture.reply(
            "PUT",
            &format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),
            json!({}),
        );
        assert_eq!(
            fixture
                .cf(CloudflareRequest::Unpublish {
                    zone_id: ZONE.into(),
                    tunnel_id: TUNNEL.into(),
                    hostname: "app.example.com".into()
                })
                .unwrap()["published"],
            false
        );
        assert!(
            manifest(&fixture).unwrap().tunnels[TUNNEL]
                .routes
                .is_empty()
        );
        fixture.pending();
    }
    #[test]
    fn failed_credential_rotation_preserves_existing_account_secret() {
        let fixture = Fixture::new();
        let before = fixture.inner.read_file(Path::new(CREDENTIALS)).unwrap();
        fixture.reject("GET", &format!("accounts/{ACCOUNT}"));
        assert!(
            fixture
                .cf(CloudflareRequest::Configure {
                    account_id: ACCOUNT.into(),
                    api_token: panel_proto::CloudflareSecret(format!("{TOKEN}_rotated"))
                })
                .is_err()
        );
        assert_eq!(
            before,
            fixture.inner.read_file(Path::new(CREDENTIALS)).unwrap()
        );
        fixture.pending();
    }
    #[test]
    fn externally_modified_routes_and_deleting_a_tunnel_with_routes_are_denied() {
        let fixture = Fixture::new();
        let mut local = manifest(&fixture).unwrap();
        local.tunnels.get_mut(TUNNEL).unwrap().routes.insert(
            "app.example.com".into(),
            Route {
                zone_id: ZONE.into(),
                record_id: RECORD.into(),
                tls: false,
            },
        );
        save_manifest(&fixture, &local).unwrap();
        assert!(
            fixture
                .cf(CloudflareRequest::TunnelAction {
                    tunnel_id: TUNNEL.into(),
                    operation: TunnelAction::Remove
                })
                .is_err()
        );
        fixture.reply("GET",&format!("accounts/{ACCOUNT}/cfd_tunnel/{TUNNEL}/configurations"),json!({"config":{"ingress":[{"hostname":"app.example.com","service":"http://external.example.net"},{"service":"http_status:404"}]}}));
        assert!(
            fixture
                .cf(CloudflareRequest::Unpublish {
                    zone_id: ZONE.into(),
                    tunnel_id: TUNNEL.into(),
                    hostname: "app.example.com".into()
                })
                .unwrap_err()
                .to_string()
                .contains("berubah di luar panel")
        );
        assert!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.0 == "GET")
        );
        assert!(
            manifest(&fixture).unwrap().tunnels[TUNNEL]
                .routes
                .contains_key("app.example.com")
        );
        fixture.pending();
    }
    const DNS_ID: &str = "a416cf3a-e13b-45b0-a905-b3312bd12a99";
    const DNS_REV: &str = "test-revision";
    fn dns_fixture() -> Fixture {
        let f = Fixture::new();
        let c = json!({"zone_id":ZONE,"zone_name":"example.com","token":TOKEN,"revision":DNS_REV});
        f.inner.set_file_content(
            "/etc/panel/dns01/credential.enc",
            crypto::encrypt_value(&[7; 32], &c.to_string())
                .unwrap()
                .as_bytes(),
        );
        f
    }
    fn dns_present() -> panel_proto::Dns01Request {
        panel_proto::Dns01Request::Present {
            id: DNS_ID.into(),
            revision: DNS_REV.into(),
            hostname: "app.example.com".into(),
            digest: "A".repeat(43),
        }
    }
    fn dns_record() -> Value {
        json!({"id":"cccccccccccccccccccccccccccccccc","type":"TXT","name":"_acme-challenge.app.example.com","content":"A".repeat(43),"comment":format!("qpanel-acme:{DNS_ID}")})
    }
    fn dns_records() -> String {
        format!("zones/{ZONE}/dns_records?name.exact=_acme-challenge.app.example.com&per_page=100")
    }
    #[test]
    fn dns01_configuration_is_separate_encrypted_scoped_and_redacted() {
        let f = Fixture::new();
        f.reply(
            "GET",
            &format!("zones/{ZONE}"),
            json!({"name":"example.com","status":"active"}),
        );
        f.reply(
            "GET",
            &format!(
                "zones/{ZONE}/dns_records?name.exact=_acme-challenge.example.com&per_page=100"
            ),
            json!([]),
        );
        let request = panel_proto::Dns01Request::Configure {
            zone_id: ZONE.into(),
            zone_name: "example.com".into(),
            api_token: panel_proto::CloudflareSecret(TOKEN.into()),
        };
        assert!(!format!("{request:?}").contains(TOKEN));
        let result = crate::dns01::execute(&f, request).unwrap();
        assert_eq!(result["configured"], true);
        let encrypted = String::from_utf8(
            f.inner
                .read_file(Path::new("/etc/panel/dns01/credential.enc"))
                .unwrap(),
        )
        .unwrap();
        assert!(encrypted.starts_with(crypto::CIPHER_PREFIX));
        assert!(!encrypted.contains(TOKEN));
        let status = crate::dns01::execute(&f, panel_proto::Dns01Request::Status).unwrap();
        assert!(!status.to_string().contains(TOKEN));
        assert!(f.inner.path_exists(Path::new(CREDENTIALS))); // existing Tunnel credential is untouched
        crate::dns01::execute(&f, panel_proto::Dns01Request::Revoke).unwrap();
        assert!(f.inner.path_exists(Path::new(CREDENTIALS)));
        f.pending();
    }
    #[test]
    fn dns01_lost_post_response_recovers_same_record_and_preserves_other_txt() {
        let f = dns_fixture();
        let path = dns_records();
        f.reply("GET", &path, json!([{"type":"TXT","content":"operator"}]));
        f.reject("POST", &format!("zones/{ZONE}/dns_records"));
        assert!(crate::dns01::execute(&f, dns_present()).is_err());
        let status = crate::dns01::execute(&f, panel_proto::Dns01Request::Status).unwrap();
        assert_eq!(status["pending"].as_array().unwrap().len(), 1);
        assert!(crate::dns01::execute(&f, panel_proto::Dns01Request::Revoke).is_err());
        f.reply("GET",&path,json!([dns_record(),{"id":"dddddddddddddddddddddddddddddddd","type":"TXT","content":"operator"}]));
        assert_eq!(
            crate::dns01::execute(&f, dns_present()).unwrap()["present"],
            true
        );
        f.reply(
            "GET",
            &path,
            json!([dns_record(),{"type":"TXT","content":"operator"}]),
        );
        f.reply(
            "DELETE",
            &format!("zones/{ZONE}/dns_records/cccccccccccccccccccccccccccccccc"),
            Value::Null,
        );
        crate::dns01::execute(&f, panel_proto::Dns01Request::Cleanup { id: DNS_ID.into() })
            .unwrap();
        assert_eq!(
            crate::dns01::execute(&f, panel_proto::Dns01Request::Status).unwrap()["pending"],
            json!([])
        );
        assert_eq!(
            f.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _, _)| m == "POST")
                .count(),
            1
        );
        f.pending();
    }
    #[test]
    fn dns01_cname_conflict_never_replaces_dns_and_invalid_scope_never_calls_provider() {
        let f = dns_fixture();
        for request in [
            panel_proto::Dns01Request::Present {
                id: DNS_ID.into(),
                revision: "old".into(),
                hostname: "app.example.com".into(),
                digest: "A".repeat(43),
            },
            panel_proto::Dns01Request::Present {
                id: DNS_ID.into(),
                revision: DNS_REV.into(),
                hostname: "evil-example.com".into(),
                digest: "A".repeat(43),
            },
            panel_proto::Dns01Request::Present {
                id: DNS_ID.into(),
                revision: DNS_REV.into(),
                hostname: "1.1.1.1".into(),
                digest: "A".repeat(43),
            },
        ] {
            assert!(crate::dns01::execute(&f, request).is_err());
        }
        assert!(f.requests.lock().unwrap().is_empty());
        f.reply(
            "GET",
            &dns_records(),
            json!([{"type":"CNAME","content":"validation.other.test"}]),
        );
        assert!(crate::dns01::execute(&f, dns_present()).is_err());
        assert_eq!(f.requests.lock().unwrap().len(), 1);
        f.pending();
    }
    #[test]
    fn dns01_create_and_cleanup_refuse_changed_operator_record() {
        let f = dns_fixture();
        let path = dns_records();
        f.reply("GET", &path, json!([]));
        f.reply("POST", &format!("zones/{ZONE}/dns_records"), dns_record());
        crate::dns01::execute(&f, dns_present()).unwrap();
        let mut changed = dns_record();
        changed["content"] = json!("operator-edited");
        f.reply("GET", &path, json!([changed]));
        assert!(
            crate::dns01::execute(&f, panel_proto::Dns01Request::Cleanup { id: DNS_ID.into() })
                .is_err()
        );
        assert!(
            !crate::dns01::execute(&f, panel_proto::Dns01Request::Status).unwrap()["pending"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            !f.requests
                .lock()
                .unwrap()
                .iter()
                .any(|(m, _, _)| m == "DELETE")
        );
        f.pending();
    }
    #[test]
    fn dns01_pending_cleanup_can_recover_with_a_fresh_token_for_the_same_zone() {
        let f = dns_fixture();
        f.reply("GET", &dns_records(), json!([]));
        f.reject("POST", &format!("zones/{ZONE}/dns_records"));
        assert!(crate::dns01::execute(&f, dns_present()).is_err());
        assert!(
            crate::dns01::execute(
                &f,
                panel_proto::Dns01Request::Configure {
                    zone_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                    zone_name: "other.test".into(),
                    api_token: panel_proto::CloudflareSecret(TOKEN.into())
                }
            )
            .is_err()
        );
        f.reply(
            "GET",
            &format!("zones/{ZONE}"),
            json!({"name":"example.com","status":"active"}),
        );
        f.reply(
            "GET",
            &format!(
                "zones/{ZONE}/dns_records?name.exact=_acme-challenge.example.com&per_page=100"
            ),
            json!([]),
        );
        let status = crate::dns01::execute(
            &f,
            panel_proto::Dns01Request::Configure {
                zone_id: ZONE.into(),
                zone_name: "example.com".into(),
                api_token: panel_proto::CloudflareSecret(TOKEN.into()),
            },
        )
        .unwrap();
        assert_eq!(status["pending"].as_array().unwrap().len(), 1);
        assert_ne!(status["revision"], DNS_REV);
        f.pending();
    }
}
