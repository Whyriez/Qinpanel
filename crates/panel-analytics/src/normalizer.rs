//! URL Path normalizer for grouping high-cardinality endpoint paths into templates.
//!
//! Example:
//! `/posts/123` -> `/posts/:id`
//! `/api/v1/orders/550e8400-e29b-41d4-a716-446655440000` -> `/api/v1/orders/:uuid`
//! `/files/d41d8cd98f00b204e9800998ecf8427e/download` -> `/files/:hash/download`

pub struct PathNormalizer;

impl PathNormalizer {
    /// Normalize a raw request URI by stripping query strings and abstracting IDs/UUIDs/hashes.
    pub fn normalize_path(uri: &str) -> String {
        // Strip query string and fragment
        let path = match uri.find('?') {
            Some(idx) => &uri[..idx],
            None => match uri.find('#') {
                Some(idx) => &uri[..idx],
                None => uri,
            },
        };

        if path.is_empty() || path == "/" {
            return "/".to_string();
        }

        let is_root_prefixed = path.starts_with('/');
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if segments.is_empty() {
            return "/".to_string();
        }

        let mut normalized_segments = Vec::with_capacity(segments.len());

        for segment in segments {
            if Self::is_numeric_id(segment) {
                normalized_segments.push(":id");
            } else if Self::is_uuid(segment) {
                normalized_segments.push(":uuid");
            } else if Self::is_hex_hash(segment) {
                normalized_segments.push(":hash");
            } else {
                normalized_segments.push(segment);
            }
        }

        let mut result = String::with_capacity(path.len());
        if is_root_prefixed {
            result.push('/');
        }
        result.push_str(&normalized_segments.join("/"));

        result
    }

    /// Check if segment is an integer ID (e.g., "123", "042").
    fn is_numeric_id(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
    }

    /// Check if segment matches standard UUID format (8-4-4-4-12 hex).
    fn is_uuid(s: &str) -> bool {
        if s.len() != 36 {
            return false;
        }
        let bytes = s.as_bytes();
        if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
            return false;
        }

        for (i, &b) in bytes.iter().enumerate() {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                continue;
            }
            if !b.is_ascii_hexdigit() {
                return false;
            }
        }

        true
    }

    /// Check if segment is a 32-char (MD5) or 40-char (SHA1) or 64-char (SHA256) hex hash.
    fn is_hex_hash(s: &str) -> bool {
        let len = s.len();
        (len == 32 || len == 40 || len == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_root_and_simple() {
        assert_eq!(PathNormalizer::normalize_path("/"), "/");
        assert_eq!(PathNormalizer::normalize_path(""), "/");
        assert_eq!(PathNormalizer::normalize_path("/home"), "/home");
        assert_eq!(
            PathNormalizer::normalize_path("/api/v1/health"),
            "/api/v1/health"
        );
    }

    #[test]
    fn test_normalize_strip_query_string() {
        assert_eq!(
            PathNormalizer::normalize_path("/search?q=rust&page=2"),
            "/search"
        );
        assert_eq!(
            PathNormalizer::normalize_path("/posts?sort=recent#top"),
            "/posts"
        );
    }

    #[test]
    fn test_normalize_numeric_id() {
        assert_eq!(PathNormalizer::normalize_path("/posts/123"), "/posts/:id");
        assert_eq!(
            PathNormalizer::normalize_path("/api/v1/users/42/items/999"),
            "/api/v1/users/:id/items/:id"
        );
    }

    #[test]
    fn test_normalize_uuid() {
        assert_eq!(
            PathNormalizer::normalize_path("/orders/550e8400-e29b-41d4-a716-446655440000"),
            "/orders/:uuid"
        );
        assert_eq!(
            PathNormalizer::normalize_path("/tenant/a1b2c3d4-e5f6-7890-abcd-ef1234567890/settings"),
            "/tenant/:uuid/settings"
        );
    }

    #[test]
    fn test_normalize_hex_hash() {
        assert_eq!(
            PathNormalizer::normalize_path("/files/d41d8cd98f00b204e9800998ecf8427e/view"),
            "/files/:hash/view"
        );
    }
}
