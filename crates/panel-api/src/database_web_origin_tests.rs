#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use axum::extract::ConnectInfo;
use std::net::SocketAddr;

fn browser(method: &str, query: &str, origin: &str, peer: Option<&str>) -> Request {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/api/v1/databases/1/web/index.php?{query}"))
        .header("host", "panel.test")
        .header("origin", origin)
        .header("sec-fetch-site", "same-origin");
    if let Some(peer) = peer {
        builder = builder.extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    }
    builder.body(Body::empty()).unwrap()
}

#[test]
fn https_bootstrap_posts_use_the_public_scheme_with_http_cookie_policy() {
    for (method, query) in [
        ("GET", "route=/database/structure"),
        ("POST", "route=/config/get"),
        ("POST", "route=/navigation&ajax_request=1"),
    ] {
        let mut request = browser(method, query, "https://panel.test", Some("127.0.0.1:44000"));
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        // Reproduces the previous mismatch: a valid HTTPS Origin was compared to HTTP.
        assert!(!same_origin(request.headers(), false));
        let secure = request_secure(&request, false);
        assert!(secure);
        assert!(same_origin(request.headers(), secure));
        let credentials = DatabaseWebCredentials {
            username: "fixture-user".into(),
            password: "fixture-only".into(),
        };
        let params = parameters(WebRequest {
            engine: "mariadb",
            database: "tenant_shop",
            prefix: "/api/v1/databases/1/web/",
            asset: "index.php",
            query,
            method,
            content_type: "application/x-www-form-urlencoded",
            host: "panel.test",
            cookie: "phpMyAdmin_https=fixture",
            session: &"a".repeat(64),
            credentials: &credentials,
            secure,
            length: 0,
        })
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params["HTTPS"], "on");
        assert_eq!(params["SERVER_PORT"], "443");
        assert_eq!(params["QUERY_STRING"], query);
        assert_eq!(params["REQUEST_METHOD"], method);
        assert_eq!(params["HTTP_COOKIE"], "phpMyAdmin_https=fixture");
    }
}

#[test]
fn cloudflare_visitor_scheme_survives_an_http_nginx_hop() {
    for peer in ["127.0.0.1:44000", "[::1]:44000"] {
        let mut request = browser(
            "POST",
            "route=/config/get",
            "https://panel.test",
            Some(peer),
        );
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("http"));
        request.headers_mut().insert(
            "x-qpanel-cf-visitor",
            HeaderValue::from_static(r#"{"scheme":"https"}"#),
        );
        assert!(request_secure(&request, false));
        assert!(same_origin(
            request.headers(),
            request_secure(&request, false)
        ));
    }
}

#[test]
fn forwarded_scheme_requires_a_known_loopback_peer() {
    for peer in [None, Some("192.0.2.20:44000")] {
        let mut request = browser("POST", "route=/navigation", "https://panel.test", peer);
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        request.headers_mut().insert(
            "x-qpanel-cf-visitor",
            HeaderValue::from_static(r#"{"scheme":"https"}"#),
        );
        assert!(!request_secure(&request, false));
        assert!(!same_origin(
            request.headers(),
            request_secure(&request, false)
        ));
    }
}

#[test]
fn cookie_minimum_does_not_change_transport_and_ambiguous_metadata_cannot_select_https() {
    for value in ["http", "https,http", "ftp", "https://", ""] {
        let mut request = browser(
            "POST",
            "route=/config/get",
            "https://panel.test",
            Some("127.0.0.1:44000"),
        );
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_str(value).unwrap());
        assert!(!request_secure(&request, false));
        assert!(!request_secure(&request, true));
    }
    let mut request = browser("GET", "", "https://panel.test", Some("127.0.0.1:44000"));
    request
        .headers_mut()
        .append("x-forwarded-proto", HeaderValue::from_static("https"));
    request
        .headers_mut()
        .append("x-forwarded-proto", HeaderValue::from_static("http"));
    assert!(!request_secure(&request, false));
    request.headers_mut().remove("x-forwarded-proto");
    for value in [
        "not-json",
        r#"{"scheme":"ftp"}"#,
        r#"{"scheme":true}"#,
        "[]",
    ] {
        request
            .headers_mut()
            .insert("x-qpanel-cf-visitor", HeaderValue::from_str(value).unwrap());
        assert!(!request_secure(&request, false));
    }
}

