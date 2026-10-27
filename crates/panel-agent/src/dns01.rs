//! Dedicated encrypted DNS credential and crash-recoverable TXT ownership journal.
use crate::{
    AgentError,
    cloudflare::{Credentials, api},
    crypto,
    system::SystemRunner,
};
use panel_proto::Dns01Request;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
const DIR: &str = "/etc/panel/dns01";
const CREDENTIAL: &str = "/etc/panel/dns01/credential.enc";
const JOURNAL: &str = "/etc/panel/dns01/challenges.json";
#[derive(Serialize, Deserialize)]
struct Credential {
    zone_id: String,
    zone_name: String,
    token: String,
    revision: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    digest: String,
    record_id: Option<String>,
    created_at: i64,
}
type Journal = BTreeMap<String, Entry>;
fn error(text: &str) -> AgentError {
    AgentError::InvalidInput(text.into())
}
fn uuid(value: &str) -> Result<(), AgentError> {
    if !uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        return Err(error("ID DNS-01 tidak valid."));
    }
    Ok(())
}
fn hex(value: &str) -> Result<(), AgentError> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("ID zona/record tidak valid."));
    }
    Ok(())
}
fn hostname(value: &str) -> Result<String, AgentError> {
    if value.parse::<std::net::IpAddr>().is_ok() || value.contains('*') {
        return Err(error(
            "DNS-01 hanya untuk hostname tepat, bukan IP/wildcard.",
        ));
    }
    panel_core::DomainName::parse(value)
        .map(|d| d.as_ascii().to_owned())
        .map_err(|_| error("Hostname tidak valid."))
}
fn atomic(r: &dyn SystemRunner, path: &str, bytes: &[u8]) -> Result<(), AgentError> {
    r.create_dir_all(Path::new(DIR), 0o700, 0, 0)?;
    let tmp = format!("{path}.new");
    r.write_file(Path::new(&tmp), bytes, 0o600)?;
    r.set_ownership(Path::new(&tmp), 0, 0)?;
    r.move_path(Path::new(&tmp), Path::new(path))
}
fn journal(r: &dyn SystemRunner) -> Result<Journal, AgentError> {
    if !r.path_exists(Path::new(JOURNAL)) {
        return Ok(Journal::new());
    }
    let bytes = r.read_file(Path::new(JOURNAL))?;
    if bytes.len() > 256 * 1024 {
        return Err(error("Jurnal DNS terlalu besar; cleanup diperlukan."));
    }
    serde_json::from_slice(&bytes).map_err(|_| error("Jurnal DNS rusak; tidak menghapus record."))
}
fn save(r: &dyn SystemRunner, j: &Journal) -> Result<(), AgentError> {
    atomic(
        r,
        JOURNAL,
        &serde_json::to_vec(j).map_err(|_| error("Jurnal gagal disimpan."))?,
    )
}
fn credential(r: &dyn SystemRunner) -> Result<Credential, AgentError> {
    let bytes = r.read_file(Path::new(CREDENTIAL))?;
    let encrypted = std::str::from_utf8(&bytes).map_err(|_| error("Kredensial DNS rusak."))?;
    if !encrypted.starts_with(crypto::CIPHER_PREFIX) {
        return Err(error("Kredensial DNS wajib terenkripsi."));
    }
    let key = crypto::get_or_create_master_key(r, None)?;
    serde_json::from_str(&crypto::decrypt_value(&key, encrypted)?)
        .map_err(|_| error("Kredensial DNS rusak."))
}
fn provider(c: &Credential) -> Credentials {
    Credentials {
        account_id: String::new(),
        api_token: c.token.clone(),
    }
}
fn records(r: &dyn SystemRunner, c: &Credential, name: &str) -> Result<Vec<Value>, AgentError> {
    let response = api(
        r,
        &provider(c),
        "GET",
        &format!(
            "zones/{}/dns_records?name.exact={name}&per_page=100",
            c.zone_id
        ),
        None,
    )?;
    let values = response["result"]
        .as_array()
        .ok_or_else(|| error("Daftar DNS tidak valid."))?;
    if values.len() >= 100
        || response["result_info"]["total_pages"]
            .as_u64()
            .is_some_and(|n| n > 1)
    {
        return Err(error(
            "Terlalu banyak record challenge; periksa DNS secara manual.",
        ));
    }
    Ok(values.clone())
}
fn owned(v: &Value, id: &str, entry: &Entry) -> bool {
    v["type"] == "TXT"
        && v["name"] == entry.name
        && v["content"].as_str().is_some_and(|value| {
            value == entry.digest
                || value.strip_prefix('"').and_then(|v| v.strip_suffix('"'))
                    == Some(entry.digest.as_str())
        })
        && v["comment"] == format!("qpanel-acme:{id}")
}
fn cleanup(r: &dyn SystemRunner, id: &str) -> Result<Value, AgentError> {
    uuid(id)?;
    let mut j = journal(r)?;
    let Some(entry) = j.get(id).cloned() else {
        return Ok(json!({"cleaned":true}));
    };
    let c = credential(r)?;
    let values = records(r, &c, &entry.name)?;
    let matches = values
        .iter()
        .filter(|v| owned(v, id, &entry))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(error("Kepemilikan TXT ambigu; record dipertahankan."));
    }
    if let Some(record) = matches.first() {
        let record_id = record["id"]
            .as_str()
            .ok_or_else(|| error("ID TXT hilang."))?;
        hex(record_id)?;
        if entry
            .record_id
            .as_deref()
            .is_some_and(|known| known != record_id)
        {
            return Err(error("ID TXT berubah; cleanup otomatis ditolak."));
        }
        api(
            r,
            &provider(&c),
            "DELETE",
            &format!("zones/{}/dns_records/{record_id}", c.zone_id),
            None,
        )?;
    } else if entry
        .record_id
        .as_ref()
        .is_some_and(|id| values.iter().any(|v| v["id"] == *id))
    {
        return Err(error("Isi TXT berubah; record operator tidak dihapus."));
    }
    j.remove(id);
    save(r, &j)?;
    Ok(json!({"cleaned":true}))
}
pub fn execute(r: &dyn SystemRunner, request: Dns01Request) -> Result<Value, AgentError> {
    match request {
        Dns01Request::Status => {
            let j = journal(r)?;
            let pending = j
                .iter()
                .map(|(id, e)| json!({"id":id,"hostname":e.name,"created_at":e.created_at}))
                .collect::<Vec<_>>();
            if !r.path_exists(Path::new(CREDENTIAL)) {
                return Ok(json!({"configured":false,"pending":pending}));
            }
            let c = credential(r)?;
            Ok(
                json!({"configured":true,"zone_id":c.zone_id,"zone_name":c.zone_name,"revision":c.revision,"pending":pending}),
            )
        }
        Dns01Request::Configure {
            zone_id,
            zone_name,
            api_token,
        } => {
            hex(&zone_id)?;
            let zone_name = hostname(&zone_name)?;
            if !(20..=256).contains(&api_token.0.len())
                || !api_token
                    .0
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(error("Token DNS tidak valid."));
            }
            if !journal(r)?.is_empty() {
                let old = credential(r)?;
                if old.zone_id != zone_id || old.zone_name != zone_name {
                    return Err(error(
                        "Zona tidak boleh diganti sebelum cleanup TXT selesai. Token zona yang sama boleh diperbarui untuk memulihkan akses.",
                    ));
                }
            }
            let c = Credential {
                zone_id,
                zone_name,
                token: api_token.0,
                revision: uuid::Uuid::new_v4().to_string(),
            };
            let zone = api(
                r,
                &provider(&c),
                "GET",
                &format!("zones/{}", c.zone_id),
                None,
            )?;
            if zone["result"]["name"] != c.zone_name || zone["result"]["status"] != "active" {
                return Err(error("Zona DNS harus cocok dan aktif di Cloudflare."));
            }
            records(r, &c, &format!("_acme-challenge.{}", c.zone_name))?;
            let key = crypto::get_or_create_master_key(r, None)?;
            let encoded = serde_json::to_string(&c).map_err(|_| error("Konfigurasi DNS gagal."))?;
            atomic(
                r,
                CREDENTIAL,
                crypto::encrypt_value(&key, &encoded)?.as_bytes(),
            )?;
            execute(r, Dns01Request::Status)
        }
        Dns01Request::Revoke => {
            if !journal(r)?.is_empty() {
                return Err(error(
                    "Cleanup TXT tertunda harus selesai sebelum melepas token.",
                ));
            }
            if r.path_exists(Path::new(CREDENTIAL)) {
                r.remove_file(Path::new(CREDENTIAL))?;
            }
            Ok(json!({"configured":false,"pending":[]}))
        }
        Dns01Request::Present {
            id,
            revision,
            hostname: host,
            digest,
        } => {
            uuid(&id)?;
            let host = hostname(&host)?;
            let c = credential(r)?;
            if revision != c.revision
                || (host != c.zone_name && !host.ends_with(&format!(".{}", c.zone_name)))
            {
                return Err(error("Zona atau revisi token DNS tidak cocok."));
            }
            if digest.len() != 43
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(error("Digest DNS-01 tidak valid."));
            }
            let name = format!("_acme-challenge.{host}");
            let mut j = journal(r)?;
            if j.len() >= 40 && !j.contains_key(&id) {
                return Err(error("Cleanup DNS tertunda diperlukan."));
            }
            if let Some(old) = j.get(&id)
                && (old.name != name || old.digest != digest)
            {
                return Err(error("ID challenge tidak boleh digunakan ulang."));
            }
            let entry = j
                .entry(id.clone())
                .or_insert_with(|| Entry {
                    name: name.clone(),
                    digest: digest.clone(),
                    record_id: None,
                    created_at: chrono::Utc::now().timestamp(),
                })
                .clone();
            // Persist intent BEFORE POST. A lost response is recoverable by the unique comment.
            save(r, &j)?;
            let values = records(r, &c, &name)?;
            if values
                .iter()
                .any(|v| v["type"] == "CNAME" || v["type"] == "NS")
            {
                return Err(error(
                    "CNAME/NS challenge ditemukan. Delegasi otomatis belum didukung; DNS dipertahankan.",
                ));
            }
            let matches = values
                .iter()
                .filter(|v| owned(v, &id, &entry))
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err(error("TXT milik challenge ambigu; tidak ditimpa."));
            }
            let record_id = if let Some(v) = matches.first() {
                v["id"]
                    .as_str()
                    .ok_or_else(|| error("ID TXT hilang."))?
                    .to_owned()
            } else {
                if entry.record_id.is_some() {
                    return Err(error(
                        "TXT managed hilang/berubah; mulai order baru setelah cleanup.",
                    ));
                }
                let result = api(
                    r,
                    &provider(&c),
                    "POST",
                    &format!("zones/{}/dns_records", c.zone_id),
                    Some(
                        json!({"type":"TXT","name":name,"content":digest,"ttl":60,"comment":format!("qpanel-acme:{id}")}),
                    ),
                )?;
                let v = &result["result"];
                if !owned(v, &id, &entry) {
                    return Err(error(
                        "Respons pembuatan TXT tidak cocok; cleanup jurnal diperlukan.",
                    ));
                }
                v["id"]
                    .as_str()
                    .ok_or_else(|| error("ID TXT hilang."))?
                    .to_owned()
            };
            hex(&record_id)?;
            if entry
                .record_id
                .as_deref()
                .is_some_and(|old| old != record_id)
            {
                return Err(error("ID TXT berubah."));
            }
            if let Some(e) = j.get_mut(&id) {
                e.record_id = Some(record_id);
            }
            save(r, &j)?;
            Ok(json!({"present":true}))
        }
        Dns01Request::Cleanup { id } => cleanup(r, &id),
        Dns01Request::VerifyOrigin { id, hostname: host } => {
            uuid(&id)?;
            let host = hostname(&host)?;
            let bytes = r.read_file(Path::new(&format!(
                "/etc/panel/dns01/certs/{id}/fullchain.pem"
            )))?;
            let chain =
                std::str::from_utf8(&bytes).map_err(|_| error("Kandidat PEM tidak valid."))?;
            r.verify_origin_tls(&host, chain)?;
            Ok(json!({"verified":true}))
        }
        Dns01Request::StageCertificate {
            id,
            hostname: host,
            names,
            fullchain_pem,
            private_key_pem,
        } => {
            uuid(&id)?;
            let host = hostname(&host)?;
            if !names.contains(&host) {
                return Err(error("Hostname utama tidak ada pada kandidat."));
            }
            panel_core::panel_tls::validate_dns_candidate(
                &fullchain_pem,
                &private_key_pem,
                &names,
                chrono::Utc::now().timestamp(),
            )
            .map_err(error)?;
            let dir = format!("/etc/panel/dns01/certs/{id}");
            r.create_dir_all(Path::new(&dir), 0o700, 0, 0)?;
            let cert = format!("{dir}/fullchain.pem");
            let key = format!("{dir}/privkey.pem");
            for (path, bytes) in [
                (&cert, fullchain_pem.as_bytes()),
                (&key, private_key_pem.as_bytes()),
            ] {
                if r.path_exists(Path::new(path)) {
                    if r.read_file(Path::new(path))? != bytes {
                        return Err(error("Kandidat immutable berbeda; tidak ditimpa."));
                    }
                } else {
                    r.write_file(Path::new(path), bytes, 0o600)?;
                    r.set_ownership(Path::new(path), 0, 0)?;
                }
            }
            Ok(json!({"cert_path":cert,"key_path":key}))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn dns01_candidate_is_immutable_validated_and_private_without_changing_live_files() {
        let r = crate::system::MockSystemRunner::new();
        let id = uuid::Uuid::new_v4().to_string();
        let names = vec!["app.example.com".to_owned()];
        let params = rcgen::CertificateParams::new(names.clone()).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap().pem();
        let request = Dns01Request::StageCertificate {
            id: id.clone(),
            hostname: names[0].clone(),
            names: names.clone(),
            fullchain_pem: cert,
            private_key_pem: key.serialize_pem(),
        };
        let result = execute(&r, request.clone()).unwrap();
        execute(&r, request.clone()).unwrap();
        assert_eq!(
            result["key_path"],
            format!("/etc/panel/dns01/certs/{id}/privkey.pem")
        );
        let mut bad = request;
        if let Dns01Request::StageCertificate {
            private_key_pem, ..
        } = &mut bad
        {
            *private_key_pem = rcgen::KeyPair::generate().unwrap().serialize_pem();
        }
        assert!(execute(&r, bad).is_err());
        assert!(
            r.recorded_actions()
                .iter()
                .all(|a| !matches!(a, crate::system::MockAction::Command { .. }))
        );
        assert!(
            r.recorded_actions()
                .iter()
                .any(|a| matches!(a, crate::system::MockAction::WriteFile { mode: 0o600, .. }))
        );
    }
    #[test]
    fn dns01_provider_txt_quoting_does_not_change_ownership_but_extra_segments_do() {
        let id = "a416cf3a-e13b-45b0-a905-b3312bd12a99";
        let entry = Entry {
            name: "_acme-challenge.app.example.com".into(),
            digest: "A".repeat(43),
            record_id: None,
            created_at: 0,
        };
        for content in [entry.digest.clone(), format!("\"{}\"", entry.digest)] {
            let record = json!({"type":"TXT","name":entry.name,"content":content,"comment":format!("qpanel-acme:{id}")});
            assert!(owned(&record, id, &entry));
        }
        let record = json!({"type":"TXT","name":entry.name,"content":format!("\"{}\" \"operator\"",entry.digest),"comment":format!("qpanel-acme:{id}")});
        assert!(!owned(&record, id, &entry));
    }
}
