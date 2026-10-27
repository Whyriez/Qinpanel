//! Generator for Nginx configuration directives derived from structured site rules (P8-T3).

use panel_core::site_rules::{
    CorsPreset, HotlinkAction, IpAction, RedirectMatchType, SiteRulesConfig,
};

/// Generates the complete Nginx configuration block for a site's configured rules.
///
/// This block is embedded directly inside the `server { ... }` block of the site's vhost.
pub fn generate_site_rules_nginx_block(
    rules: &SiteRulesConfig,
    site_id: &str,
    htpasswd_path: Option<&str>,
) -> String {
    let mut out = String::new();
    let auth_file = htpasswd_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("/etc/panel/nginx/auth/site_{site_id}.htpasswd"));

    append_maintenance_block(&mut out, rules);
    append_security_and_custom_headers(&mut out, rules);
    append_cors_block(&mut out, rules);
    append_global_ip_access(&mut out, rules);
    append_global_basic_auth(&mut out, rules, &auth_file);
    if !rules
        .custom_error_pages
        .iter()
        .any(|page| page.status_code == 404)
    {
        out.push_str(&crate::error_pages::render_default_not_found());
    }
    append_custom_error_pages(&mut out, rules);
    append_structured_redirects(&mut out, rules);
    append_hotlink_protection(&mut out, rules);
    append_scoped_basic_auth(&mut out, rules, &auth_file);
    append_scoped_ip_access(&mut out, rules);
    append_nginx_snippet(&mut out, rules);

    out
}

fn append_maintenance_block(out: &mut String, rules: &SiteRulesConfig) {
    if !rules.maintenance.enabled {
        return;
    }
    out.push_str("    # --- Maintenance Mode ---\n");
    out.push_str("    set $maintenance_active 1;\n");
    for ip in &rules.maintenance.bypass_ips {
        let clean_ip = ip.trim();
        if !clean_ip.is_empty() {
            out.push_str(&format!(
                "    if ($remote_addr = \"{clean_ip}\") {{ set $maintenance_active 0; }}\n"
            ));
        }
    }
    out.push_str("    if ($maintenance_active = 1) {\n");
    out.push_str("        return 503;\n");
    out.push_str("    }\n");
    let retry_after = rules.maintenance.retry_after_secs;
    out.push_str("    error_page 503 @panel_maintenance_mode;\n");
    out.push_str("    location @panel_maintenance_mode {\n");
    out.push_str(&format!(
        "        add_header Retry-After {retry_after} always;\n"
    ));
    out.push_str("        add_header Content-Type \"text/html; charset=utf-8\" always;\n");
    let escaped_msg = rules
        .maintenance
        .custom_message
        .replace('"', "\\\"")
        .replace('\n', " ");
    out.push_str(&format!(
        "        return 503 \"<!DOCTYPE html><html><head><meta charset='utf-8'><title>Maintenance</title><style>body{{font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#0f172a;color:#f8fafc;}}div{{text-align:center;max-width:600px;padding:2rem;background:#1e293b;border-radius:1rem;border:1px solid #334155;}}h1{{color:#38bdf8;margin-bottom:1rem;}}p{{color:#94a3b8;line-height:1.6;}}</style></head><body><div><h1>Pemeliharaan Situs</h1><p>{escaped_msg}</p></div></body></html>\";\n"
    ));
    out.push_str("    }\n\n");
}

