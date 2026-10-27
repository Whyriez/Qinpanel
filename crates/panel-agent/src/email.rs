//! Panel-owned Postfix/Dovecot/OpenDKIM. All paths and queries derive from typed identities.
use crate::{AgentError, crypto, system::SystemRunner};
use panel_core::{DomainName, Username};
use panel_proto::{EmailFolder, EmailMailbox, EmailRequest, EmailServerConfig, EmailServiceAction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Mutex,
};

static LOCK: Mutex<()> = Mutex::new(());
const DIR: &str = "/etc/panel/mail";
const MANIFEST: &str = "/etc/panel/mail/managed.json";
const CONTEXT: &str = "qpanel-smtp:v1:";
const SERVICES: [&str; 3] = ["opendkim", "dovecot", "postfix"];
#[derive(Default, Serialize, Deserialize, Clone)]
struct Manifest {
    server: Option<EmailServerConfig>,
    #[serde(default)]
    ready: bool,
    mailboxes: BTreeMap<String, EmailMailbox>,
    #[serde(default)]
    suspended: BTreeSet<u32>,
}
fn invalid(s: &str) -> AgentError {
    AgentError::InvalidInput(s.into())
}
fn load(r: &dyn SystemRunner) -> Result<Manifest, AgentError> {
    if !r.path_exists(Path::new(MANIFEST)) {
        return Ok(Manifest::default());
    }
    serde_json::from_slice(&r.read_file(Path::new(MANIFEST))?)
        .map_err(|_| invalid("Manifest email rusak; hentikan perubahan dan periksa backup"))
}
fn save(r: &dyn SystemRunner, m: &Manifest) -> Result<(), AgentError> {
    r.create_dir_all(Path::new(DIR), 0o700, 0, 0)?;
    r.write_file(
        Path::new("/etc/panel/mail/managed.tmp"),
        &serde_json::to_vec(m).map_err(|_| invalid("Manifest tidak dapat disimpan"))?,
        0o600,
    )?;
    r.move_path(
        Path::new("/etc/panel/mail/managed.tmp"),
        Path::new(MANIFEST),
    )
}
fn run(r: &dyn SystemRunner, bin: &str, args: &[&str]) -> Result<Vec<u8>, AgentError> {
    let out = r.run_command(bin, args)?;
    if !out.status.success() {
        return Err(invalid(
            "Perintah layanan email gagal; periksa journal layanan",
        ));
    }
    Ok(out.stdout)
}
fn canonical_id(id: &str) -> Result<(), AgentError> {
    if !uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) {
        return Err(invalid("ID mailbox tidak valid"));
    }
    Ok(())
}
fn domain(name: &str) -> Result<String, AgentError> {
    DomainName::parse(name)
        .map(|d| d.as_ascii().to_owned())
        .map_err(|_| invalid("Domain email tidak valid"))
}
pub fn validate_address(value: &str) -> Result<(), AgentError> {
    let (local, host) = value
        .split_once('@')
        .ok_or_else(|| invalid("Alamat email tidak valid"))?;
    if local.is_empty()
        || local.len() > 64
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || value.len() > 254
        || domain(host)? != host
        || value != value.to_ascii_lowercase()
    {
        return Err(invalid("Gunakan alamat email huruf kecil yang valid"));
    }
    Ok(())
}
fn unix_user(r: &dyn SystemRunner, name: &str, uid: u32) -> Result<u32, AgentError> {
    Username::parse(name).map_err(|_| invalid("Username tenant tidak valid"))?;
    if !(10000..=59999).contains(&uid) {
        return Err(invalid("UID tenant tidak valid"));
    }
    let line = String::from_utf8(run(r, "getent", &["passwd", name])?)
        .map_err(|_| invalid("Identitas Linux tidak valid"))?;
    let fields = line.trim().split(':').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0] != name || fields[2].parse::<u32>().ok() != Some(uid) {
        return Err(invalid("Identitas Linux berbeda dari tenant panel"));
    }
    fields[3]
        .parse::<u32>()
        .ok()
        .filter(|id| (10000..=59999).contains(id))
        .ok_or_else(|| invalid("GID tenant tidak valid"))
}
fn service_gid(r: &dyn SystemRunner, name: &str) -> Result<u32, AgentError> {
    let line = String::from_utf8(run(r, "getent", &["group", name])?)
        .map_err(|_| invalid("Grup layanan tidak valid"))?;
    line.split(':')
        .nth(2)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid("Grup layanan tidak ditemukan"))
}
fn server_valid(r: &dyn SystemRunner, s: &EmailServerConfig) -> Result<(), AgentError> {
    if domain(&s.hostname)? != s.hostname
        || s.certificate_id <= 0
        || domain(&s.certificate_domain)? != s.certificate_domain
    {
        return Err(invalid("Hostname atau sertifikat email tidak valid"));
    }
    for file in ["fullchain.pem", "privkey.pem"] {
        if !r.path_exists(Path::new(&format!(
            "/etc/panel/ssl/{}/{file}",
            s.certificate_domain
        ))) {
            return Err(invalid("Sertifikat TLS panel belum diterapkan"));
        }
    }
    Ok(())
}
fn mailbox_valid(r: &dyn SystemRunner, m: &EmailMailbox) -> Result<u32, AgentError> {
    canonical_id(&m.id)?;
    validate_address(&m.address)?;
    // Only PHC Argon2id strings; no passdb separators, raw password, or scheme injection.
    if !m.password_hash.0.starts_with("$argon2id$v=19$m=")
        || m.password_hash.0.len() > 256
        || !m
            .password_hash
            .0
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"$=,+/".contains(&b))
        || !(10..=102400).contains(&m.quota_mb)
    {
        return Err(invalid("Hash atau kuota mailbox tidak valid"));
    }
    unix_user(r, &m.linux_username, m.uid)
}
fn write(r: &dyn SystemRunner, path: &str, text: &str) -> Result<(), AgentError> {
    r.write_file(Path::new(path), text.as_bytes(), 0o644)
}
fn configure(r: &dyn SystemRunner, m: &Manifest) -> Result<(), AgentError> {
    let s = m
        .server
        .as_ref()
        .ok_or_else(|| invalid("Server email belum dikonfigurasi"))?;
    server_valid(r, s)?;
    let os = r.operating_system()?;
    let rpm = os.is_rpm();
    let map_kind = if rpm { "lmdb" } else { "hash" };
    let (compatibility_level, tls_protocols) =
        if os == panel_core::platform::LinuxPlatform::AlmaLinux9 {
            ("2", "!SSLv2,!SSLv3,!TLSv1,!TLSv1.1")
        } else {
            ("3.6", ">=TLSv1.2")
        };
    let mut domains = BTreeSet::new();
    let mut recipients = String::new();
    let mut senders = String::new();
    let mut passwords = String::new();
    let mut userdb = String::new();
    let dovecot_gid = service_gid(r, "dovecot")?;
    for mailbox in m.mailboxes.values() {
        let gid = mailbox_valid(r, mailbox)?;
        if !mailbox.enabled || m.suspended.contains(&mailbox.uid) {
            continue;
        }
        let host = mailbox
            .address
            .split_once('@')
            .ok_or_else(|| invalid("Alamat email tidak valid"))?
            .1;
        domains.insert(host.to_string());
        // Root-owned parents prevent tenants replacing mailbox roots with symlinks.
        r.create_dir_all(Path::new("/var/lib/panel/mail"), 0o755, 0, 0)?;
        r.create_dir_all(
            Path::new(&format!("/var/lib/panel/mail/{}", mailbox.uid)),
            0o750,
            0,
            gid,
        )?;
        let home = format!("/var/lib/panel/mail/{}/{}", mailbox.uid, mailbox.id);
        r.create_dir_all(Path::new(&home), 0o700, mailbox.uid, gid)?;
        // Dovecot drops to the mailbox UID. Permit traversal only, without
        // listing panel state or granting read access to its database/secrets.
        run(
            r,
            "setfacl",
            &[
                "-m",
                &format!("u:{}:--x", mailbox.linux_username),
                "--",
                "/var/lib/panel",
            ],
        )?;
        // Dovecot creates Maildir as the tenant; root never follows its contents.
        recipients.push_str(&format!("{} OK\n", mailbox.address));
        senders.push_str(&format!("{} {}\n", mailbox.address, mailbox.address));
        passwords.push_str(&format!("{}:{{ARGON2ID}}{}:{}:{}::{home}::userdb_mail=maildir:{home}/Maildir userdb_quota_rule=*:storage={}M\n",mailbox.address,mailbox.password_hash.0,mailbox.uid,gid,mailbox.quota_mb));
        userdb.push_str(&format!("{}::{}:{}::{home}::userdb_mail=maildir:{home}/Maildir userdb_quota_rule=*:storage={}M\n", mailbox.address, mailbox.uid, gid, mailbox.quota_mb));
    }
    let domains = domains.into_iter().collect::<Vec<_>>();
    let mut keys = String::new();
    let mut signing = String::new();
    let dkim_gid = service_gid(r, "opendkim")?;
    let postfix_gid = service_gid(r, "postfix")?;
    let service = String::from_utf8(run(r, "getent", &["passwd", "opendkim"])?)
        .map_err(|_| invalid("Identitas OpenDKIM tidak valid"))?;
    let dkim_uid = service
        .split(':')
        .nth(2)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|id| (1..10000).contains(id))
        .ok_or_else(|| invalid("UID layanan OpenDKIM tidak valid"))?;
    // Persistent parent and inherited ACL survive socket recreation and reboot.
    // Unprivileged tenants must never reach the milter protocol and forge macros.
    r.create_dir_all(
        Path::new("/var/spool/postfix/qpanel-dkim"),
        0o750,
        dkim_uid,
        postfix_gid,
    )?;
    run(
        r,
        "setfacl",
        &[
            "-m",
            "u:postfix:rx,d:u::rwx,d:g::---,d:g:postfix:rwx,d:m::rwx,d:o::---",
            "/var/spool/postfix/qpanel-dkim",
        ],
    )?;
    for host in &domains {
        let dir = format!("/etc/opendkim/keys/{host}");
        r.create_dir_all(Path::new(&dir), 0o750, 0, dkim_gid)?;
        if !r.path_exists(Path::new(&format!("{dir}/qpanel.private"))) {
            run(
                r,
                "opendkim-genkey",
                &["-D", &dir, "-d", host, "-s", "qpanel", "-b", "2048"],
            )?;
        }
        r.set_permissions(Path::new(&format!("{dir}/qpanel.private")), 0o640)?;
        r.set_ownership(Path::new(&format!("{dir}/qpanel.private")), 0, dkim_gid)?;
        keys.push_str(&format!(
            "qpanel._domainkey.{host} {host}:qpanel:{dir}/qpanel.private\n"
        ));
        signing.push_str(&format!("*@{host} qpanel._domainkey.{host}\n"));
    }
    write(r, "/etc/opendkim/KeyTable", &keys)?;
    write(r, "/etc/opendkim/SigningTable", &signing)?;
    write(r, "/etc/opendkim/TrustedHosts", "")?;
    write(
        r,
        "/etc/opendkim.conf",
        "# Managed by QonPanel\nSyslog yes\nUMask 007\nUserID opendkim\nMode sv\nCanonicalization relaxed/simple\nSocket local:/var/spool/postfix/qpanel-dkim/opendkim.sock\nKeyTable refile:/etc/opendkim/KeyTable\nSigningTable refile:/etc/opendkim/SigningTable\nInternalHosts file:/etc/opendkim/TrustedHosts\nMTA csl:QpanelLocal\nOversignHeaders From\nSenderMacro {mail_addr}\n",
    )?;
    write(
        r,
        "/etc/postfix/virtual_domains",
        &domains
            .iter()
            .map(|d| format!("{d} OK\n"))
            .collect::<String>(),
    )?;
    write(r, "/etc/postfix/virtual_mailboxes", &recipients)?;
    write(r, "/etc/postfix/sender_login", &senders)?;
    for table in ["virtual_domains", "virtual_mailboxes", "sender_login"] {
        run(r, "postmap", &[&format!("{map_kind}:/etc/postfix/{table}")])?;
    }
    let cert = format!("/etc/panel/ssl/{}", s.certificate_domain);
    write(
        r,
        "/etc/postfix/main.cf",
        &format!(
            "# Managed by QonPanel\ncompatibility_level = {compatibility_level}\nmyhostname = {}\nmydestination = localhost\ninet_interfaces = all\ninet_protocols = all\nmynetworks = 127.0.0.0/8 [::1]/128\nsmtpd_relay_restrictions = permit_sasl_authenticated, reject_unauth_destination\nsmtpd_recipient_restrictions = reject_unlisted_recipient, reject_unauth_destination\nvirtual_mailbox_domains = {map_kind}:/etc/postfix/virtual_domains\nvirtual_mailbox_maps = {map_kind}:/etc/postfix/virtual_mailboxes\nvirtual_transport = lmtp:unix:private/dovecot-lmtp\nsmtpd_tls_cert_file = {cert}/fullchain.pem\nsmtpd_tls_key_file = {cert}/privkey.pem\nsmtpd_tls_security_level = may\nsmtpd_tls_protocols = {tls_protocols}\nsmtpd_tls_mandatory_protocols = {tls_protocols}\nsmtp_tls_security_level = may\nsmtpd_sasl_type = dovecot\nsmtpd_sasl_path = private/auth\nsmtpd_sasl_auth_enable = no\nsmtpd_sender_login_maps = {map_kind}:/etc/postfix/sender_login\nsmtpd_milters = unix:/var/spool/postfix/qpanel-dkim/opendkim.sock\nnon_smtpd_milters = unix:/var/spool/postfix/qpanel-dkim/opendkim.sock\nmilter_macro_daemon_name = QpanelLocal\nmilter_default_action = tempfail\nmilter_protocol = 6\nmilter_mail_macros = i {{mail_addr}} {{mail_host}} {{mail_mailer}} {{auth_type}} {{auth_authen}}\nauthorized_submit_users = root, panel-api\nmessage_size_limit = 10485760\ndisable_vrfy_command = yes\n",
            s.hostname
        ),
    )?;
    write(
        r,
        "/etc/postfix/master.cf",
        "# Managed by QonPanel\nsmtp inet n - n - - smtpd\n  -o milter_macro_daemon_name=QpanelInbound\nsubmission inet n - n - - smtpd\n  -o milter_macro_daemon_name=QpanelSubmission\n  -o smtpd_tls_security_level=encrypt\n  -o smtpd_sasl_auth_enable=yes\n  -o smtpd_sender_restrictions=reject_authenticated_sender_login_mismatch\n  -o smtpd_recipient_restrictions=permit_sasl_authenticated,reject\npickup unix n - n 60 1 pickup\ncleanup unix n - n - 0 cleanup\nqmgr unix n - n 300 1 qmgr\ntlsmgr unix - - n 1000? 1 tlsmgr\nrewrite unix - - n - - trivial-rewrite\nbounce unix - - n - 0 bounce\ndefer unix - - n - 0 bounce\ntrace unix - - n - 0 bounce\nverify unix - - n - 1 verify\nflush unix n - n 1000? 0 flush\nproxymap unix - - n - - proxymap\nproxywrite unix - - n - 1 proxymap\nsmtp unix - - n - - smtp\nrelay unix - - n - - smtp\nshowq unix n - n - - showq\nerror unix - - n - - error\nretry unix - - n - - error\ndiscard unix - - n - - discard\nlocal unix - n n - - local\nvirtual unix - n n - - virtual\nlmtp unix - - n - - lmtp\nanvil unix - - n - 1 anvil\nscache unix - - n - 1 scache\n",
    )?;
    r.write_file(
        Path::new("/etc/dovecot/qpanel-users"),
        passwords.as_bytes(),
        0o640,
    )?;
    r.set_ownership(Path::new("/etc/dovecot/qpanel-users"), 0, dovecot_gid)?;
    if rpm {
        r.write_file(
            Path::new("/etc/dovecot/qpanel-userdb"),
            userdb.as_bytes(),
            0o640,
        )?;
        r.set_ownership(Path::new("/etc/dovecot/qpanel-userdb"), 0, dovecot_gid)?;
    }
    let authentication = if rpm {
        "passdb {\n driver = checkpassword\n args = /usr/local/libexec/qonpanel-mail-auth\n}\nuserdb {\n driver = passwd-file\n args = /etc/dovecot/qpanel-userdb\n}\n"
    } else {
        "passdb {\n driver = passwd-file\n args = /etc/dovecot/qpanel-users\n}\nuserdb {\n driver = passwd-file\n args = /etc/dovecot/qpanel-users\n}\n"
    };
    write(
        r,
        "/etc/dovecot/dovecot.conf",
        &format!(
            r#"# Managed by QonPanel
protocols = imap lmtp
listen = *, ::
ssl = required
ssl_min_protocol = TLSv1.2
ssl_cert = <{cert}/fullchain.pem
ssl_key = <{cert}/privkey.pem
disable_plaintext_auth = yes
auth_mechanisms = plain login
auth_username_format = %Lu
first_valid_uid = 10000
last_valid_uid = 59999
mail_location = maildir:~/Maildir
mail_plugins = quota
{authentication}
namespace inbox {{
 inbox = yes
 separator = /
 mailbox Sent {{
  auto = subscribe
  special_use = \Sent
 }}
 mailbox Trash {{
  auto = subscribe
  special_use = \Trash
 }}
}}
plugin {{
 quota = maildir:Mailbox quota
}}
protocol imap {{
 mail_plugins = $mail_plugins imap_quota
}}
protocol lmtp {{
 postmaster_address = postmaster@{hostname}
 mail_plugins = $mail_plugins
}}
service imap-login {{
 inet_listener imap {{
  port = 0
 }}
 inet_listener imaps {{
  port = 993
  ssl = yes
 }}
}}
service auth {{
 unix_listener /var/spool/postfix/private/auth {{
  mode = 0660
  user = postfix
  group = postfix
 }}
}}
service lmtp {{
 unix_listener /var/spool/postfix/private/dovecot-lmtp {{
  mode = 0660
  user = postfix
  group = postfix
 }}
}}
"#,
            hostname = s.hostname
        ),
    )?;
    for path in [
        "/etc/postfix",
        "/etc/dovecot",
        "/etc/opendkim",
        "/etc/opendkim.conf",
        "/var/lib/panel/mail",
        "/var/spool/postfix/qpanel-dkim",
    ] {
        if r.path_exists(Path::new(path)) {
            crate::platform::relabel(r, Path::new(path))?;
        }
    }
    run(r, "postfix", &["check"])?;
    run(r, "doveconf", &["-n"])?;
    run(r, "opendkim", &["-n", "-x", "/etc/opendkim.conf"])?;
    Ok(())
}
const CONFIG_FILES: [&str; 12] = [
    "/etc/postfix/main.cf",
    "/etc/postfix/master.cf",
    "/etc/postfix/virtual_domains",
    "/etc/postfix/virtual_mailboxes",
    "/etc/postfix/sender_login",
    "/etc/dovecot/dovecot.conf",
    "/etc/dovecot/qpanel-users",
    "/etc/dovecot/qpanel-userdb",
    "/etc/opendkim.conf",
    "/etc/opendkim/KeyTable",
    "/etc/opendkim/SigningTable",
    "/etc/opendkim/TrustedHosts",
];
fn apply(r: &dyn SystemRunner, m: &Manifest) -> Result<(), AgentError> {
    let previous_ready = load(r)?.ready;
    let snapshot = CONFIG_FILES
        .iter()
        .map(|path| {
            Ok((
                *path,
                if r.path_exists(Path::new(path)) {
                    Some(r.read_file(Path::new(path))?)
                } else {
                    None
                },
            ))
        })
        .collect::<Result<Vec<_>, AgentError>>()?;
    let result = configure(r, m).and_then(|()| {
        for service in SERVICES {
            run(r, "systemctl", &["restart", service])?;
        }
        save(r, m)
    });
    if let Err(error) = result {
        for (path, content) in snapshot {
            if let Some(content) = content {
                let mode = if path.ends_with("qpanel-users") || path.ends_with("qpanel-userdb") {
                    0o640
                } else {
                    0o644
                };
                let _ = r.write_file(Path::new(path), &content, mode);
            } else {
                let _ = r.remove_file(Path::new(path));
            }
        }
        for table in ["virtual_domains", "virtual_mailboxes", "sender_login"] {
            let lmdb = r
                .read_file(Path::new("/etc/postfix/main.cf"))
                .is_ok_and(|bytes| {
                    String::from_utf8_lossy(&bytes).contains(&format!("lmdb:/etc/postfix/{table}"))
                });
            let kind = if lmdb { "lmdb" } else { "hash" };
            let _ = run(r, "postmap", &[&format!("{kind}:/etc/postfix/{table}")]);
        }
        for service in SERVICES {
            let _ = run(
                r,
                "systemctl",
                &[if previous_ready { "restart" } else { "stop" }, service],
            );
            if !previous_ready {
                let _ = run(r, "systemctl", &["disable", service]);
            }
        }
        return Err(error);
    }
    Ok(())
}
pub fn install(r: &dyn SystemRunner, server: EmailServerConfig) -> Result<Value, AgentError> {
    let _lock = LOCK
        .lock()
        .map_err(|_| invalid("Lock email tidak tersedia"))?;
    server_valid(r, &server)?;
    let mut m = load(r)?;
    if m.server.is_none()
        && [
            "/etc/postfix/main.cf",
            "/etc/dovecot/dovecot.conf",
            "/etc/opendkim.conf",
        ]
        .iter()
        .any(|p| r.path_exists(Path::new(p)))
    {
        return Err(invalid(
            "Konfigurasi mail existing bukan milik panel. Migrasikan secara manual sebelum instalasi",
        ));
    }
    if m.ready {
        m.server = Some(server);
        apply(r, &m)?;
        for service in SERVICES {
            run(r, "systemctl", &["enable", service])?;
        }
        return status(r, &m);
    }
    let rpm = r.operating_system()?.is_rpm();
    if !rpm && r.path_exists(Path::new("/usr/sbin/policy-rc.d")) {
        return Err(invalid(
            "policy-rc.d sudah ada. Tinjau kebijakan instalasi server sebelum melanjutkan",
        ));
    }
    m.server = Some(server);
    m.ready = false;
    save(r, &m)?;
    let package_result = if rpm {
        install_rpm_mail(r)
    } else {
        r.write_file(
            Path::new("/usr/sbin/policy-rc.d"),
            b"#!/bin/sh\nexit 101\n",
            0o755,
        )?;
        let result = r.run_command_streamed(
            "apt-get",
            &[
                "install",
                "-y",
                "postfix",
                "dovecot-core",
                "dovecot-imapd",
                "dovecot-lmtpd",
                "opendkim",
                "opendkim-tools",
                "acl",
            ],
            &|_| {},
        );
        let cleanup = r.remove_file(Path::new("/usr/sbin/policy-rc.d"));
        cleanup?;
        result.map(|_| ())
    };
    if let Err(error) = package_result {
        for service in SERVICES {
            let _ = run(r, "systemctl", &["stop", service]);
            let _ = run(r, "systemctl", &["disable", service]);
        }
        return Err(error);
    }
    // APT may enable units even though policy-rc.d prevented starting them.
    // Keep fresh/default configurations disabled across reboot until validation succeeds.
    let mut disabled = Ok(());
    for service in SERVICES {
        if let Err(error) = run(r, "systemctl", &["disable", service]) {
            disabled = Err(error);
        }
    }
    disabled?;
    let schemes = String::from_utf8_lossy(&run(r, "doveadm", &["pw", "-l"])?).to_string();
    if rpm {
        run(r, "/usr/local/libexec/qonpanel-mail-auth", &["--check"])?;
    } else if !schemes.split_whitespace().any(|s| s == "ARGON2ID") {
        return Err(invalid(
            "Dovecot harus mendukung password ARGON2ID; layanan belum diaktifkan",
        ));
    }
    m.ready = true;
    apply(r, &m)?;
    for service in SERVICES {
        run(r, "systemctl", &["enable", service])?;
    }
    status(r, &m)
}

