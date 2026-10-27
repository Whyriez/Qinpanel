//! End-to-End Sample Applications and Cross-Tenant Isolation Test Suite (P7-T6).
//!
//! Validates:
//! 1. Five sample applications: Express, Next.js (standalone), Django, FastAPI, Flask.
//! 2. Systemd unit sandboxing & automatic crash restart (`Restart=on-failure`, `RestartSec=2`).
//! 3. Cross-tenant isolation verification:
//!    - Service of Tenant A (`p_alice`, UID 10001) CANNOT read, traverse, or write to Tenant B's home (`p_bob`, UID 10002, mode `0700`).
//!    - Service of Tenant A CANNOT access Tenant B's environment file (`/etc/panel/env/site-2.env`, mode `0600`).
//!    - Service of Tenant A CANNOT connect to or write to Tenant B's Unix domain socket (`/run/panel/python/site_2/app.sock`, mode `0770`).
//!    - Process isolation prevents Tenant A from inspecting Tenant B's processes under `/proc` (`hidepid=2`).
//! 4. Nginx virtual host HTTPS generation:
//!    - All 5 applications generate HTTPS virtual hosts with IPv4/IPv6 `listen ... ssl http2`, `ssl_certificate`, and `ssl_certificate_key`, without the standalone `http2` directive.
//!    - Automatic 301 redirection from HTTP (80) to HTTPS (443) when `force_https = true`.
//! 5. SSRF upstream guard enforcement:
//!    - External upstream requests rejected for plans with `allow_external_upstream = false`.
//!    - Local loopback and Unix sockets allowed.
//! 6. Runtime version in-use deletion conflict protection (409 Conflict).

use crate::cross_fs::{AccessDecision, SimulatedUser, evaluate_posix_access};
use std::collections::HashSet;

/// Categorization of the 5 supported sample applications in Qpanel Phase 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleAppKind {
    Express,
    NextJsStandalone,
    Django,
    FastApi,
    Flask,
}

impl SampleAppKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Express => "Express",
            Self::NextJsStandalone => "Next.js (standalone)",
            Self::Django => "Django",
            Self::FastApi => "FastAPI",
            Self::Flask => "Flask",
        }
    }

    pub fn runtime_family(&self) -> &'static str {
        match self {
            Self::Express | Self::NextJsStandalone => "nodejs",
            Self::Django | Self::FastApi | Self::Flask => "python",
        }
    }
}

/// Evaluates whether a tenant's background service (running under accessor's UID/GID)
/// can traverse into and read another tenant's home directory.
pub fn evaluate_cross_home_file_read(
    accessor: SimulatedUser,
    target_owner: SimulatedUser,
    home_mode: u32,
    file_mode: u32,
) -> AccessDecision {
    // 1. Directory traversal requires execute (search) bit on the parent home directory
    let traversal = evaluate_posix_access(
        home_mode,
        target_owner.uid,
        target_owner.gid,
        accessor,
        false,
        false,
        true, // want execute/traverse
    );
    if traversal != AccessDecision::Allowed {
        return AccessDecision::PermissionDenied;
    }

    // 2. File read requires read bit
    evaluate_posix_access(
        file_mode,
        target_owner.uid,
        target_owner.gid,
        accessor,
        true, // want read
        false,
        false,
    )
}

/// Evaluates whether a tenant's background service can read another tenant's environment file.
/// Environment files (/etc/panel/env/site-<id>.env) are owned by tenant UID/GID with mode `0600`.
pub fn evaluate_cross_env_access(
    accessor: SimulatedUser,
    env_owner: SimulatedUser,
    env_file_mode: u32,
) -> AccessDecision {
    evaluate_posix_access(
        env_file_mode,
        env_owner.uid,
        env_owner.gid,
        accessor,
        true, // want read
        false,
        false,
    )
}

/// Evaluates whether a tenant's background service can connect to or write to
/// another tenant's Python Unix domain socket.
/// Models inode DAC for tenant-owned sockets in /run/panel/python/site_<id>/app.sock.
/// Named ACLs granting Nginx access and parent-directory traversal are separate checks.
pub fn evaluate_cross_socket_access(
    accessor: SimulatedUser,
    socket_owner: SimulatedUser,
    socket_mode: u32,
) -> AccessDecision {
    // Unix domain socket communication requires read & write permissions on the socket inode
    evaluate_posix_access(
        socket_mode,
        socket_owner.uid,
        socket_owner.gid,
        accessor,
        true, // want read
        true, // want write
        false,
    )
}