fn append_security_and_custom_headers(out: &mut String, rules: &SiteRulesConfig) {
    let sec = &rules.security_headers;
    let mut header_lines = Vec::new();

    if sec.hsts_enabled {
        let mut hsts_val = "max-age=31536000".to_string();
        if sec.hsts_subdomains {
            hsts_val.push_str("; includeSubDomains");
        }
        if sec.hsts_preload {
            hsts_val.push_str("; preload");
        }
        header_lines.push(format!(
            "    add_header Strict-Transport-Security \"{hsts_val}\" always;"
        ));
    }

    if let Some(ref xfo) = sec.x_frame_options {
        let xfo_upper = xfo.to_uppercase();
        if xfo_upper == "DENY" || xfo_upper == "SAMEORIGIN" {
            header_lines.push(format!(
                "    add_header X-Frame-Options \"{xfo_upper}\" always;"
            ));
        }
    }

    if sec.x_content_type_options {
        header_lines.push("    add_header X-Content-Type-Options \"nosniff\" always;".to_string());
    }

    if let Some(ref ref_pol) = sec.referrer_policy {
        let clean_pol = ref_pol.trim();
        if !clean_pol.is_empty() {
            header_lines.push(format!(
                "    add_header Referrer-Policy \"{clean_pol}\" always;"
            ));
        }
    }

    if let Some(ref perm_pol) = sec.permissions_policy {
        let clean_perm = perm_pol.trim();
        if !clean_perm.is_empty() {
            header_lines.push(format!(
                "    add_header Permissions-Policy \"{clean_perm}\" always;"
            ));
        }
    }

    if let Some(ref csp) = sec.csp {
        let clean_csp = csp.trim().replace('"', "\\\"");
        if !clean_csp.is_empty() {
            header_lines.push(format!(
                "    add_header Content-Security-Policy \"{clean_csp}\" always;"
            ));
        }
    }

    for h in &rules.custom_headers {
        if h.enabled {
            let h_val = h.value.replace('"', "\\\"");
            let always_flag = if h.always { " always" } else { "" };
            header_lines.push(format!(
                "    add_header {} \"{}\"{};",
                h.name, h_val, always_flag
            ));
        }
    }

    if !header_lines.is_empty() {
        out.push_str("    # --- Security & Custom Response Headers ---\n");
        for line in header_lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }
}

fn append_cors_block(out: &mut String, rules: &SiteRulesConfig) {
    if !rules.cors.enabled {
        return;
    }
    out.push_str("    # --- Cross-Origin Resource Sharing (CORS) ---\n");
    let methods_str = if rules.cors.allowed_methods.is_empty() {
        "GET, POST, OPTIONS, PUT, DELETE, PATCH".to_string()
    } else {
        rules.cors.allowed_methods.join(", ")
    };
    let headers_str = if rules.cors.allowed_headers.is_empty() {
        "DNT,User-Agent,X-Requested-With,If-Modified-Since,Cache-Control,Content-Type,Range,Authorization"
            .to_string()
    } else {
        rules.cors.allowed_headers.join(", ")
    };
    let max_age = rules.cors.max_age_secs;

    match rules.cors.preset {
        CorsPreset::AllowAll => {
            out.push_str("    add_header 'Access-Control-Allow-Origin' '*' always;\n");
            out.push_str(&format!(
                "    add_header 'Access-Control-Allow-Methods' '{methods_str}' always;\n"
            ));
            out.push_str(&format!(
                "    add_header 'Access-Control-Allow-Headers' '{headers_str}' always;\n"
            ));
            if rules.cors.allow_credentials {
                out.push_str("    add_header 'Access-Control-Allow-Credentials' 'true' always;\n");
            }
            out.push_str("    if ($request_method = 'OPTIONS') {\n");
            out.push_str("        add_header 'Access-Control-Allow-Origin' '*' always;\n");
            out.push_str(&format!(
                "        add_header 'Access-Control-Allow-Methods' '{methods_str}' always;\n"
            ));
            out.push_str(&format!(
                "        add_header 'Access-Control-Allow-Headers' '{headers_str}' always;\n"
            ));
            out.push_str(&format!(
                "        add_header 'Access-Control-Max-Age' {max_age};\n"
            ));
            out.push_str("        add_header 'Content-Type' 'text/plain; charset=utf-8';\n");
            out.push_str("        add_header 'Content-Length' 0;\n");
            out.push_str("        return 204;\n");
            out.push_str("    }\n\n");
        }
        CorsPreset::Custom => {
            if !rules.cors.allowed_origins.is_empty() {
                let escaped_origins: Vec<String> = rules
                    .cors
                    .allowed_origins
                    .iter()
                    .map(|o| regex_escape_origin(o))
                    .collect();
                let regex_pattern = escaped_origins.join("|");
                out.push_str("    set $cors_origin \"\";\n");
                out.push_str(&format!(
                    "    if ($http_origin ~* \"^({regex_pattern})$\") {{\n        set $cors_origin $http_origin;\n    }}\n"
                ));
                out.push_str("    add_header 'Access-Control-Allow-Origin' $cors_origin always;\n");
                out.push_str(&format!(
                    "    add_header 'Access-Control-Allow-Methods' '{methods_str}' always;\n"
                ));
                out.push_str(&format!(
                    "    add_header 'Access-Control-Allow-Headers' '{headers_str}' always;\n"
                ));
                if rules.cors.allow_credentials {
                    out.push_str(
                        "    add_header 'Access-Control-Allow-Credentials' 'true' always;\n",
                    );
                }
                out.push_str("    if ($request_method = 'OPTIONS') {\n");
                out.push_str(
                    "        add_header 'Access-Control-Allow-Origin' $cors_origin always;\n",
                );
                out.push_str(&format!(
                    "        add_header 'Access-Control-Allow-Methods' '{methods_str}' always;\n"
                ));
                out.push_str(&format!(
                    "        add_header 'Access-Control-Allow-Headers' '{headers_str}' always;\n"
                ));
                out.push_str(&format!(
                    "        add_header 'Access-Control-Max-Age' {max_age};\n"
                ));
                out.push_str("        add_header 'Content-Type' 'text/plain; charset=utf-8';\n");
                out.push_str("        add_header 'Content-Length' 0;\n");
                out.push_str("        return 204;\n");
                out.push_str("    }\n\n");
            }
        }
        CorsPreset::SameOrigin | CorsPreset::Disabled => {}
    }
}