fn install_rpm_mail(r: &dyn SystemRunner) -> Result<(), AgentError> {
    let journal = Path::new("/etc/panel/mail/package-masks.json");
    let mut masks: Vec<String> = if r.path_exists(journal) {
        serde_json::from_slice(&r.read_file(journal)?)
            .map_err(|_| invalid("Journal paket mail tidak valid"))?
    } else {
        Vec::new()
    };
    if masks.iter().any(|unit| !SERVICES.contains(&unit.as_str())) {
        return Err(invalid("Journal mask mail di luar cakupan"));
    }
    // Recover an interrupted transaction before starting another one.
    for service in &masks {
        run(r, "systemctl", &["unmask", "--runtime", service])?;
    }
    masks.clear();
    let result = (|| {
        for service in SERVICES {
            let state = String::from_utf8_lossy(&run(
                r,
                "systemctl",
                &["show", "--property=UnitFileState", "--value", service],
            )?)
            .trim()
            .to_string();
            if matches!(state.as_str(), "masked" | "masked-runtime") {
                continue;
            }
            masks.push(service.into());
            r.write_file(
                journal,
                &serde_json::to_vec(&masks).map_err(|_| invalid("Journal mask mail gagal"))?,
                0o600,
            )?;
            run(r, "systemctl", &["mask", "--runtime", service])?;
        }
        crate::platform::install(
            r,
            &[
                "postfix",
                "postfix-lmdb",
                "dovecot-core",
                "dovecot-imapd",
                "dovecot-lmtpd",
                "opendkim",
                "opendkim-tools",
                "acl",
                "libsodium",
            ],
        )?;
        Ok(())
    })();
    for service in &masks {
        run(r, "systemctl", &["unmask", "--runtime", service])?;
    }
    r.remove_file(journal)?;
    result
}
fn status(r: &dyn SystemRunner, m: &Manifest) -> Result<Value, AgentError> {
    let services = SERVICES
        .iter()
        .map(|s| {
            let running = r
                .run_command("systemctl", &["is-active", s])
                .is_ok_and(|out| {
                    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "active"
                });
            json!({"name":s,"running":running})
        })
        .collect::<Vec<_>>();
    Ok(json!({"configured":m.ready,"server":m.server,"services":services}))
}
fn folder(f: &EmailFolder) -> &'static str {
    match f {
        EmailFolder::Inbox => "INBOX",
        EmailFolder::Sent => "Sent",
        EmailFolder::Trash => "Trash",
    }
}
fn json_command(r: &dyn SystemRunner, args: &[&str]) -> Result<Vec<Value>, AgentError> {
    let bytes = run(r, "doveadm", args)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid(
            "Pesan terlalu besar untuk ditampilkan. Gunakan aplikasi IMAP",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Format respons Dovecot tidak valid"))
}
fn number(v: &Value) -> Option<u32> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
fn folder_status(
    r: &dyn SystemRunner,
    m: &EmailMailbox,
    f: &EmailFolder,
) -> Result<Value, AgentError> {
    json_command(
        r,
        &[
            "-f",
            "json",
            "mailbox",
            "status",
            "-u",
            &m.address,
            "messages unseen uidnext uidvalidity",
            folder(f),
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("Folder tidak ditemukan"))
}
fn check_uid(
    r: &dyn SystemRunner,
    m: &EmailMailbox,
    f: &EmailFolder,
    uid: u32,
    validity: u32,
) -> Result<(), AgentError> {
    if uid == 0
        || validity == 0
        || number(&folder_status(r, m, f)?["uidvalidity"]) != Some(validity)
    {
        return Err(invalid(
            "Daftar pesan berubah. Muat ulang folder sebelum melanjutkan",
        ));
    }
    Ok(())
}
pub fn execute(r: &dyn SystemRunner, request: EmailRequest) -> Result<Value, AgentError> {
    let _lock = LOCK
        .lock()
        .map_err(|_| invalid("Lock email tidak tersedia"))?;
    if let EmailRequest::SecretSeal { value } = request {
        if value.0.len() > 4096 {
            return Err(invalid("Secret terlalu panjang"));
        }
        let key = crypto::get_or_create_master_key(r, None)?;
        return Ok(json!({"value":crypto::encrypt_value(&key,&format!("{CONTEXT}{}",value.0))?}));
    }
    if let EmailRequest::SecretOpen { value } = request {
        if !value.0.starts_with(crypto::CIPHER_PREFIX) {
            return Err(invalid("Secret SMTP belum terenkripsi"));
        }
        let key = crypto::get_or_create_master_key(r, None)?;
        let plain = crypto::decrypt_value(&key, &value.0)?;
        return Ok(
            json!({"value":plain.strip_prefix(CONTEXT).ok_or_else(||invalid("Context secret SMTP berbeda"))?}),
        );
    }
    let mut manifest = load(r)?;
    if matches!(request, EmailRequest::Status) {
        return status(r, &manifest);
    }
    if !manifest.ready {
        return Err(invalid(
            "Server email belum siap. Administrator perlu menyelesaikan instalasi",
        ));
    }
    match request {
        EmailRequest::MailboxApply { mailbox } => {
            mailbox_valid(r, &mailbox)?;
            if manifest
                .mailboxes
                .values()
                .any(|m| m.address == mailbox.address && m.id != mailbox.id)
            {
                return Err(invalid("Alamat mailbox sudah ada"));
            }
            if manifest.mailboxes.get(&mailbox.id).is_some_and(|m| {
                m.address != mailbox.address
                    || m.uid != mailbox.uid
                    || m.linux_username != mailbox.linux_username
            }) {
                return Err(invalid("Identitas mailbox tidak boleh berubah"));
            }
            manifest.mailboxes.insert(mailbox.id.clone(), mailbox);
            apply(r, &manifest)?;
            Ok(json!({"applied":true}))
        }
        EmailRequest::MailboxRemove { id } => {
            canonical_id(&id)?;
            let removed = manifest.mailboxes.remove(&id);
            if let Some(m) = &removed {
                let home = format!("/var/lib/panel/mail/{}/{}", m.uid, m.id);
                r.set_permissions(Path::new(&home), 0o700)?;
                r.set_ownership(Path::new(&home), 0, 0)?;
            }
            if let Err(error) = apply(r, &manifest) {
                if let Some(m) = removed
                    && let Ok(gid) = unix_user(r, &m.linux_username, m.uid)
                {
                    let _ = r.set_ownership(
                        Path::new(&format!("/var/lib/panel/mail/{}/{}", m.uid, m.id)),
                        m.uid,
                        gid,
                    );
                }
                return Err(error);
            }
            Ok(json!({"removed":true,"messages_retained":true}))
        }
        EmailRequest::Service { operation } => {
            let action = match operation {
                EmailServiceAction::Start => "start",
                EmailServiceAction::Stop => "stop",
                EmailServiceAction::Restart => "restart",
            };
            for service in SERVICES {
                run(r, "systemctl", &[action, service])?;
            }
            status(r, &manifest)
        }
        EmailRequest::DnsRecords { domain: host } => {
            let host = domain(&host)?;
            let server = manifest
                .server
                .as_ref()
                .ok_or_else(|| invalid("Hostname mail belum disetel"))?;
            let txt = r.read_file(Path::new(&format!("/etc/opendkim/keys/{host}/qpanel.txt")))?;
            let txt = String::from_utf8(txt).map_err(|_| invalid("Record DKIM tidak valid"))?;
            let dkim = txt
                .split('"')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, s)| s)
                .collect::<String>();
            if !dkim.starts_with("v=DKIM1;") || !dkim.contains("p=") {
                return Err(invalid("Record DKIM belum tersedia"));
            }
            Ok(
                json!({"hostname":server.hostname,"records":[{"type":"MX","name":host,"content":server.hostname,"priority":10},{"type":"TXT","name":host,"content":"v=spf1 mx -all"},{"type":"TXT","name":format!("qpanel._domainkey.{host}"),"content":dkim},{"type":"TXT","name":format!("_dmarc.{host}"),"content":"v=DMARC1; p=none"}]}),
            )
        }
        other => {
            let id = match &other {
                EmailRequest::Messages { id, .. }
                | EmailRequest::Message { id, .. }
                | EmailRequest::MessageTrash { id, .. }
                | EmailRequest::Send { id, .. } => id,
                _ => return Err(invalid("Operasi email tidak sesuai")),
            };
            canonical_id(id)?;
            let m = manifest
                .mailboxes
                .get(id)
                .filter(|m| m.enabled && !manifest.suspended.contains(&m.uid))
                .ok_or_else(|| invalid("Mailbox tidak aktif atau tidak ditemukan"))?;
            unix_user(r, &m.linux_username, m.uid)?;
            match other {
                EmailRequest::Messages {
                    folder: f, before, ..
                } => {
                    let state = folder_status(r, m, &f)?;
                    let next = number(&state["uidnext"])
                        .ok_or_else(|| invalid("UID folder tidak valid"))?;
                    let end = before.unwrap_or(next).min(next).saturating_sub(1);
                    let start = end.saturating_sub(49).max(1);
                    let mut items = if end == 0 {
                        Vec::new()
                    } else {
                        json_command(
                            r,
                            &[
                                "-f",
                                "json",
                                "fetch",
                                "-u",
                                &m.address,
                                "uid flags hdr.from hdr.to hdr.subject date.received.unixtime size.physical",
                                "mailbox",
                                folder(&f),
                                "uid",
                                &format!("{start}:{end}"),
                            ],
                        )?
                    };
                    items.sort_by_key(|v| std::cmp::Reverse(number(&v["uid"]).unwrap_or(0)));
                    Ok(
                        json!({"items":items,"uid_validity":number(&state["uidvalidity"]),"unseen":number(&state["unseen"]),"total":number(&state["messages"]),"before":if start>1 {Some(start)} else {None}}),
                    )
                }
                EmailRequest::Message {
                    folder: f,
                    uid,
                    uid_validity,
                    ..
                } => {
                    check_uid(r, m, &f, uid, uid_validity)?;
                    let items = json_command(
                        r,
                        &[
                            "-f",
                            "json",
                            "fetch",
                            "-u",
                            &m.address,
                            "uid hdr.from hdr.to hdr.subject text.utf8",
                            "mailbox",
                            folder(&f),
                            "uid",
                            &uid.to_string(),
                            "smaller",
                            "1048576",
                        ],
                    )?;
                    let message=items.into_iter().next().ok_or_else(||invalid("Pesan tidak ada atau lebih dari 1 MiB. Gunakan aplikasi IMAP untuk pesan besar"))?;
                    run(
                        r,
                        "doveadm",
                        &[
                            "flags",
                            "add",
                            "-u",
                            &m.address,
                            "\\Seen",
                            "mailbox",
                            folder(&f),
                            "uid",
                            &uid.to_string(),
                        ],
                    )?;
                    Ok(message)
                }
                EmailRequest::MessageTrash {
                    folder: f,
                    uid,
                    uid_validity,
                    ..
                } => {
                    if f == EmailFolder::Trash {
                        return Err(invalid("Pesan sudah berada di Sampah"));
                    }
                    check_uid(r, m, &f, uid, uid_validity)?;
                    run(
                        r,
                        "doveadm",
                        &[
                            "move",
                            "-u",
                            &m.address,
                            "Trash",
                            "mailbox",
                            folder(&f),
                            "uid",
                            &uid.to_string(),
                        ],
                    )?;
                    Ok(json!({"moved":true}))
                }
                EmailRequest::Send {
                    to, subject, body, ..
                } => {
                    let mime = panel_core::email::build_message(
                        &m.address,
                        &m.address,
                        &panel_core::email::EmailMessage {
                            to: to.clone(),
                            subject,
                            body,
                        },
                    )
                    .map_err(|_| invalid("Isi atau alamat email tidak valid"))?;
                    r.run_command_with_input(
                        "sendmail",
                        &["-i", "-f", &m.address, "--", &to],
                        mime.as_bytes(),
                    )?;
                    let saved = r
                        .run_command_with_input(
                            "doveadm",
                            &["save", "-u", &m.address, "-m", "Sent"],
                            mime.as_bytes(),
                        )
                        .is_ok();
                    Ok(
                        json!({"accepted":true,"saved_to_sent":saved,"message":"Pesan diterima antrean Postfix. Penerimaan akhir bergantung pada server tujuan."}),
                    )
                }
                _ => Err(invalid("Operasi mailbox tidak sesuai")),
            }
        }
    }
}