#[test]
fn custom_ports_ipv6_and_default_port_redirects_use_the_request_origin() {
    let prefix = "/api/v1/databases/1/web/";
    for (host, location) in [
        (
            "panel.test",
            "https://PANEL.test:443/api/v1/databases/1/web/index.php?route=%2Fsql",
        ),
        (
            "panel.test:8443",
            "https://panel.test:8443/api/v1/databases/1/web/index.php",
        ),
        (
            "[2001:db8::7]:19443",
            "https://[2001:db8::7]:19443/api/v1/databases/1/web/index.php",
        ),
    ] {
        let response = parse_response(
            format!("Location: {location}\r\n\r\n").as_bytes(),
            prefix,
            true,
            host,
        )
        .unwrap();
        assert!(
            response.headers()["location"]
                .to_str()
                .unwrap()
                .starts_with(prefix)
        );
    }
    for location in [
        "https://panel.test.evil.test/api/v1/databases/1/web/",
        "http://panel.test/api/v1/databases/1/web/",
        "//evil.test",
        "/api/v1/databases/1/web/../other",
        "/api/v1/databases/1/web/%2e%2e/other",
        "https://panel.test:8443/api/v1/databases/1/web/",
    ] {
        assert!(
            parse_response(
                format!("Location: {location}\r\n\r\n").as_bytes(),
                prefix,
                true,
                "panel.test"
            )
            .is_err(),
            "{location}"
        );
    }
    let credentials = DatabaseWebCredentials {
        username: "fixture".into(),
        password: "fixture".into(),
    };
    for (host, server, port) in [
        ("[2001:db8::7]:19443", "2001:db8::7", "19443"),
        ("panel.test:8443", "panel.test", "8443"),
    ] {
        let params: std::collections::HashMap<_, _> = parameters(WebRequest {
            engine: "mariadb",
            database: "fixture",
            prefix,
            asset: "index.php",
            query: "",
            method: "GET",
            content_type: "",
            host,
            cookie: "",
            session: "fixture",
            credentials: &credentials,
            secure: true,
            length: 0,
        })
        .into_iter()
        .collect();
        assert_eq!(params["SERVER_NAME"], server);
        assert_eq!(params["SERVER_PORT"], port);
        assert_eq!(params["HTTP_HOST"], host);
    }
}

#[test]
fn public_https_never_accepts_foreign_null_or_downgraded_origins() {
    for origin in [
        "https://evil.test",
        "https://sub.panel.test",
        "https://panel.test:444",
        "http://panel.test",
        "null",
        "https://panel.test/",
    ] {
        let mut request = browser("POST", "route=/config/get", origin, Some("127.0.0.1:44000"));
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(!same_origin(
            request.headers(),
            request_secure(&request, false)
        ));
    }
    let mut request = browser(
        "POST",
        "route=/config/get",
        "https://panel.test",
        Some("127.0.0.1:44000"),
    );
    request
        .headers_mut()
        .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
    assert!(!same_origin(request.headers(), true));
}

#[test]
fn direct_http_still_works_and_https_responses_keep_scoped_secure_cookies() {
    let request = browser(
        "POST",
        "route=/config/get",
        "http://panel.test",
        Some("127.0.0.1:44000"),
    );
    assert!(!request_secure(&request, false));
    assert!(same_origin(request.headers(), false));
    let response = parse_response(
        b"Location: https://panel.test/api/v1/databases/1/web/index.php\r\nSet-Cookie: phpMyAdmin_https=fixture; Path=/\r\n\r\n",
        "/api/v1/databases/1/web/", true, "panel.test",
    ).unwrap();
    assert_eq!(
        response.headers()["location"],
        "/api/v1/databases/1/web/index.php"
    );
    assert_eq!(
        response.headers()["set-cookie"],
        "phpMyAdmin_https=fixture; Path=/api/v1/databases/1/web/; HttpOnly; SameSite=Strict; Secure"
    );
}
