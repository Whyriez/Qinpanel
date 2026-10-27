//! Stable release identifiers shared by the update API and privileged agent.
use crate::CoreError;

/// Distribution is selected by the release build, never by an API request.
pub fn release_metadata_url(base: Option<&str>) -> Result<String, CoreError> {
    let Some(base) = base.filter(|s| !s.is_empty()) else {
        return Ok("https://api.github.com/repos/Whyriez/Qinpanel/releases/latest".into());
    };
    let invalid = || CoreError::Validation("URL distribusi release tidak valid".into());
    let authority = base.strip_prefix("https://").ok_or_else(invalid)?;
    let mut segments = authority.split('/');
    let host = segments.next().ok_or_else(invalid)?;
    if base.len() > 512
        || host.is_empty()
        || !host.contains('.')
        || host.contains("..")
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        || !host
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !host
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || segments.any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    {
        return Err(invalid());
    }
    Ok(format!("{base}/latest.json"))
}

pub fn release_version(tag: &str) -> Result<(u32, u32, u32), CoreError> {
    let invalid =
        || CoreError::Validation("Versi harus berupa tag stabil vMAJOR.MINOR.PATCH".into());
    let body = tag.strip_prefix('v').ok_or_else(invalid)?;
    if body.len() > 26 {
        return Err(invalid());
    }
    let mut parts = body.split('.');
    let mut part = || {
        let value = parts.next().ok_or_else(invalid)?;
        if value.is_empty()
            || value.len() > 8
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(invalid());
        }
        value.parse().map_err(|_| invalid())
    };
    let result = (part()?, part()?, part()?);
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(result)
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod distribution_tests {
    use super::*;
    #[test]
    fn build_pins_https_origin_and_path_without_credentials_or_overrides() {
        for base in [
            "https://download.arelqon.com/qonpanel",
            "https://project.supabase.co/storage/v1/object/public/qonpanel-releases",
        ] {
            assert_eq!(
                release_metadata_url(Some(base)).unwrap(),
                format!("{base}/latest.json")
            );
        }
        for base in [
            "http://downloads.example.com",
            "https://user:secret@example.com",
            "https://example.com/a/../b",
            "https://example.com/a?key=secret",
            "https://example.com/a#x",
            "https://example.com/",
            "https://example.com\n",
            "https://example.com//path",
            "https://example.com:443",
        ] {
            assert!(release_metadata_url(Some(base)).is_err());
        }
        assert!(
            release_metadata_url(None)
                .unwrap()
                .starts_with("https://api.github.com/")
        );
    }
}