fn append_global_ip_access(out: &mut String, rules: &SiteRulesConfig) {
    if rules.ip_access.enabled && rules.ip_access.scope == "global" {
        out.push_str("    # --- IP Access Control (Global) ---\n");
        for r in &rules.ip_access.rules {
            match r.action {
                IpAction::Allow => {
                    out.push_str(&format!("    allow {};\n", r.cidr_or_ip));
                }
                IpAction::Deny => {
                    out.push_str(&format!("    deny {};\n", r.cidr_or_ip));
                }
            }
        }
        if rules.ip_access.default_action == IpAction::Deny {
            out.push_str("    deny all;\n");
        }
        out.push('\n');
    }
}

fn append_global_basic_auth(out: &mut String, rules: &SiteRulesConfig, auth_file: &str) {
    if rules.basic_auth.enabled
        && (rules.basic_auth.path == "/" || rules.basic_auth.path.is_empty())
    {
        out.push_str("    # --- HTTP Basic Authentication (Global) ---\n");
        let clean_realm = rules.basic_auth.realm.replace('"', "\\\"");
        out.push_str(&format!("    auth_basic \"{clean_realm}\";\n"));
        out.push_str(&format!("    auth_basic_user_file {auth_file};\n\n"));
    }
}

fn append_custom_error_pages(out: &mut String, rules: &SiteRulesConfig) {
    if !rules.custom_error_pages.is_empty() {
        out.push_str("    # --- Custom Error Pages ---\n");
        for ep in &rules.custom_error_pages {
            let code = ep.status_code;
            let target = ep.target.trim();
            if !target.is_empty() {
                out.push_str(&format!("    error_page {code} {target};\n"));
            }
        }
        out.push('\n');
    }
}

fn append_structured_redirects(out: &mut String, rules: &SiteRulesConfig) {
    let mut has_redirects = false;
    for r in &rules.redirects {
        if r.enabled {
            if !has_redirects {
                out.push_str("    # --- Structured Redirects ---\n");
                has_redirects = true;
            }
            match r.match_type {
                RedirectMatchType::Exact => {
                    out.push_str(&format!(
                        "    location = {} {{\n        return {} {};\n    }}\n",
                        r.source, r.status_code, r.target
                    ));
                }
                RedirectMatchType::Prefix => {
                    out.push_str(&format!(
                        "    location ^~ {} {{\n        return {} {};\n    }}\n",
                        r.source, r.status_code, r.target
                    ));
                }
                RedirectMatchType::Regex => {
                    let flag = if r.status_code == 301 {
                        "permanent"
                    } else {
                        "redirect"
                    };
                    out.push_str(&format!("    rewrite {} {} {flag};\n", r.source, r.target));
                }
            }
        }
    }
    if has_redirects {
        out.push('\n');
    }
}

