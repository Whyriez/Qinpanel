//! Framework compatibility & multi-tenant PHP isolation test suite (P4-T8).
//!
//! Validates:
//! 1. Framework preset structure & sensitive file protection (WordPress, Laravel, Plain PHP).
//! 2. Multi-tenant multi-version concurrency (Tenant A on PHP 8.1, Tenant B on PHP 8.3).
//! 3. Cross-tenant file isolation: Tenant A's PHP worker cannot read Tenant B's files via POSIX DAC and `open_basedir`.
//! 4. Zero idle worker concurrency (`pm = ondemand`).

use crate::cross_fs::{AccessDecision, SimulatedUser, evaluate_posix_access};
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum OpenBaseDirDecision {
    Allowed,
    Restricted(String),
}

/// Evaluates PHP's `open_basedir` sandbox policy against a target path.
pub fn evaluate_open_basedir(
    target_path: &Path,
    allowed_prefixes: &[&Path],
) -> OpenBaseDirDecision {
    let target_components: Vec<_> = target_path.components().collect();

    for prefix in allowed_prefixes {
        let prefix_components: Vec<_> = prefix.components().collect();
        if target_components.len() >= prefix_components.len()
            && target_components[..prefix_components.len()] == prefix_components[..]
        {
            return OpenBaseDirDecision::Allowed;
        }
    }

    OpenBaseDirDecision::Restricted(format!(
        "open_basedir restriction in effect. File ({}) is not within the allowed path(s)",
        target_path.display()
    ))
}

/// Evaluates cross-tenant file read attempt by a PHP-FPM worker.
/// A PHP-FPM worker runs as the tenant's UID/GID.
pub fn evaluate_php_worker_cross_read(
    worker_tenant: SimulatedUser,
    target_owner: SimulatedUser,
    target_home_mode: u32,
    target_file_mode: u32,
    target_file_path: &Path,
    worker_open_basedir: &[&Path],
) -> AccessDecision {
    // 1. Layer 1: OS POSIX DAC on parent home directory traversal
    let home_traversal = evaluate_posix_access(
        target_home_mode,
        target_owner.uid,
        target_owner.gid,
        worker_tenant,
        false,
        false,
        true, // Must have execute/traverse bit
    );
    if home_traversal != AccessDecision::Allowed {
        return AccessDecision::PermissionDenied;
    }

    // 2. Layer 2: OS POSIX DAC on target file
    let file_access = evaluate_posix_access(
        target_file_mode,
        target_owner.uid,
        target_owner.gid,
        worker_tenant,
        true, // want read
        false,
        false,
    );
    if file_access != AccessDecision::Allowed {
        return AccessDecision::PermissionDenied;
    }

    // 3. Layer 3: PHP runtime open_basedir restriction
    match evaluate_open_basedir(target_file_path, worker_open_basedir) {
        OpenBaseDirDecision::Allowed => AccessDecision::Allowed,
        OpenBaseDirDecision::Restricted(_) => AccessDecision::PermissionDenied,
    }
}

