//! Shared candidate validation. IP identities never match DNS SANs or Common Name.
use std::net::IpAddr;
use tokio_rustls::rustls::{
    crypto,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    sign::CertifiedKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validity {
    pub not_before: i64,
    pub not_after: i64,
}

pub fn validate_ip_candidate(
    chain: &str,
    key: &str,
    ip: IpAddr,
    now: i64,
) -> Result<Validity, &'static str> {
    validate_candidate(chain, key, Some(ip), &[], now)
}

/// Exact DNS SANs only; wildcards and IP literals are outside DNS-01 scope.
pub fn validate_dns_candidate(
    chain: &str,
    key: &str,
    names: &[String],
    now: i64,
) -> Result<Validity, &'static str> {
    if names.is_empty()
        || names.len() > 20
        || names.iter().any(|n| {
            n.parse::<IpAddr>().is_ok() || crate::DomainName::parse(n).is_err() || n.contains('*')
        })
    {
        return Err("Invalid DNS certificate identities.");
    }
    validate_candidate(chain, key, None, names, now)
}
fn validate_candidate(
    chain: &str,
    key: &str,
    ip: Option<IpAddr>,
    names: &[String],
    now: i64,
) -> Result<Validity, &'static str> {
    if chain.len() > 128 * 1024 || key.len() > 16 * 1024 {
        return Err("Berkas sertifikat terlalu besar.");
    }
    let chain = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "PEM sertifikat tidak valid.")?;
    let leaf = chain.first().ok_or("Rantai sertifikat kosong.")?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf).map_err(|_| "X.509 tidak valid.")?;
    let validity = Validity {
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
    };
    if validity.not_before > now || validity.not_after <= now + 3600 {
        return Err("Sertifikat belum berlaku atau tersisa kurang dari satu jam.");
    }
    if cert.is_ca() {
        return Err("Sertifikat CA tidak boleh dipakai sebagai leaf panel.");
    }
    if cert
        .extended_key_usage()
        .map_err(|_| "EKU sertifikat tidak valid.")?
        .is_some_and(|eku| !eku.value.server_auth)
    {
        return Err("Sertifikat tidak mengizinkan TLS server.");
    }
    let matched = cert
        .subject_alternative_name()
        .map_err(|_| "SAN tidak valid.")?
        .is_some_and(|san| {
            if ip.is_none() {
                return names.iter().all(|required| san.value.general_names.iter().any(|name| matches!(name, x509_parser::extensions::GeneralName::DNSName(dns) if dns.eq_ignore_ascii_case(required))));
            }
            san.value.general_names.iter().any(|name| {
                if let x509_parser::extensions::GeneralName::IPAddress(bytes) = name {
                    match ip {
                        Some(IpAddr::V4(v)) => *bytes == v.octets(),
                        Some(IpAddr::V6(v)) => *bytes == v.octets(),
                        None => false,
                    }
                } else {
                    false
                }
            })
        });
    if !matched {
        return Err(if ip.is_some() {
            "Sertifikat harus memiliki SAN bertipe IP yang cocok."
        } else {
            "Sertifikat harus mencakup seluruh SAN DNS yang diminta."
        });
    }
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|_| "PEM private key tidak valid.")?;
    let pair = CertifiedKey::from_der(chain, key, &crypto::ring::default_provider())
        .map_err(|_| "Private key dan sertifikat tidak cocok.")?;
    pair.keys_match()
        .map_err(|_| "Private key dan sertifikat tidak cocok.")?;
    Ok(validity)
}