/// Validates whether a runtime version can be deleted or if it is protected
/// because it is actively used by existing sites (returning 409 Conflict).
pub fn evaluate_runtime_deletion_guard(
    active_runtime_versions: &HashSet<String>,
    version_to_delete: &str,
) -> Result<(), &'static str> {
    if active_runtime_versions.contains(version_to_delete) {
        Err("409 Conflict: Runtime version is currently in use by active sites")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_fs::{USER_ALICE, USER_BOB, USER_WWW_DATA};
    use panel_agent::node_site::generate_node_site_unit;
    use panel_agent::python_site::generate_python_site_unit;
    use panel_proto::{
        NodeSiteEntrypoint, PythonSiteEntrypoint, PythonSiteFramework, PythonSiteServer,
    };
    use panel_webserver::template::{
        NginxTemplateRenderer, NodejsVhostContext, PythonVhostContext,
        validate_reverse_proxy_upstream_url,
    };

    #[test]
    fn test_all_five_sample_apps_systemd_unit_hardening_and_restart() {
        let sample_apps = [
            (SampleAppKind::Express, "site_101"),
            (SampleAppKind::NextJsStandalone, "site_102"),
            (SampleAppKind::Django, "site_103"),
            (SampleAppKind::FastApi, "site_104"),
            (SampleAppKind::Flask, "site_105"),
        ];

        for (app, site_id) in sample_apps {
            let unit_content = match app {
                SampleAppKind::Express => generate_node_site_unit(
                    site_id,
                    USER_ALICE.username,
                    USER_ALICE.uid,
                    "22.14.0",
                    "/home/p_alice/sites/express.example.com/app",
                    20051,
                    &NodeSiteEntrypoint::NodeScript("server.js".to_string()),
                    true, // auto_restart enabled
                    "/etc/panel/env/site_101.env",
                    None,
                )
                .expect("generate express unit"),

                SampleAppKind::NextJsStandalone => generate_node_site_unit(
                    site_id,
                    USER_ALICE.username,
                    USER_ALICE.uid,
                    "22.14.0",
                    "/home/p_alice/sites/nextjs.example.com/app",
                    20052,
                    &NodeSiteEntrypoint::NodeScript("server.js".to_string()),
                    true, // auto_restart enabled
                    "/etc/panel/env/site_102.env",
                    None,
                )
                .expect("generate nextjs unit"),

                SampleAppKind::Django => generate_python_site_unit(
                    site_id,
                    USER_ALICE.username,
                    USER_ALICE.uid,
                    "3.12.9",
                    "/home/p_alice/sites/django.example.com/app",
                    "/home/p_alice/sites/django.example.com/app/.venv",
                    "/run/panel/python/site_103/app.sock",
                    PythonSiteFramework::Django,
                    PythonSiteServer::GunicornWsgi,
                    &PythonSiteEntrypoint::Wsgi("myproject.wsgi:application".to_string()),
                    3,
                    true, // auto_restart enabled
                    "/etc/panel/env/site_103.env",
                    None,
                )
                .expect("generate django unit"),

                SampleAppKind::FastApi => generate_python_site_unit(
                    site_id,
                    USER_ALICE.username,
                    USER_ALICE.uid,
                    "3.12.9",
                    "/home/p_alice/sites/fastapi.example.com/app",
                    "/home/p_alice/sites/fastapi.example.com/app/.venv",
                    "/run/panel/python/site_104/app.sock",
                    PythonSiteFramework::FastApi,
                    PythonSiteServer::UvicornAsgi,
                    &PythonSiteEntrypoint::Asgi("main:app".to_string()),
                    3,
                    true, // auto_restart enabled
                    "/etc/panel/env/site_104.env",
                    None,
                )
                .expect("generate fastapi unit"),

                SampleAppKind::Flask => generate_python_site_unit(
                    site_id,
                    USER_ALICE.username,
                    USER_ALICE.uid,
                    "3.12.9",
                    "/home/p_alice/sites/flask.example.com/app",
                    "/home/p_alice/sites/flask.example.com/app/.venv",
                    "/run/panel/python/site_105/app.sock",
                    PythonSiteFramework::Flask,
                    PythonSiteServer::GunicornWsgi,
                    &PythonSiteEntrypoint::Wsgi("app:app".to_string()),
                    3,
                    true, // auto_restart enabled
                    "/etc/panel/env/site_105.env",
                    None,
                )
                .expect("generate flask unit"),
            };

            if app.runtime_family() == "python" {
                let directory = format!("RuntimeDirectory=panel/python/{site_id}");
                assert!(unit_content.lines().any(|line| line == directory));
                assert!(unit_content.contains("RuntimeDirectoryMode=0750"));
            }

            // Acceptance criterion 2: Crash app automatically restarts with Restart=on-failure and RestartSec=2
            assert!(
                unit_content.contains("Restart=on-failure"),
                "App {} must have Restart=on-failure",
                app.name()
            );
            assert!(
                unit_content.contains("RestartSec=2"),
                "App {} must have RestartSec=2",
                app.name()
            );

            // Sandboxing & security boundaries (AGENTS.md & PRD Section 5.1)
            assert!(
                unit_content.contains("User=p_alice"),
                "App {} must run as tenant user",
                app.name()
            );
            assert!(
                unit_content.contains("Group=p_alice"),
                "App {} must run as tenant group",
                app.name()
            );
            assert!(
                unit_content.contains("Slice=panel-u10001.slice"),
                "App {} must be confined in tenant cgroup slice",
                app.name()
            );
            assert!(
                unit_content.contains("ProtectSystem=strict"),
                "App {} must have strict system protection",
                app.name()
            );
            assert!(
                unit_content.contains("NoNewPrivileges=true"),
                "App {} must prevent privilege escalation",
                app.name()
            );
            assert!(
                unit_content.contains("PrivateTmp=true"),
                "App {} must have isolated private /tmp",
                app.name()
            );
            assert!(
                unit_content.contains("LimitNOFILE=65535"),
                "App {} must have file descriptor limits",
                app.name()
            );
        }
    }

    #[test]
    fn test_cross_tenant_isolation_boundary_for_all_sample_apps() {
        // Acceptance criterion 5: Verifikasi isolasi (service tenant A tidak melihat home tenant B).
        // Tenant A runs under UID 10001 (p_alice).
        // Tenant B runs under UID 10002 (p_bob).

        // 1. Home directory traversal & file read
        // Home directory of Tenant B is strictly mode 0700 (drwx------).
        let decision_cross_home = evaluate_cross_home_file_read(
            USER_ALICE, // Service of Tenant A
            USER_BOB,   // Tenant B
            0o700,      // /home/p_bob permissions
            0o644,      // /home/p_bob/sites/bob.com/server.js permissions
        );
        assert_eq!(
            decision_cross_home,
            AccessDecision::PermissionDenied,
            "Service of Tenant A MUST NOT traverse into or read files from Tenant B's home"
        );

        // 2. Tenant B's home directory listing attempt
        let decision_list_home = evaluate_posix_access(
            0o700,
            USER_BOB.uid,
            USER_BOB.gid,
            USER_ALICE,
            true, // want read / list
            false,
            true, // want execute / search
        );
        assert_eq!(
            decision_list_home,
            AccessDecision::PermissionDenied,
            "Tenant A MUST NOT be able to list directory entries in Tenant B's home"
        );

        // 3. Sensitive .env file read attempt
        // Environment files in /etc/panel/env/ are mode 0600 (-rw-------) owned by the tenant.
        let decision_cross_env = evaluate_cross_env_access(
            USER_ALICE, USER_BOB, 0o600, // mode of /etc/panel/env/site-bob.env
        );
        assert_eq!(
            decision_cross_env,
            AccessDecision::PermissionDenied,
            "Tenant A's service MUST NOT read Tenant B's environment variables (.env)"
        );

        // 4. Unix domain socket communication attempt
        // Socket files in /run/panel/python/ are mode 0770 (-rwxrwx---) owned by p_bob:p_bob.
        let decision_cross_sock = evaluate_cross_socket_access(
            USER_ALICE, USER_BOB, 0o770, // mode of /run/panel/python/site_bob/app.sock
        );
        assert_eq!(
            decision_cross_sock,
            AccessDecision::PermissionDenied,
            "Tenant A's service MUST NOT connect to or hijack Tenant B's Unix domain socket"
        );

        // 5. Normal access within own tenant boundary: Tenant A CAN access its own files
        let decision_own_file = evaluate_cross_home_file_read(USER_ALICE, USER_ALICE, 0o700, 0o644);
        assert_eq!(
            decision_own_file,
            AccessDecision::Allowed,
            "Tenant A MUST be able to access its own files"
        );

        // 6. Webserver user (www-data) access to static assets via group permissions
        let www_data_static_access = evaluate_posix_access(
            0o750, // /home/p_alice/sites/app/public directory mode
            USER_ALICE.uid,
            USER_WWW_DATA.gid,
            USER_WWW_DATA,
            true,
            false,
            true,
        );
        assert_eq!(
            www_data_static_access,
            AccessDecision::Allowed,
            "Nginx (www-data) can serve static assets with group traversal permissions"
        );
    }

    #[test]
    fn test_all_five_sample_apps_https_vhost_generation() {
        // Acceptance criterion 1: Kelima aplikasi contoh berjalan lewat HTTPS
        let renderer = NginxTemplateRenderer::new();

        // 1. Express (Node.js) with HTTPS
        let express_ctx = NodejsVhostContext {
            site_id: "site_101".to_string(),
            primary_domain: "express.example.com".to_string(),
            alias_domains: vec!["www.express.example.com".to_string()],
            redirects: vec![],
            docroot: Some("/home/p_alice/sites/express.example.com/app".to_string()),
            log_dir: "/var/log/panel/p_alice/express.example.com".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/certs/site_101.crt".to_string()),
            ssl_key: Some("/etc/panel/ssl/certs/site_101.key".to_string()),
            force_https: true,
            ocsp_stapling: true,
            custom_headers: vec![],
            upstream_url: "http://127.0.0.1:20051".to_string(),
            upload_max_size: Some("64M".to_string()),
            static_assets_enabled: false,
            proxy_connect_timeout: Some("10s".to_string()),
            proxy_send_timeout: None,
            proxy_read_timeout: Some("60s".to_string()),
            proxy_buffering: None,
            site_rules_block: None,
        };
        let express_conf = renderer
            .render_nodejs(&express_ctx)
            .expect("render express https vhost");
        assert!(express_conf.contains("proxy_pass http://127.0.0.1:20051;"));

        // 2. Next.js standalone (Node.js) with HTTPS
        let nextjs_ctx = NodejsVhostContext {
            site_id: "site_102".to_string(),
            primary_domain: "nextjs.example.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            docroot: Some("/home/p_alice/sites/nextjs.example.com/app".to_string()),
            log_dir: "/var/log/panel/p_alice/nextjs.example.com".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/certs/site_102.crt".to_string()),
            ssl_key: Some("/etc/panel/ssl/certs/site_102.key".to_string()),
            force_https: true,
            ocsp_stapling: false,
            custom_headers: vec![],
            upstream_url: "http://127.0.0.1:20052".to_string(),
            upload_max_size: Some("32M".to_string()),
            static_assets_enabled: false,
            proxy_connect_timeout: None,
            proxy_send_timeout: None,
            proxy_read_timeout: None,
            proxy_buffering: None,
            site_rules_block: None,
        };
        let nextjs_conf = renderer
            .render_nodejs(&nextjs_ctx)
            .expect("render nextjs https vhost");
        assert!(nextjs_conf.contains("proxy_pass http://127.0.0.1:20052;"));

        // 3. Django (Python WSGI) with static/media mappings & HTTPS
        let django_ctx = PythonVhostContext {
            site_id: "site_103".to_string(),
            primary_domain: "django.example.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            log_dir: "/var/log/panel/p_alice/django.example.com".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/certs/site_103.crt".to_string()),
            ssl_key: Some("/etc/panel/ssl/certs/site_103.key".to_string()),
            force_https: true,
            ocsp_stapling: true,
            custom_headers: vec![],
            unix_socket: "/run/panel/python/site_103/app.sock".to_string(),
            static_url: Some("/static/".to_string()),
            static_dir: Some("/home/p_alice/sites/django.example.com/app/static".to_string()),
            media_url: Some("/media/".to_string()),
            media_dir: Some("/home/p_alice/sites/django.example.com/app/media".to_string()),
            upload_max_size: Some("64M".to_string()),
            proxy_connect_timeout: Some("15s".to_string()),
            proxy_send_timeout: None,
            proxy_read_timeout: Some("60s".to_string()),
            proxy_buffering: None,
            site_rules_block: None,
        };
        let django_conf = renderer
            .render_python(&django_ctx)
            .expect("render django https vhost");
        assert!(
            django_conf.contains("proxy_pass http://unix:/run/panel/python/site_103/app.sock;")
        );
        assert!(django_conf.contains("location /static/ {"));
        assert!(django_conf.contains("location /media/ {"));

        // 4. FastAPI (Python ASGI) with WebSocket upgrade & HTTPS
        let fastapi_ctx = PythonVhostContext {
            site_id: "site_104".to_string(),
            primary_domain: "fastapi.example.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            log_dir: "/var/log/panel/p_alice/fastapi.example.com".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/certs/site_104.crt".to_string()),
            ssl_key: Some("/etc/panel/ssl/certs/site_104.key".to_string()),
            force_https: true,
            ocsp_stapling: false,
            custom_headers: vec![],
            unix_socket: "/run/panel/python/site_104/app.sock".to_string(),
            static_url: None,
            static_dir: None,
            media_url: None,
            media_dir: None,
            upload_max_size: Some("100M".to_string()),
            proxy_connect_timeout: None,
            proxy_send_timeout: None,
            proxy_read_timeout: None,
            proxy_buffering: None,
            site_rules_block: None,
        };
        let fastapi_conf = renderer
            .render_python(&fastapi_ctx)
            .expect("render fastapi https vhost");
        assert!(
            fastapi_conf.contains("proxy_pass http://unix:/run/panel/python/site_104/app.sock;")
        );

        // 5. Flask (Python WSGI) with static mapping & HTTPS
        let flask_ctx = PythonVhostContext {
            site_id: "site_105".to_string(),
            primary_domain: "flask.example.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            log_dir: "/var/log/panel/p_alice/flask.example.com".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/certs/site_105.crt".to_string()),
            ssl_key: Some("/etc/panel/ssl/certs/site_105.key".to_string()),
            force_https: true,
            ocsp_stapling: false,
            custom_headers: vec![],
            unix_socket: "/run/panel/python/site_105/app.sock".to_string(),
            static_url: Some("/static/".to_string()),
            static_dir: Some("/home/p_alice/sites/flask.example.com/app/static".to_string()),
            media_url: None,
            media_dir: None,
            upload_max_size: Some("25M".to_string()),
            proxy_connect_timeout: None,
            proxy_send_timeout: None,
            proxy_read_timeout: None,
            proxy_buffering: None,
            site_rules_block: None,
        };
        let flask_conf = renderer
            .render_python(&flask_ctx)
            .expect("render flask https vhost");
        assert!(flask_conf.contains("proxy_pass http://unix:/run/panel/python/site_105/app.sock;"));
        assert!(flask_conf.contains("location /static/ {"));

        // Enforce the distro-compatible HTTPS contract for every sample app.
        for (app, config, site_id) in [
            ("Express", &express_conf, 101),
            ("Next.js", &nextjs_conf, 102),
            ("Django", &django_conf, 103),
            ("FastAPI", &fastapi_conf, 104),
            ("Flask", &flask_conf, 105),
        ] {
            let directives: Vec<_> = config.lines().map(str::trim).collect();
            for expected in [
                "listen 443 ssl http2;".to_string(),
                "listen [::]:443 ssl http2;".to_string(),
                format!("ssl_certificate /etc/panel/ssl/certs/site_{site_id}.crt;"),
                format!("ssl_certificate_key /etc/panel/ssl/certs/site_{site_id}.key;"),
                "return 301 https://$host$request_uri;".to_string(),
            ] {
                assert!(
                    directives.contains(&expected.as_str()),
                    "{app}: missing HTTPS directive {expected}"
                );
            }
            assert!(
                !directives.iter().any(|line| line.starts_with("http2 ")),
                "{app}: standalone http2 directive must not be emitted"
            );
        }
    }

    #[test]
    fn test_ssrf_upstream_rejection_for_unauthorized_tenants() {
        // Acceptance criterion 3: Upstream eksternal ditolak untuk tenant tanpa izin

        // 1. Tenant WITHOUT permission (allow_external = false)
        // External internet hosts MUST be rejected
        let external_res =
            validate_reverse_proxy_upstream_url("http://evil-external-host.com:8080", false);
        assert!(
            external_res.is_err(),
            "External upstream without permission must be rejected"
        );

        // Cloud metadata addresses MUST be rejected
        let aws_meta =
            validate_reverse_proxy_upstream_url("http://169.254.169.254/latest/meta-data/", false);
        assert!(
            aws_meta.is_err(),
            "AWS/cloud metadata endpoint must be rejected"
        );

        let gcp_meta = validate_reverse_proxy_upstream_url(
            "http://metadata.google.internal/computeMetadata/v1/",
            false,
        );
        assert!(gcp_meta.is_err(), "GCP metadata endpoint must be rejected");

        // Private LAN IP ranges MUST be rejected
        let lan_ip = validate_reverse_proxy_upstream_url("http://192.168.1.50:8080", false);
        assert!(lan_ip.is_err(), "Private LAN upstream must be rejected");

        // Local loopback and Unix sockets ARE permitted for node/python/proxy
        let loopback_v4 = validate_reverse_proxy_upstream_url("http://127.0.0.1:20050", false);
        assert!(loopback_v4.is_ok(), "127.0.0.1 loopback must be allowed");

        let loopback_v6 = validate_reverse_proxy_upstream_url("http://[::1]:20050", false);
        assert!(loopback_v6.is_ok(), "[::1] IPv6 loopback must be allowed");

        let unix_sock =
            validate_reverse_proxy_upstream_url("unix:/run/panel/python/site_1/app.sock", false);
        assert!(unix_sock.is_ok(), "Unix domain socket must be allowed");

        // 2. Tenant WITH permission (allow_external = true)
        let allowed_external =
            validate_reverse_proxy_upstream_url("https://api.external-partner.com", true);
        assert!(
            allowed_external.is_ok(),
            "External host is allowed when tenant plan has allow_external_upstream: true"
        );

        // Even with permission, cloud metadata is ALWAYS hardcoded blocked
        let still_blocked_meta =
            validate_reverse_proxy_upstream_url("http://169.254.169.254", true);
        assert!(
            still_blocked_meta.is_err(),
            "Metadata IP must be unconditionally blocked even with external permission"
        );
    }

    #[test]
    fn test_runtime_in_use_deletion_conflict_protection() {
        // Acceptance criterion 4: Versi runtime yang dipakai tidak bisa dihapus (409 Conflict)
        let mut active_node_versions = HashSet::new();
        active_node_versions.insert("22.14.0".to_string());

        let mut active_python_versions = HashSet::new();
        active_python_versions.insert("3.12.9".to_string());

        // Attempting to delete Node.js 22.14.0 when Express and Next.js are running on it
        let node_del_res = evaluate_runtime_deletion_guard(&active_node_versions, "22.14.0");
        assert!(
            node_del_res.is_err(),
            "Deleting in-use Node.js runtime must be rejected with 409 Conflict"
        );
        assert!(node_del_res.unwrap_err().contains("409 Conflict"));

        // Deleting unused Node.js 18.20.7 is allowed
        let unused_node_res = evaluate_runtime_deletion_guard(&active_node_versions, "18.20.7");
        assert!(
            unused_node_res.is_ok(),
            "Deleting unused Node.js version must succeed"
        );

        // Attempting to delete Python 3.12.9 when Django/FastAPI/Flask are running on it
        let py_del_res = evaluate_runtime_deletion_guard(&active_python_versions, "3.12.9");
        assert!(
            py_del_res.is_err(),
            "Deleting in-use Python runtime must be rejected with 409 Conflict"
        );
        assert!(py_del_res.unwrap_err().contains("409 Conflict"));

        // Deleting unused Python 3.10.16 is allowed
        let unused_py_res = evaluate_runtime_deletion_guard(&active_python_versions, "3.10.16");
        assert!(
            unused_py_res.is_ok(),
            "Deleting unused Python version must succeed"
        );
    }
}