fn append_hotlink_protection(out: &mut String, rules: &SiteRulesConfig) {
    if rules.hotlink_protection.enabled && !rules.hotlink_protection.file_extensions.is_empty() {
        out.push_str("    # --- Hotlink Protection ---\n");
        let exts = rules.hotlink_protection.file_extensions.join("|");
        let none_flag = if rules.hotlink_protection.allow_empty_referer {
            "none blocked"
        } else {
            "blocked"
        };
        let mut allowed = String::new();
        for d in &rules.hotlink_protection.allowed_domains {
            allowed.push(' ');
            allowed.push_str(d.trim());
        }
        out.push_str(&format!(
            "    location ~* \\.(?:{exts})$ {{\n        valid_referers {none_flag} server_names{allowed};\n        if ($invalid_referer) {{\n"
        ));
        match rules.hotlink_protection.action {
            HotlinkAction::Deny => {
                out.push_str("            return 403;\n");
            }
            HotlinkAction::Redirect => {
                if let Some(ref redir) = rules.hotlink_protection.redirect_url {
                    out.push_str(&format!("            return 302 {redir};\n"));
                } else {
                    out.push_str("            return 403;\n");
                }
            }
        }
        out.push_str("        }\n    }\n\n");
    }
}

fn append_scoped_basic_auth(out: &mut String, rules: &SiteRulesConfig, auth_file: &str) {
    if rules.basic_auth.enabled && rules.basic_auth.path != "/" && !rules.basic_auth.path.is_empty()
    {
        out.push_str("    # --- HTTP Basic Authentication (Path Scope) ---\n");
        let clean_path = rules.basic_auth.path.trim();
        let clean_realm = rules.basic_auth.realm.replace('"', "\\\"");
        out.push_str(&format!(
            "    location {} {{\n        auth_basic \"{clean_realm}\";\n        auth_basic_user_file {auth_file};\n    }}\n\n",
            clean_path
        ));
    }
}

fn append_scoped_ip_access(out: &mut String, rules: &SiteRulesConfig) {
    if !rules.ip_access.enabled || rules.ip_access.scope != "path" {
        return;
    }
    if let Some(clean_path) = rules
        .ip_access
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        out.push_str("    # --- IP Access Control (Path Scope) ---\n");
        out.push_str(&format!("    location {clean_path} {{\n"));
        for r in &rules.ip_access.rules {
            match r.action {
                IpAction::Allow => {
                    out.push_str(&format!("        allow {};\n", r.cidr_or_ip));
                }
                IpAction::Deny => {
                    out.push_str(&format!("        deny {};\n", r.cidr_or_ip));
                }
            }
        }
        if rules.ip_access.default_action == IpAction::Deny {
            out.push_str("        deny all;\n");
        }
        out.push_str("    }\n\n");
    }
}

