#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Transport fixtures only. The explicit ignored smoke reads official metadata, never installs.
use super::*;
use serde_json::json;

#[test]
fn source_responses_distinguish_network_tls_http_and_invalid_json() {
    assert_eq!(
        decode_source(b"{\n\"ok\":true\n}\n200", Some(0)).unwrap(),
        json!({"ok":true})
    );
    for (output, exit, message, retryable) in [
        (b"\n000".as_slice(), 6, "DNS", true),
        (b"partial JSON\n200".as_slice(), 28, "waktu", true),
        (b"\n000".as_slice(), 60, "TLS", false),
        (b"\n404".as_slice(), 22, "HTTP 404", false),
        (b"\n429".as_slice(), 22, "HTTP 429", true),
        (b"\n503".as_slice(), 22, "HTTP 503", true),
        (b"\n000".as_slice(), 63, "8 MiB", false),
        (b"<html>error</html>\n200".as_slice(), 0, "JSON", false),
        (b"\n302".as_slice(), 0, "HTTP 302", false),
    ] {
        let error = decode_source(output, Some(exit)).unwrap_err();
        assert!(error.reason.contains(message), "{error:?}");
        assert_eq!(error.retryable, retryable, "{error:?}");
    }
}

#[tokio::test]
async fn decompressed_source_bytes_are_bounded_before_json_parsing() {
    let small = b"{}\n200";
    assert_eq!(read_source_output(small.as_slice()).await.unwrap(), small);
    let expanded = vec![b' '; MAX_SOURCE_BYTES + 1];
    let error = read_source_output(expanded.as_slice()).await.unwrap_err();
    assert!(error.reason.contains("8 MiB"));
    assert!(!error.retryable);
}

#[tokio::test]
async fn partial_timeout_retries_with_a_fresh_response_buffer() {
    let mut calls = 0;
    let value = fetch_retry("Status dukungan Python (PEP)", 2, || {
        calls += 1;
        std::future::ready(if calls == 1 {
            decode_source(b"{\"partial\":\n200", Some(28))
        } else {
            decode_source(b"{\"complete\":true}\n200", Some(0))
        })
    })
    .await
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(value, json!({"complete":true}));
}

#[tokio::test]
async fn permanent_source_errors_do_not_retry_or_expose_raw_diagnostics() {
    let mut calls = 0;
    let error = fetch_retry("Katalog Python (Astral)", 2, || {
        calls += 1;
        std::future::ready(decode_source(b"\n404", Some(22)))
    })
    .await
    .unwrap_err();
    assert_eq!(calls, 1);
    assert!(error.contains("Katalog Python (Astral)") && error.contains("HTTP 404"));
    assert!(error.contains("Katalog terakhir dipertahankan"));
}

#[tokio::test]
async fn exhausted_retry_keeps_last_good_python_cache_then_recovers() {
    let pool = panel_db::pool::init_pool("sqlite::memory:").await.unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();
    let previous = vec![PythonVersionEntry {
        version: "3.14.8".into(),
        major_minor: "3.14".into(),
        release_date: None,
        is_default: true,
        is_lts_or_stable: true,
        description: "CPython stabil".into(),
    }];
    store_refresh(&pool, "runtime.catalog.python", Ok(previous.clone()), 1000)
        .await
        .unwrap();
    let mut calls = 0;
    let failure = fetch_retry("Status dukungan Python (PEP)", 2, || {
        calls += 1;
        std::future::ready(decode_source(b"\n503", Some(22)))
    })
    .await
    .map(|_| Vec::<PythonVersionEntry>::new());
    assert_eq!(calls, 2);
    store_refresh(&pool, "runtime.catalog.python", failure, 2000)
        .await
        .unwrap();
    let cache = python_cache(&pool).await.unwrap();
    assert_eq!(cache.data[0].version, "3.14.8");
    assert_eq!(cache.refreshed_at, Some(1000));
    assert_eq!(cache.checked_at, Some(2000));
    assert!(cache.error.unwrap().contains("HTTP 503"));
    store_refresh(&pool, "runtime.catalog.python", Ok(previous), 3000)
        .await
        .unwrap();
    let cache = python_cache(&pool).await.unwrap();
    assert_eq!(cache.refreshed_at, Some(3000));
    assert!(cache.error.is_none());
}

#[tokio::test]
#[ignore = "Opt-in read-only smoke against fixed official metadata URLs; no daemon or installer"]
async fn official_python_sources_produce_a_supported_catalog() {
    let (downloads, cycles) =
        tokio::try_join!(fetch(PYTHON_DOWNLOADS), fetch(PYTHON_CYCLES)).unwrap();
    let versions = parse_python(downloads, &cycles, Utc::now().date_naive()).unwrap();
    assert!(!versions.is_empty());
    assert!(versions[0].is_default);
    assert!(versions.iter().all(|version| version.is_lts_or_stable));
    println!(
        "Official Python catalog: {}",
        versions
            .iter()
            .map(|version| version.version.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}
