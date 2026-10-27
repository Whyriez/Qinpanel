#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Render real templates without starting Nginx, an agent, or application services.
use panel_webserver::{NginxTemplateRenderer, WebserverError};
use serde_json::{Value, json};

fn context() -> Value {
    json!({
        "site_id":"site_options", "primary_domain":"options.example.test",
        "alias_domains":[], "redirects":[],
        "docroot":"/home/p_customer/sites/options.example.test/public",
        "log_dir":"/var/log/panel/p_customer/options.example.test",
        "snippets_dir":"/etc/panel/nginx/snippets",
        "has_ssl":false, "force_https":false, "ocsp_stapling":false,
        "custom_headers":[], "spa_fallback":false,
        "php_version":"8.3", "fpm_socket":"/run/panel/php/p_customer-8.3.sock",
        "preset":"generic", "upstream_url":"http://127.0.0.1:20001",
        "static_assets_enabled":false, "unix_socket":"/run/panel/python/site_options.sock",
        "preserve_host":true, "websocket_support":true, "custom_request_headers":[],
        "proxy_ssl_server_name":false, "allow_external_upstream":false
    })
}

fn render(kind: &str, context: Value) -> Result<String, WebserverError> {
    let renderer = NginxTemplateRenderer::new();
    match kind {
        "nodejs" => renderer.render_nodejs(&serde_json::from_value(context).unwrap()),
        "python" => renderer.render_python(&serde_json::from_value(context).unwrap()),
        "proxy" => renderer.render_proxy(&serde_json::from_value(context).unwrap()),
        "php" => renderer.render_php(&serde_json::from_value(context).unwrap()),
        "static" => renderer.render_static(&serde_json::from_value(context).unwrap()),
        _ => panic!("Unknown fixture kind"),
    }
}

fn assert_defaults(kind: &str) {
    for null in [false, true] {
        let mut input = context();
        if null {
            for field in [
                "upload_max_size",
                "proxy_connect_timeout",
                "proxy_send_timeout",
                "proxy_read_timeout",
                "proxy_buffering",
            ] {
                input[field] = Value::Null;
            }
        }
        let config = render(kind, input).unwrap();
        if kind != "proxy" {
            assert!(
                config.contains("client_max_body_size 100M;"),
                "{kind} null={null}: expected the upload default, not a null literal\n{config}"
            );
        } else {
            assert!(
                !config.contains("client_max_body_size"),
                "standalone proxy retains the inherited body limit"
            );
        }
        for directive in [
            "proxy_connect_timeout 60s;",
            "proxy_send_timeout 60s;",
            "proxy_read_timeout 60s;",
            "proxy_buffering on;",
        ] {
            assert!(
                config.contains(directive),
                "{kind} null={null}: missing {directive}"
            );
        }
        assert!(
            !config
                .lines()
                .any(|line| line.trim_end().to_ascii_lowercase().ends_with(" none;"))
        );
    }
}

#[test]
fn nodejs_without_optional_settings_has_valid_upload_timeout_and_buffering_defaults() {
    assert_defaults("nodejs");
}

#[test]
fn python_without_optional_settings_has_valid_upload_timeout_and_buffering_defaults() {
    assert_defaults("python");
}

#[test]
fn standalone_proxy_uses_valid_timeouts_and_retains_inherited_body_limit() {
    assert_defaults("proxy");
}

#[test]
fn explicit_proxy_settings_and_zero_upload_limit_are_preserved() {
    for kind in ["nodejs", "python", "proxy"] {
        for size in ["0", "1024", "32k", "50M", "2G"] {
            let mut input = context();
            input["upload_max_size"] = json!(size);
            input["proxy_connect_timeout"] = json!("15s");
            input["proxy_send_timeout"] = json!("30s");
            input["proxy_read_timeout"] = json!("120s");
            input["proxy_buffering"] = json!("off");
            let config = render(kind, input).unwrap();
            assert!(
                config.contains(&format!("client_max_body_size {size};")),
                "{kind} {size}"
            );
            for directive in [
                "proxy_connect_timeout 15s;",
                "proxy_send_timeout 30s;",
                "proxy_read_timeout 120s;",
                "proxy_buffering off;",
            ] {
                assert!(config.contains(directive), "{kind}: {directive}");
            }
        }
    }
}

#[test]
fn invalid_upload_sizes_are_rejected_before_config_is_generated() {
    for kind in ["nodejs", "python", "proxy", "php"] {
        for size in [
            "",
            "none",
            "None",
            "null",
            "64MB",
            "64MM",
            "1MG",
            "-1",
            "1.5M",
            "64M; include /tmp/x;",
            "64M\nreturn 200;",
            " 64M",
            "64M ",
            "9223372036854775808",
            "8589934592G",
        ] {
            let mut input = context();
            input["upload_max_size"] = json!(size);
            assert!(
                render(kind, input).is_err(),
                "{kind} accepted invalid upload size {size:?}"
            );
        }
    }
}

#[test]
fn upload_sizes_accept_the_supported_signed_64_bit_boundary() {
    for kind in ["nodejs", "python", "proxy", "php"] {
        for size in ["0", "0M", "9223372036854775807", "8589934591G"] {
            let mut input = context();
            input["upload_max_size"] = json!(size);
            assert!(
                render(kind, input).is_ok(),
                "{kind} rejected valid upload size {size}"
            );
        }
    }
}

#[test]
fn static_and_php_optional_defaults_do_not_render_null_literals() {
    let input = context();
    let static_config = render("static", input.clone()).unwrap();
    assert!(static_config.contains("index index.html index.htm;"));
    let mut php_input = input;
    php_input["preset"] = json!("custom");
    php_input["fastcgi_cache"] = json!({"enabled":true,"zone_name":null,"valid_time":null,"use_stale":true,"custom_bypass_cookies":[]});
    let php_config = render("php", php_input).unwrap();
    for directive in [
        "index index.php index.html index.htm;",
        "client_max_body_size 64M;",
        "/index.php?$args;",
        "fastcgi_cache panel_php_cache;",
        "fastcgi_cache_valid 200 301 302 60m;",
    ] {
        assert!(php_config.contains(directive), "missing {directive}");
    }
}

#[test]
fn distributed_static_and_php_templates_use_the_same_null_defaults() {
    let mut environment = minijinja::Environment::new();
    environment.add_global(
        "qpanel_default_error_page",
        panel_webserver::error_pages::render_default_not_found(),
    );
    environment
        .add_template(
            "static",
            include_str!("../../../templates/nginx/vhost_static.j2"),
        )
        .unwrap();
    environment
        .add_template("php", include_str!("../../../templates/nginx/vhost_php.j2"))
        .unwrap();
    let mut input = context();
    for field in ["index_files", "upload_max_size", "custom_entry_file"] {
        input[field] = Value::Null;
    }
    input["preset"] = json!("custom");
    input["fastcgi_cache"] = json!({"enabled":true,"zone_name":null,"valid_time":null,"use_stale":true,"custom_bypass_cookies":[]});
    let static_config = environment
        .get_template("static")
        .unwrap()
        .render(&input)
        .unwrap();
    let php_config = environment
        .get_template("php")
        .unwrap()
        .render(&input)
        .unwrap();
    assert!(static_config.contains("index index.html index.htm;"));
    for directive in [
        "index index.php index.html index.htm;",
        "client_max_body_size 64M;",
        "/index.php?$args;",
        "fastcgi_cache panel_php_cache;",
        "fastcgi_cache_valid 200 301 302 60m;",
    ] {
        assert!(php_config.contains(directive), "missing {directive}");
    }
}