/// Proportional fallback: renew after 2/3 of actual lifetime, at most 30 days
/// before expiry. For 160h certificates this leaves about 53h, not minus 30 days.
pub fn renewal_at(not_before: i64, not_after: i64, ari: Option<(i64, i64)>) -> i64 {
    let lifetime = not_after.saturating_sub(not_before).max(1);
    let fallback = not_after.saturating_sub((lifetime / 3).clamp(1, 30 * 86400));
    if let Some((start, end)) =
        ari.filter(|(s, e)| *s >= not_before && *e > *s && *e < not_after.saturating_sub(3600))
    {
        return start.saturating_add((end - start) / 2).min(fallback);
    }
    fallback
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    fn fixture(
        san: rcgen::SanType,
        nb: i64,
        na: i64,
        eku: Vec<rcgen::ExtendedKeyUsagePurpose>,
    ) -> (String, String) {
        let mut p = rcgen::CertificateParams::default();
        p.subject_alt_names = vec![san];
        p.extended_key_usages = eku;
        p.not_before = time::OffsetDateTime::from_unix_timestamp(nb).unwrap();
        p.not_after = time::OffsetDateTime::from_unix_timestamp(na).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        (p.self_signed(&key).unwrap().pem(), key.serialize_pem())
    }
    #[test]
    fn ip_san_key_pair_and_validity_are_required_for_both_families() {
        let now = chrono::Utc::now().timestamp();
        for ip in ["1.1.1.1", "2606:4700:4700::1111"] {
            let ip: IpAddr = ip.parse().unwrap();
            let (cert, key) = fixture(
                rcgen::SanType::IpAddress(ip),
                now - 60,
                now + 160 * 3600,
                vec![],
            );
            assert!(validate_ip_candidate(&cert, &key, ip, now).is_ok());
            assert!(
                validate_ip_candidate(
                    &cert,
                    &rcgen::KeyPair::generate().unwrap().serialize_pem(),
                    ip,
                    now
                )
                .is_err()
            );
            assert!(validate_ip_candidate(&cert, &key, "8.8.8.8".parse().unwrap(), now).is_err());
            assert!(validate_ip_candidate(&cert, &key, ip, now - 120).is_err());
            assert!(validate_ip_candidate(&cert, &key, ip, now + 160 * 3600 - 60).is_err());
        }
    }
    #[test]
    fn dns_san_that_looks_like_ip_is_not_an_ip_identity_and_client_only_is_rejected() {
        let now = chrono::Utc::now().timestamp();
        let ip = "1.1.1.1".parse().unwrap();
        let (cert, key) = fixture(
            rcgen::SanType::DnsName("1.1.1.1".try_into().unwrap()),
            now - 60,
            now + 86400,
            vec![],
        );
        assert!(validate_ip_candidate(&cert, &key, ip, now).is_err());
        let (cert, key) = fixture(
            rcgen::SanType::IpAddress(ip),
            now - 60,
            now + 86400,
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth],
        );
        assert!(validate_ip_candidate(&cert, &key, ip, now).is_err());
        assert!(validate_ip_candidate("not a certificate", "not a key", ip, now).is_err());
        assert!(validate_ip_candidate(&"X".repeat(128 * 1024 + 1), "", ip, now).is_err());
    }
    #[test]
    fn short_lifetime_and_ari_do_not_use_a_thirty_day_window() {
        let lifetime = 160 * 3600;
        assert_eq!(renewal_at(0, lifetime, None), lifetime - lifetime / 3);
        assert!(renewal_at(0, lifetime, None) > 100 * 3600);
        assert_eq!(renewal_at(0, 90 * 86400, None), 60 * 86400);
        assert_eq!(renewal_at(0, lifetime, Some((3600, 7200))), 5400);
        assert_eq!(
            renewal_at(0, lifetime, Some((lifetime - 100, lifetime + 100))),
            renewal_at(0, lifetime, None)
        );
        assert_eq!(
            renewal_at(0, lifetime, Some((-1, 7200))),
            renewal_at(0, lifetime, None)
        );
    }
    #[test]
    fn dns01_requires_every_exact_dns_san_and_a_matching_server_key() {
        let now = chrono::Utc::now().timestamp();
        let names = vec!["app.example.com".to_owned(), "www.example.com".to_owned()];
        let mut params = rcgen::CertificateParams::new(names.clone()).unwrap();
        params.not_before = time::OffsetDateTime::from_unix_timestamp(now - 60).unwrap();
        params.not_after = time::OffsetDateTime::from_unix_timestamp(now + 90 * 86400).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let chain = params.self_signed(&key).unwrap().pem();
        assert!(validate_dns_candidate(&chain, &key.serialize_pem(), &names, now).is_ok());
        for required in [
            vec!["missing.example.com".into()],
            vec!["*.example.com".into()],
            vec!["1.1.1.1".into()],
            Vec::new(),
        ] {
            assert!(validate_dns_candidate(&chain, &key.serialize_pem(), &required, now).is_err());
        }
        assert!(
            validate_dns_candidate(
                &chain,
                &rcgen::KeyPair::generate().unwrap().serialize_pem(),
                &names,
                now
            )
            .is_err()
        );
        assert!(
            validate_dns_candidate(&chain, &key.serialize_pem(), &names, now + 90 * 86400).is_err()
        );
        let mut cn_only = rcgen::CertificateParams::default();
        cn_only
            .distinguished_name
            .push(rcgen::DnType::CommonName, "app.example.com");
        assert!(
            validate_dns_candidate(
                &cn_only.self_signed(&key).unwrap().pem(),
                &key.serialize_pem(),
                &names[..1],
                now
            )
            .is_err()
        );
    }
}