/// Evaluates socket connection permission for a simulated user.
pub fn evaluate_fpm_socket_access(
    socket_owner_uid: u32,
    socket_owner_gid: u32,
    socket_mode: u32,
    accessor: SimulatedUser,
) -> AccessDecision {
    // Standard Unix domain socket DAC check: requires read & write
    evaluate_posix_access(
        socket_mode,
        socket_owner_uid,
        socket_owner_gid,
        accessor,
        true,
        true,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_fs::{USER_ALICE, USER_BOB, USER_WWW_DATA};
    use panel_runtime::fpm::{PhpFpmMasterConfig, PhpFpmPoolConfig};
    use panel_runtime::php::PhpVersion;
    use panel_webserver::template::{NginxTemplateRenderer, PhpPreset, PhpVhostContext};

    #[test]
    fn test_wordpress_preset_vhost_and_security_rules() {
        let renderer = NginxTemplateRenderer::new();
        let ctx = PhpVhostContext {
            site_id: "1".to_string(),
            primary_domain: "myblog.com".to_string(),
            alias_domains: vec!["www.myblog.com".to_string()],
            redirects: vec![],
            docroot: "/home/p_alice/sites/myblog.com/public".to_string(),
            log_dir: "/var/log/panel/p_alice/1".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            index_files: Some("index.php index.html".to_string()),
            has_ssl: true,
            ssl_cert: Some("/etc/panel/ssl/myblog.com/fullchain.pem".to_string()),
            ssl_key: Some("/etc/panel/ssl/myblog.com/privkey.pem".to_string()),
            force_https: true,
            ocsp_stapling: false,
            custom_headers: vec![],
            php_version: "8.3".to_string(),
            fpm_socket: "unix:/run/panel/php/p_alice-8.3/fpm.sock".to_string(),
            preset: PhpPreset::Wordpress,
            custom_entry_file: None,
            upload_max_size: Some("64M".to_string()),
            fastcgi_cache: None,
            site_rules_block: None,
        };

        let rendered = renderer.render_php(&ctx).expect("render wordpress vhost");

        // 1. Verify WordPress front controller rewrite rule
        assert!(
            rendered.contains("try_files $uri $uri/ /index.php?$args;"),
            "WordPress template must route permalinks through index.php?$args"
        );

        // 2. Verify blocking wp-config.php and sensitive files
        assert!(
            rendered.contains("location = /wp-config.php") && rendered.contains("deny all;"),
            "WordPress template must explicitly deny access to /wp-config.php"
        );

        // 3. Verify upload directory PHP execution denial rule
        assert!(
            rendered.contains("location ~* ^/wp-content/uploads/.*\\.php$")
                && rendered.contains("deny all;"),
            "WordPress template must deny PHP execution in upload directories"
        );

        // 4. Verify FastCGI socket configuration
        assert!(
            rendered.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.3/fpm.sock;"),
            "Vhost must route FastCGI to tenant's PHP 8.3 socket"
        );
    }

    #[test]
    fn test_laravel_preset_vhost_and_docroot_containment() {
        let renderer = NginxTemplateRenderer::new();
        let ctx = PhpVhostContext {
            site_id: "2".to_string(),
            primary_domain: "laravelapp.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            docroot: "/home/p_bob/sites/laravelapp.com/public".to_string(),
            log_dir: "/var/log/panel/p_bob/2".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            index_files: Some("index.php index.html".to_string()),
            has_ssl: false,
            ssl_cert: None,
            ssl_key: None,
            force_https: false,
            ocsp_stapling: false,
            custom_headers: vec![],
            php_version: "8.2".to_string(),
            fpm_socket: "unix:/run/panel/php/p_bob-8.2/fpm.sock".to_string(),
            preset: PhpPreset::Laravel,
            custom_entry_file: None,
            upload_max_size: Some("32M".to_string()),
            fastcgi_cache: None,
            site_rules_block: None,
        };

        let rendered = renderer.render_php(&ctx).expect("render laravel vhost");

        // 1. Verify docroot points to public/
        assert!(
            rendered.contains("root /home/p_bob/sites/laravelapp.com/public;"),
            "Laravel docroot must be /home/p_bob/sites/laravelapp.com/public"
        );

        // 2. Verify front-controller rewrite
        assert!(
            rendered.contains("try_files $uri $uri/ /index.php?$query_string;"),
            "Laravel template must rewrite requests to /index.php"
        );

        // 3. Verify sensitive environment files are shielded
        assert!(
            rendered.contains("deny-sensitive.conf"),
            "Laravel template must include sensitive file block (.env, storage/logs, etc.)"
        );
    }

    #[test]
    fn test_plain_php_generic_preset() {
        let renderer = NginxTemplateRenderer::new();
        let ctx = PhpVhostContext {
            site_id: "3".to_string(),
            primary_domain: "simple.com".to_string(),
            alias_domains: vec![],
            redirects: vec![],
            docroot: "/home/p_alice/sites/simple.com/public".to_string(),
            log_dir: "/var/log/panel/p_alice/3".to_string(),
            snippets_dir: "/etc/panel/nginx/snippets".to_string(),
            index_files: Some("index.php index.html".to_string()),
            has_ssl: false,
            ssl_cert: None,
            ssl_key: None,
            force_https: false,
            ocsp_stapling: false,
            custom_headers: vec![],
            php_version: "8.1".to_string(),
            fpm_socket: "unix:/run/panel/php/p_alice-8.1/fpm.sock".to_string(),
            preset: PhpPreset::Generic,
            custom_entry_file: None,
            upload_max_size: Some("16M".to_string()),
            fastcgi_cache: None,
            site_rules_block: None,
        };

        let rendered = renderer.render_php(&ctx).expect("render generic vhost");
        assert!(rendered.contains("fastcgi_index index.php;"));
        assert!(rendered.contains("fastcgi_pass unix:/run/panel/php/p_alice-8.1/fpm.sock;"));
    }

    #[test]
    fn test_concurrent_multi_tenant_multi_version_php_fpm() {
        // Tenant A: Alice on PHP 8.1
        let master_alice =
            PhpFpmMasterConfig::new(USER_ALICE.username, USER_ALICE.uid, PhpVersion::Php81)
                .expect("create alice master");

        let alice_pool = PhpFpmPoolConfig::new(
            &master_alice,
            "site_1",
            "/home/p_alice/sites/alice.com/public",
        )
        .expect("create alice pool")
        .with_pm_limits(10, 10, 500);

        // Tenant B: Bob on PHP 8.3
        let master_bob =
            PhpFpmMasterConfig::new(USER_BOB.username, USER_BOB.uid, PhpVersion::Php83)
                .expect("create bob master");

        let bob_pool =
            PhpFpmPoolConfig::new(&master_bob, "site_2", "/home/p_bob/sites/bob.com/public")
                .expect("create bob pool")
                .with_pm_limits(20, 10, 500);

        let alice_conf = alice_pool.render_pool_conf();
        let bob_conf = bob_pool.render_pool_conf();

        // 1. Verify user/group binding is strictly isolated per tenant
        assert!(alice_conf.contains("user = p_alice"));
        assert!(alice_conf.contains("group = p_alice"));
        assert!(bob_conf.contains("user = p_bob"));
        assert!(bob_conf.contains("group = p_bob"));

        // 2. Verify ondemand process manager (zero idle workers)
        assert!(alice_conf.contains("pm = ondemand"));
        assert!(alice_conf.contains("pm.process_idle_timeout = 10s"));
        assert!(alice_conf.contains("pm.max_children = 10"));

        assert!(bob_conf.contains("pm = ondemand"));
        assert!(bob_conf.contains("pm.process_idle_timeout = 10s"));
        assert!(bob_conf.contains("pm.max_children = 20"));

        // 3. Verify distinct socket endpoints
        assert!(alice_conf.contains("listen = /run/panel/php/p_alice-8.1/fpm.sock"));
        assert!(bob_conf.contains("listen = /run/panel/php/p_bob-8.3/fpm.sock"));
        assert_ne!(alice_pool.socket_path, bob_pool.socket_path);
        assert!(alice_conf.contains("listen.acl_users = www-data\n"));
        assert!(bob_conf.contains("listen.acl_users = www-data\n"));

        // 4. Verify systemd unit isolation
        let alice_unit = master_alice.render_systemd_unit();
        let bob_unit = master_bob.render_systemd_unit();
        assert!(alice_unit.contains("User=p_alice"));
        assert!(alice_unit.contains("Slice=panel-u10001.slice"));
        assert!(bob_unit.contains("User=p_bob"));
        assert!(bob_unit.contains("Slice=panel-u10002.slice"));
        assert!(alice_unit.contains("RuntimeDirectory=panel/php/p_alice-8.1"));
        assert!(bob_unit.contains("RuntimeDirectory=panel/php/p_bob-8.3"));

        // 5. Verify socket permissions allow www-data but reject cross-tenant access
        let socket_alice_mode = 0o660;
        let socket_alice_uid = USER_ALICE.uid;
        let socket_alice_gid = USER_WWW_DATA.gid;

        // www-data (Nginx reverse proxy) MUST be allowed
        let www_access = evaluate_fpm_socket_access(
            socket_alice_uid,
            socket_alice_gid,
            socket_alice_mode,
            USER_WWW_DATA,
        );
        assert_eq!(www_access, AccessDecision::Allowed);

        // Alice (owner) MUST be allowed
        let alice_access = evaluate_fpm_socket_access(
            socket_alice_uid,
            socket_alice_gid,
            socket_alice_mode,
            USER_ALICE,
        );
        assert_eq!(alice_access, AccessDecision::Allowed);

        // Bob (Tenant B) MUST BE REJECTED from Alice's socket
        let bob_to_alice_socket = evaluate_fpm_socket_access(
            socket_alice_uid,
            socket_alice_gid,
            socket_alice_mode,
            USER_BOB,
        );
        assert_eq!(bob_to_alice_socket, AccessDecision::PermissionDenied);
    }

    #[test]
    fn test_cross_tenant_file_read_prevention() {
        let alice_wp_config = Path::new("/home/p_alice/sites/alice.com/wp-config.php");
        let alice_env = Path::new("/home/p_alice/sites/alice.com/.env");

        let bob_allowed_basedir = [
            Path::new("/home/p_bob/sites/bob.com"),
            Path::new("/tmp/panel/p_bob/site_2"),
        ];

        // Scenario 1: Alice's home directory has standard mode 0700
        // Bob's PHP worker attempts to read Alice's wp-config.php
        let res_wp = evaluate_php_worker_cross_read(
            USER_BOB,
            USER_ALICE,
            0o700, // Home mode 0700
            0o600, // File mode 0600
            alice_wp_config,
            &bob_allowed_basedir,
        );
        assert_eq!(
            res_wp,
            AccessDecision::PermissionDenied,
            "Bob's PHP worker must NOT be able to read Alice's wp-config.php"
        );

        // Scenario 2: Bob's PHP worker attempts to read Alice's .env file
        let res_env = evaluate_php_worker_cross_read(
            USER_BOB,
            USER_ALICE,
            0o700,
            0o600,
            alice_env,
            &bob_allowed_basedir,
        );
        assert_eq!(
            res_env,
            AccessDecision::PermissionDenied,
            "Bob's PHP worker must NOT be able to read Alice's .env"
        );

        // Scenario 3: Even if Alice's home mode was misconfigured as 0755,
        // PHP's open_basedir barrier MUST catch and reject the read!
        let res_misconfigured_home = evaluate_php_worker_cross_read(
            USER_BOB,
            USER_ALICE,
            0o755, // Misconfigured permissive home
            0o644, // Misconfigured permissive file
            alice_wp_config,
            &bob_allowed_basedir,
        );
        assert_eq!(
            res_misconfigured_home,
            AccessDecision::PermissionDenied,
            "open_basedir must reject access to /home/p_alice even if POSIX mode allows it"
        );

        // Scenario 4: Bob can legitimately read his own files within his open_basedir
        let bob_index = Path::new("/home/p_bob/sites/bob.com/public/index.php");
        let res_bob_own_file = evaluate_php_worker_cross_read(
            USER_BOB,
            USER_BOB,
            0o700,
            0o644,
            bob_index,
            &bob_allowed_basedir,
        );
        assert_eq!(
            res_bob_own_file,
            AccessDecision::Allowed,
            "Bob must be allowed to read his own index.php"
        );
    }

    #[test]
    fn test_open_basedir_enforces_php_admin_value_immutability() {
        let master =
            PhpFpmMasterConfig::new("p_alice", 10001, PhpVersion::Php83).expect("create master");

        let pool = PhpFpmPoolConfig::new(&master, "site_1", "/home/p_alice/sites/alice.com/public")
            .expect("create pool");

        let rendered = pool.render_pool_conf();

        // Must be rendered with php_admin_value, NOT php_value
        // php_admin_value cannot be overridden by ini_set() in customer PHP scripts
        assert!(
            rendered.contains("php_admin_value[open_basedir] ="),
            "open_basedir must be set via php_admin_value to prevent ini_set() bypass"
        );

        // Ensure distinct isolated upload_tmp_dir and session.save_path
        assert!(
            rendered.contains("php_admin_value[upload_tmp_dir] = /home/p_alice/tmp"),
            "upload_tmp_dir must be isolated per tenant home"
        );
        assert!(
            rendered.contains("php_admin_value[session.save_path] = /home/p_alice/tmp"),
            "session.save_path must be isolated per tenant home"
        );
    }
}