fn append_nginx_snippet(out: &mut String, rules: &SiteRulesConfig) {
    if rules.nginx_snippet.enabled {
        let trimmed_snippet = rules.nginx_snippet.content.trim();
        if !trimmed_snippet.is_empty() {
            out.push_str("    # --- Custom Nginx Snippet (Admin) ---\n");
            for line in trimmed_snippet.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }
}

fn regex_escape_origin(origin: &str) -> String {
    let mut out = String::new();
    for c in origin.chars() {
        if c == '.'
            || c == '+'
            || c == '?'
            || c == '*'
            || c == '^'
            || c == '$'
            || c == '('
            || c == ')'
            || c == '['
            || c == ']'
            || c == '{'
            || c == '}'
            || c == '|'
            || c == '\\'
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use panel_core::site_rules::*;

    #[test]
    fn default_404_is_preserved_with_other_custom_errors_and_replaced_by_custom_404() {
        for custom_code in [500, 404] {
            let rules = SiteRulesConfig {
                custom_error_pages: vec![CustomErrorPage {
                    status_code: custom_code,
                    target: "/errors/custom.html".into(),
                }],
                ..SiteRulesConfig::default()
            };
            let conf = generate_site_rules_nginx_block(&rules, "1", None);
            assert!(conf.contains(&format!("error_page {custom_code} /errors/custom.html;")));
            assert_eq!(
                conf.contains("error_page 404 @qpanel_not_found;"),
                custom_code != 404
            );
            assert_eq!(
                conf.contains("location @qpanel_not_found"),
                custom_code != 404
            );
            assert!(!conf.contains("=200"));
        }
    }

    #[test]
    fn test_generate_site_rules_redirects_and_headers() {
        let rules = SiteRulesConfig {
            redirects: vec![
                StructuredRedirect {
                    id: "r1".to_string(),
                    source: "/old".to_string(),
                    target: "/new".to_string(),
                    status_code: 301,
                    match_type: RedirectMatchType::Exact,
                    enabled: true,
                },
                StructuredRedirect {
                    id: "r2".to_string(),
                    source: "^/legacy/(.*)$".to_string(),
                    target: "/current/$1".to_string(),
                    status_code: 302,
                    match_type: RedirectMatchType::Regex,
                    enabled: true,
                },
            ],
            security_headers: SecurityHeadersConfig {
                hsts_enabled: true,
                hsts_subdomains: true,
                hsts_preload: false,
                x_frame_options: Some("SAMEORIGIN".to_string()),
                x_content_type_options: true,
                referrer_policy: Some("no-referrer".to_string()),
                permissions_policy: None,
                csp: None,
            },
            ..Default::default()
        };

        let rendered = generate_site_rules_nginx_block(&rules, "42", None);
        assert!(rendered.contains("location = /old {"));
        assert!(rendered.contains("return 301 /new;"));
        assert!(rendered.contains("rewrite ^/legacy/(.*)$ /current/$1 redirect;"));
        assert!(rendered.contains(
            "add_header Strict-Transport-Security \"max-age=31536000; includeSubDomains\" always;"
        ));
        assert!(rendered.contains("add_header X-Frame-Options \"SAMEORIGIN\" always;"));
        assert!(rendered.contains("add_header X-Content-Type-Options \"nosniff\" always;"));
        assert!(rendered.contains("add_header Referrer-Policy \"no-referrer\" always;"));
    }

    #[test]
    fn test_generate_site_rules_maintenance_and_auth_and_ip() {
        let rules = SiteRulesConfig {
            maintenance: MaintenanceConfig {
                enabled: true,
                bypass_ips: vec!["203.0.113.195".to_string()],
                retry_after_secs: 1800,
                custom_message: "Sedang upgrade sistem".to_string(),
            },
            basic_auth: BasicAuthConfig {
                enabled: true,
                path: "/".to_string(),
                realm: "Area Rahasia".to_string(),
                users: vec![BasicAuthUser {
                    username: "admin".to_string(),
                    password_hash: "hash123".to_string(),
                }],
            },
            ip_access: IpAccessConfig {
                enabled: true,
                scope: "global".to_string(),
                path: None,
                default_action: IpAction::Deny,
                rules: vec![IpAccessRule {
                    id: "ip1".to_string(),
                    action: IpAction::Allow,
                    cidr_or_ip: "10.0.0.0/8".to_string(),
                    description: None,
                }],
            },
            ..Default::default()
        };

        let rendered = generate_site_rules_nginx_block(&rules, "99", Some("/tmp/custom.htpasswd"));
        assert!(rendered.contains("set $maintenance_active 1;"));
        assert!(
            rendered
                .contains("if ($remote_addr = \"203.0.113.195\") { set $maintenance_active 0; }")
        );
        assert!(rendered.contains("Retry-After 1800 always;"));
        assert!(rendered.contains("auth_basic \"Area Rahasia\";"));
        assert!(rendered.contains("auth_basic_user_file /tmp/custom.htpasswd;"));
        assert!(rendered.contains("allow 10.0.0.0/8;"));
        assert!(rendered.contains("deny all;"));
    }
}
