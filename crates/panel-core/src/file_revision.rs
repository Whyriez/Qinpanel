//! Content revisions for editor optimistic concurrency; independent of timestamps and size.
use sha2::{Digest, Sha256};

pub const MAX_EDITOR_CONTENT: u64 = 5 * 1024 * 1024;

pub fn content_etag(bytes: &[u8]) -> String {
    format!("\"sha256-{:x}\"", Sha256::digest(bytes))
}

pub fn is_content_etag(value: &str) -> bool {
    value.len() == 73
        && value.starts_with("\"sha256-")
        && value.ends_with('"')
        && value.as_bytes()[8..72].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_length_changes_have_distinct_revisions() {
        assert_ne!(content_etag(b"before"), content_etag(b"after!"));
        assert!(is_content_etag(&content_etag(b"before")));
        assert!(!is_content_etag("\"65abcd-6\""));
        assert!(!is_content_etag(""));
    }
}