/// Keep virtual credentials in step with the Linux tenant lifecycle.
pub fn tenant_access(
    r: &dyn SystemRunner,
    username: &str,
    uid: u32,
    suspended: bool,
    remove: bool,
) -> Result<(), AgentError> {
    let _lock = LOCK
        .lock()
        .map_err(|_| invalid("Lock email tidak tersedia"))?;
    let mut m = load(r)?;
    let accounts = m
        .mailboxes
        .values()
        .filter(|b| b.linux_username == username && b.uid == uid)
        .cloned()
        .collect::<Vec<_>>();
    if accounts.is_empty() {
        return Ok(());
    }
    if suspended {
        m.suspended.insert(uid);
    } else {
        m.suspended.remove(&uid);
    }
    if remove {
        for account in &accounts {
            // Root-owned parent protects this entry; retain the archive root-only.
            let home = format!("/var/lib/panel/mail/{}/{}", uid, account.id);
            r.set_permissions(Path::new(&home), 0o700)?;
            r.set_ownership(Path::new(&home), 0, 0)?;
            m.mailboxes.remove(&account.id);
        }
    }
    apply(r, &m)?;
    for account in accounts {
        let _ = run(r, "doveadm", &["kick", "-u", &account.address]);
    }
    Ok(())
}
pub fn certificate_changed(r: &dyn SystemRunner, domain: &str) -> Result<(), AgentError> {
    let _lock = LOCK
        .lock()
        .map_err(|_| invalid("Lock email tidak tersedia"))?;
    let m = load(r)?;
    if m.ready
        && m.server
            .as_ref()
            .is_some_and(|s| s.certificate_domain == domain)
    {
        for service in ["dovecot", "postfix"] {
            if r.run_command("systemctl", &["is-active", service])
                .is_ok_and(|out| {
                    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "active"
                })
            {
                run(r, "systemctl", &["reload", service])?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::system::{MockAction, MockSystemRunner};
    const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
    fn config() -> EmailServerConfig {
        EmailServerConfig {
            hostname: "mail.example.com".into(),
            certificate_id: 1,
            certificate_domain: "mail.example.com".into(),
        }
    }
    fn runner() -> MockSystemRunner {
        let r = MockSystemRunner::new();
        for f in ["fullchain.pem", "privkey.pem"] {
            r.set_file_content(format!("/etc/panel/ssl/mail.example.com/{f}"), b"fixture");
        }
        r.set_response(
            "getent passwd p_customer",
            0,
            b"p_customer:x:10001:10001::/home/p_customer:/usr/sbin/nologin\n",
            b"",
        );
        r.set_response("getent group dovecot", 0, b"dovecot:x:111:\n", b"");
        r.set_response("getent group opendkim", 0, b"opendkim:x:112:\n", b"");
        r.set_response("getent group postfix", 0, b"postfix:x:113:\n", b"");
        r.set_response(
            "getent passwd opendkim",
            0,
            b"opendkim:x:110:112::/run/opendkim:/usr/sbin/nologin",
            b"",
        );
        r.set_response("doveadm pw -l", 0, b"PLAIN SHA512-CRYPT ARGON2ID", b"");
        r.set_file_content(
            "/etc/opendkim/keys/example.com/qpanel.private",
            b"fixture-private",
        );
        r.set_file_content(
            "/etc/opendkim/keys/example.com/qpanel.txt",
            b"qpanel._domainkey IN TXT ( \"v=DKIM1; k=rsa; \" \"p=PUBLICKEY\" )",
        );
        r
    }
    fn box_config() -> EmailMailbox {
        EmailMailbox {
            id: ID.into(),
            address: "admin@example.com".into(),
            linux_username: "p_customer".into(),
            uid: 10001,
            password_hash: panel_proto::EmailSecret(
                panel_core::hash_password("SafeFixture123!").unwrap(),
            ),
            quota_mb: 1024,
            enabled: true,
        }
    }
    fn ready(r: &MockSystemRunner) {
        save(
            r,
            &Manifest {
                server: Some(config()),
                ready: true,
                ..Default::default()
            },
        )
        .unwrap();
    }
    fn contents(r: &MockSystemRunner, p: &str) -> String {
        String::from_utf8(r.read_file(Path::new(p)).unwrap()).unwrap()
    }
    #[test]
    fn smtp_secret_roundtrip_is_context_scoped_and_redacted() {
        let r = runner();
        let plain = "enc:v1:not-a-cipher";
        let sealed = execute(
            &r,
            EmailRequest::SecretSeal {
                value: panel_proto::EmailSecret(plain.into()),
            },
        )
        .unwrap()["value"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(sealed, plain);
        assert!(!sealed.contains("not-a-cipher"));
        assert_eq!(
            execute(
                &r,
                EmailRequest::SecretOpen {
                    value: panel_proto::EmailSecret(sealed)
                }
            )
            .unwrap()["value"],
            plain
        );
        assert!(
            execute(
                &r,
                EmailRequest::SecretOpen {
                    value: panel_proto::EmailSecret("plaintext".into())
                }
            )
            .is_err()
        );
        let key = crypto::get_or_create_master_key(&r, None).unwrap();
        let other = crypto::encrypt_value(&key, "some-other-secret").unwrap();
        assert!(
            execute(
                &r,
                EmailRequest::SecretOpen {
                    value: panel_proto::EmailSecret(other)
                }
            )
            .is_err()
        );
        assert!(!format!("{:?}", box_config()).contains("$argon2"));
    }
    #[test]
    fn alma_keeps_argon2id_with_checkpassword_and_a_password_free_userdb() {
        for major in ["9", "10"] {
            let r = runner();
            r.set_file_content(
                "/etc/os-release",
                format!("ID=almalinux\nVERSION_ID={major}\n").as_bytes(),
            );
            r.set_response("doveadm pw -l", 0, b"SHA512-CRYPT", b"");
            assert_eq!(install(&r, config()).unwrap()["configured"], true);
            let mailbox = box_config();
            let original_hash = mailbox.password_hash.0.clone();
            execute(&r, EmailRequest::MailboxApply { mailbox }).unwrap();
            let configuration = contents(&r, "/etc/dovecot/dovecot.conf");
            assert!(configuration.contains("driver = checkpassword"));
            assert!(configuration.contains("args = /usr/local/libexec/qonpanel-mail-auth"));
            assert!(configuration.contains("args = /etc/dovecot/qpanel-userdb"));
            let postfix = contents(&r, "/etc/postfix/main.cf");
            assert!(
                postfix.contains("virtual_mailbox_domains = lmdb:/etc/postfix/virtual_domains")
            );
            if major == "9" {
                assert!(postfix.contains("compatibility_level = 2\n"));
                assert!(postfix.contains("smtpd_tls_protocols = !SSLv2,!SSLv3,!TLSv1,!TLSv1.1"));
            } else {
                assert!(postfix.contains("compatibility_level = 3.6\n"));
                assert!(postfix.contains("smtpd_tls_protocols = >=TLSv1.2"));
            }
            assert!(contents(&r, "/etc/dovecot/qpanel-users").contains(&original_hash));
            let userdb = contents(&r, "/etc/dovecot/qpanel-userdb");
            assert!(userdb.starts_with("admin@example.com::10001:10001:"));
            assert!(!userdb.contains("argon2"));
            let commands = r.get_commands();
            assert!(
                commands
                    .iter()
                    .any(|value| value.starts_with("dnf -y") && value.contains("postfix-lmdb"))
            );
            assert!(
                commands
                    .iter()
                    .any(|value| value.starts_with("dnf -y") && value.contains("libsodium"))
            );
            assert!(
                commands.contains(&"/usr/local/libexec/qonpanel-mail-auth --check".to_string())
            );
            assert!(!commands.iter().any(|value| value.starts_with("apt")));
            assert!(!r.path_exists(Path::new("/etc/panel/mail/package-masks.json")));
        }
    }
    #[test]
    fn alma_verifier_failure_cannot_enable_unusable_mail_authentication() {
        let r = runner();
        r.set_file_content("/etc/os-release", b"ID=almalinux\nVERSION_ID=9\n");
        r.set_response(
            "/usr/local/libexec/qonpanel-mail-auth",
            111,
            b"",
            b"fixture verifier unavailable",
        );
        assert!(install(&r, config()).is_err());
        assert!(!load(&r).unwrap().ready);
        assert!(
            !r.get_commands()
                .iter()
                .any(|value| value == "systemctl enable dovecot")
        );
    }
    // Mock generation plus native syntax tools in the disposable package CI.
    // No RealSystemRunner, panel daemon, or mail service is started.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires explicitly disposable Linux package CI"]
    fn native_email_configuration_syntax() {
        use std::{fs, os::unix::fs::PermissionsExt, process::Command};
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert_eq!(std::env::var("QONPANEL_DISPOSABLE_CI").as_deref(), Ok("1"));
        assert!(Path::new("/.dockerenv").is_file() || Path::new("/run/.containerenv").is_file());
        let metadata = fs::read("/etc/os-release").unwrap();
        let os = panel_core::platform::LinuxPlatform::parse_os_release(
            std::str::from_utf8(&metadata).unwrap(),
        )
        .unwrap();
        let r = runner();
        r.set_file_content("/etc/os-release", &metadata);
        install(&r, config()).unwrap();
        execute(
            &r,
            EmailRequest::MailboxApply {
                mailbox: box_config(),
            },
        )
        .unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let postfix = temporary.path().join("postfix");
        let dovecot = temporary.path().join("dovecot");
        let certificate = temporary.path().join("certificate");
        for path in [&postfix, &dovecot, &certificate] {
            fs::create_dir(path).unwrap();
        }
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["mail.example.com".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        fs::write(certificate.join("fullchain.pem"), cert.pem()).unwrap();
        fs::write(certificate.join("privkey.pem"), key.serialize_pem()).unwrap();
        fs::set_permissions(
            certificate.join("privkey.pem"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        for file in [
            "main.cf",
            "master.cf",
            "virtual_domains",
            "virtual_mailboxes",
            "sender_login",
        ] {
            let rendered = contents(&r, &format!("/etc/postfix/{file}"))
                .replace("/etc/postfix/", &format!("{root}/postfix/"))
                .replace(
                    "/etc/panel/ssl/mail.example.com",
                    &format!("{root}/certificate"),
                );
            fs::write(postfix.join(file), rendered).unwrap();
        }
        for file in ["qpanel-users", "qpanel-userdb"] {
            if r.path_exists(Path::new(&format!("/etc/dovecot/{file}"))) {
                fs::write(
                    dovecot.join(file),
                    contents(&r, &format!("/etc/dovecot/{file}")),
                )
                .unwrap();
                fs::set_permissions(dovecot.join(file), fs::Permissions::from_mode(0o640)).unwrap();
            }
        }
        let configuration = contents(&r, "/etc/dovecot/dovecot.conf")
            .replace("/etc/dovecot/", &format!("{root}/dovecot/"))
            .replace(
                "/etc/panel/ssl/mail.example.com",
                &format!("{root}/certificate"),
            );
        fs::write(dovecot.join("dovecot.conf"), configuration).unwrap();
        let check = |binary: &str, args: &[&str]| {
            let output = panel_core::production::system_command(binary)
                .args(args)
                .env("LC_ALL", "C")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{binary} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        let directory = postfix.to_str().unwrap();
        let kind = if os.is_rpm() { "lmdb" } else { "hash" };
        for table in ["virtual_domains", "virtual_mailboxes", "sender_login"] {
            check(
                "postmap",
                &["-c", directory, &format!("{kind}:{directory}/{table}")],
            );
        }
        check("postfix", &["-c", directory, "check"]);
        check(
            "doveconf",
            &["-c", dovecot.join("dovecot.conf").to_str().unwrap(), "-n"],
        );
        let reply = check(
            "postmap",
            &[
                "-c",
                directory,
                "-q",
                "admin@example.com",
                &format!("{kind}:{directory}/virtual_mailboxes"),
            ],
        );
        assert_eq!(String::from_utf8_lossy(&reply.stdout).trim(), "OK");
        if os.is_rpm() {
            let script =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/tests/mail-auth-test.py");
            let output = Command::new("python3")
                .arg(script)
                .env("QONPANEL_TEST_SODIUM", "1")
                .env("QONPANEL_GENERATED_PASSDB", dovecot.join("qpanel-users"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Native Argon2id bridge failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    #[test]
    fn installation_refuses_unmanaged_configs_and_restores_autostart_policy_on_failure() {
        let r = runner();
        r.set_file_content("/etc/postfix/main.cf", b"existing");
        assert!(install(&r, config()).is_err());
        assert_eq!(contents(&r, "/etc/postfix/main.cf"), "existing");
        let r = runner();
        r.set_response("apt-get", 1, b"", b"fixture package failure");
        assert!(install(&r, config()).is_err());
        assert!(!r.path_exists(Path::new("/usr/sbin/policy-rc.d")));
        assert!(!load(&r).unwrap().ready);
        let r = runner();
        r.set_response("doveadm pw -l", 0, b"PLAIN", b"");
        assert!(install(&r, config()).is_err());
        for service in SERVICES {
            assert!(r.recorded_actions().iter().any(|a|matches!(a,MockAction::Command{binary,args} if binary=="systemctl" && args==&vec!["disable".to_string(),service.to_string()])));
        }
    }
    #[test]
    fn fresh_install_and_mailbox_config_require_tls_auth_and_recipient_maps() {
        let r = runner();
        assert_eq!(install(&r, config()).unwrap()["configured"], true);
        let m = box_config();
        execute(&r, EmailRequest::MailboxApply { mailbox: m.clone() }).unwrap();
        let cf = contents(&r, "/etc/postfix/main.cf");
        assert!(cf.contains("permit_sasl_authenticated, reject_unauth_destination"));
        assert!(!cf.contains("permit_mynetworks"));
        assert!(cf.contains("authorized_submit_users = root, panel-api"));
        assert!(cf.contains("smtpd_milters = unix:/var/spool/postfix/qpanel-dkim/opendkim.sock"));
        let dkim = contents(&r, "/etc/opendkim.conf");
        assert!(!dkim.contains("inet:"));
        assert!(dkim.contains("MTA csl:QpanelLocal"));
        assert!(contents(&r, "/etc/opendkim/TrustedHosts").is_empty());
        let master = contents(&r, "/etc/postfix/master.cf");
        assert!(master.contains("smtpd_tls_security_level=encrypt"));
        assert!(master.contains("reject_authenticated_sender_login_mismatch"));
        assert!(master.contains("milter_macro_daemon_name=QpanelInbound"));
        assert!(r.recorded_actions().iter().any(|a|matches!(a,MockAction::Command{binary,args} if binary=="setfacl" && args.iter().any(|v|v.contains("d:g:postfix:rwx") && v.contains("d:o::---")))));
        let dovecot = contents(&r, "/etc/dovecot/dovecot.conf");
        assert!(dovecot.contains("ssl = required"));
        assert!(dovecot.contains("port = 993"));
        let pass = contents(&r, "/etc/dovecot/qpanel-users");
        assert!(pass.contains("{ARGON2ID}$argon2id"));
        assert!(pass.contains(":10001:10001:"));
        assert!(pass.contains("storage=1024M"));
        assert!(!pass.contains("SafeFixture"));
        let actions = r.recorded_actions();
        assert!(!actions.iter().any(|a|matches!(a,MockAction::CreateDir{path,..} if path.to_string_lossy().ends_with("Maildir"))));
        assert_eq!(
            execute(
                &r,
                EmailRequest::DnsRecords {
                    domain: "example.com".into()
                }
            )
            .unwrap()["records"][2]["content"],
            "v=DKIM1; k=rsa; p=PUBLICKEY"
        );
        execute(&r, EmailRequest::MailboxRemove { id: ID.into() }).unwrap();
        assert!(r.recorded_actions().iter().any(|a|matches!(a,MockAction::SetOwnership{path,uid:0,gid:0} if path.to_string_lossy().contains(ID))));
        assert!(
            !r.recorded_actions()
                .iter()
                .any(|a| matches!(a, MockAction::RemoveDir { .. }))
        );
    }
    #[test]
    fn rejects_cross_identity_injection_and_rolls_back_invalid_config() {
        let r = runner();
        ready(&r);
        let mut m = box_config();
        m.uid = 10002;
        assert!(execute(&r, EmailRequest::MailboxApply { mailbox: m }).is_err());
        let mut m = box_config();
        m.password_hash.0.push_str(":0:0");
        assert!(execute(&r, EmailRequest::MailboxApply { mailbox: m }).is_err());
        let mut m = box_config();
        m.id = "../../etc".into();
        assert!(execute(&r, EmailRequest::MailboxApply { mailbox: m }).is_err());
        r.set_file_content("/etc/postfix/main.cf", b"# previous panel config");
        r.set_response("doveconf", 1, b"", b"fixture validation failure");
        assert!(
            execute(
                &r,
                EmailRequest::MailboxApply {
                    mailbox: box_config()
                }
            )
            .is_err()
        );
        assert_eq!(
            contents(&r, "/etc/postfix/main.cf"),
            "# previous panel config"
        );
        assert!(load(&r).unwrap().mailboxes.is_empty());
    }
    #[test]
    fn suspension_revokes_virtual_auth_and_delete_retains_root_only_archive() {
        let r = runner();
        ready(&r);
        execute(
            &r,
            EmailRequest::MailboxApply {
                mailbox: box_config(),
            },
        )
        .unwrap();
        tenant_access(&r, "p_customer", 10001, true, false).unwrap();
        assert!(contents(&r, "/etc/dovecot/qpanel-users").is_empty());
        tenant_access(&r, "p_customer", 10001, false, false).unwrap();
        assert!(contents(&r, "/etc/dovecot/qpanel-users").contains("admin@example.com"));
        tenant_access(&r, "p_customer", 10001, true, true).unwrap();
        assert!(load(&r).unwrap().mailboxes.is_empty());
        assert!(r.recorded_actions().iter().any(|a|matches!(a,MockAction::SetOwnership{path,uid:0,gid:0} if path.to_string_lossy().contains(ID))));
        assert!(
            !r.recorded_actions()
                .iter()
                .any(|a| matches!(a, MockAction::RemoveDir { .. }))
        );
    }
    #[test]
    fn inbox_is_bounded_and_stale_uid_validity_cannot_move_messages() {
        let r = runner();
        ready(&r);
        execute(
            &r,
            EmailRequest::MailboxApply {
                mailbox: box_config(),
            },
        )
        .unwrap();
        r.set_response("doveadm -f json mailbox status -u admin@example.com messages unseen uidnext uidvalidity INBOX",0,br#"[{"messages":"2","unseen":"1","uidnext":"100","uidvalidity":"42"}]"#,b"");
        r.set_response("doveadm -f json fetch -u admin@example.com uid flags hdr.from hdr.to hdr.subject date.received.unixtime size.physical mailbox INBOX uid 50:99",0,br#"[{"uid":"98","hdr.subject":"Hello"}]"#,b"");
        let result = execute(
            &r,
            EmailRequest::Messages {
                id: ID.into(),
                folder: EmailFolder::Inbox,
                before: None,
            },
        )
        .unwrap();
        assert_eq!(result["before"], 50);
        assert_eq!(result["uid_validity"], 42);
        assert_eq!(result["items"][0]["uid"], "98");
        assert!(
            execute(
                &r,
                EmailRequest::MessageTrash {
                    id: ID.into(),
                    folder: EmailFolder::Inbox,
                    uid: 98,
                    uid_validity: 41
                }
            )
            .is_err()
        );
        assert!(!r.recorded_actions().iter().any(
            |a| matches!(a,MockAction::Command{args,..} if args.first().is_some_and(|v|v=="move"))
        ));
    }
    #[test]
    fn accepted_send_is_not_failed_if_sent_copy_fails() {
        let r = runner();
        ready(&r);
        execute(
            &r,
            EmailRequest::MailboxApply {
                mailbox: box_config(),
            },
        )
        .unwrap();
        r.set_response(
            "doveadm save -u admin@example.com -m Sent",
            1,
            b"",
            b"fixture sent quota",
        );
        let result = execute(
            &r,
            EmailRequest::Send {
                id: ID.into(),
                to: "recipient@example.net".into(),
                subject: "Hello".into(),
                body: "Private body".into(),
            },
        )
        .unwrap();
        assert_eq!(result["accepted"], true);
        assert_eq!(result["saved_to_sent"], false);
        let actions = format!("{:?}", r.recorded_actions());
        assert!(!actions.contains("Private body"));
    }
}
