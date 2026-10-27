use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error returned when domain validation fails.
#[derive(Debug, Error, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum DomainValidationError {
    #[error("Domain name cannot be empty")]
    Empty,

    #[error("Domain name is too long (maximum 253 characters, found {0})")]
    TooLong(usize),

    #[error("Domain contains invalid or control characters")]
    InvalidCharacters,

    #[error("Domain must contain at least a domain and a top-level domain (TLD)")]
    TooFewLabels,

    #[error("Domain label '{label}' is invalid: {reason}")]
    InvalidLabel { label: String, reason: &'static str },

    #[error("Top-level domain (TLD) '{0}' cannot be purely numeric")]
    NumericTld(String),

    #[error("Internationalized Domain Name (IDN) error: {0}")]
    IdnaError(String),
}

/// A validated, normalized domain name holding both ASCII (Punycode) and Unicode representations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DomainName {
    /// Normalized ASCII punycode representation (e.g., "xn--caf-dma.fr" or "example.com").
    /// This value is always lowercase and safe for DNS, Nginx configs, and filesystem paths.
    ascii: String,
    /// Unicode display representation (e.g., "café.fr" or "example.com").
    unicode: String,
}

impl DomainName {
    /// Parses, validates, and normalizes a domain name (supporting both standard ASCII and IDN).
    pub fn parse(raw: &str) -> Result<Self, DomainValidationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainValidationError::Empty);
        }

        // Strict security guard against injection characters:
        // Reject whitespace, newlines, null bytes, shell metacharacters, and Nginx directive separators.
        for ch in trimmed.chars() {
            if ch.is_whitespace() || ch.is_control() {
                return Err(DomainValidationError::InvalidCharacters);
            }
            match ch {
                ';' | '{' | '}' | '$' | '"' | '\'' | '\\' | '/' | '|' | '&' | '<' | '>' | '`'
                | '!' | '(' | ')' | '*' | '?' | '[' | ']' | '^' | '%' | '#' | '@' => {
                    return Err(DomainValidationError::InvalidCharacters);
                }
                _ => {}
            }
        }

        // Convert and validate via IDN Punycode algorithms
        let ascii_domain = idna::domain_to_ascii(trimmed)
            .map_err(|e| DomainValidationError::IdnaError(format!("{:?}", e)))?;

        // Total ASCII length limit (RFC 1035 / RFC 1123: max 253 characters)
        if ascii_domain.is_empty() {
            return Err(DomainValidationError::Empty);
        }
        if ascii_domain.len() > 253 {
            return Err(DomainValidationError::TooLong(ascii_domain.len()));
        }

        // Split into labels
        let labels: Vec<&str> = ascii_domain.split('.').collect();
        if labels.len() < 2 {
            return Err(DomainValidationError::TooFewLabels);
        }

        for &label in &labels {
            if label.is_empty() {
                return Err(DomainValidationError::InvalidLabel {
                    label: label.to_string(),
                    reason: "Label cannot be empty (consecutive dots are not allowed)",
                });
            }

            if label.len() > 63 {
                return Err(DomainValidationError::InvalidLabel {
                    label: label.to_string(),
                    reason: "Label exceeds maximum length of 63 characters",
                });
            }

            if label.starts_with('-') || label.ends_with('-') {
                return Err(DomainValidationError::InvalidLabel {
                    label: label.to_string(),
                    reason: "Label cannot start or end with a hyphen",
                });
            }

            // In ASCII punycode form, labels must only contain letters, digits, and hyphens
            for c in label.chars() {
                if !c.is_ascii_alphanumeric() && c != '-' {
                    return Err(DomainValidationError::InvalidLabel {
                        label: label.to_string(),
                        reason: "Label contains non-alphanumeric characters",
                    });
                }
            }
        }

        // Validate TLD (the last label)
        let tld = match labels.last() {
            Some(&t) => t,
            None => return Err(DomainValidationError::TooFewLabels),
        };

        // RFC 1123: Top-level domains cannot be all numeric (e.g. "example.123" is invalid)
        if tld.chars().all(|c| c.is_ascii_digit()) {
            return Err(DomainValidationError::NumericTld(tld.to_string()));
        }

        // Generate Unicode representation for UI display
        let (unicode_domain, _) = idna::domain_to_unicode(&ascii_domain);

        Ok(Self {
            ascii: ascii_domain.to_ascii_lowercase(),
            unicode: unicode_domain,
        })
    }

    /// Returns the normalized ASCII / Punycode representation (lowercase).
    pub fn as_ascii(&self) -> &str {
        &self.ascii
    }

    /// Returns the Unicode representation suitable for user interface display.
    pub fn as_unicode(&self) -> &str {
        &self.unicode
    }

    /// Returns true if this domain is an Internationalized Domain Name (contains non-ASCII characters).
    pub fn is_idn(&self) -> bool {
        self.ascii.starts_with("xn--") || self.ascii.contains(".xn--")
    }

    /// Returns the root domain / apex domain (e.g., for "sub.example.com", returns "example.com" if standard).
    pub fn tld(&self) -> Option<&str> {
        self.ascii.split('.').next_back()
    }
}

impl std::fmt::Display for DomainName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.ascii)
    }
}

impl std::str::FromStr for DomainName {
    type Err = DomainValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_standard_ascii_domains() {
        let d = DomainName::parse("example.com").unwrap();
        assert_eq!(d.as_ascii(), "example.com");
        assert_eq!(d.as_unicode(), "example.com");
        assert!(!d.is_idn());

        let d2 = DomainName::parse("SUB.My-Domain.CO.ID").unwrap();
        assert_eq!(d2.as_ascii(), "sub.my-domain.co.id");
        assert_eq!(d2.as_unicode(), "sub.my-domain.co.id");
        assert!(!d2.is_idn());
        assert_eq!(d2.tld(), Some("id"));
    }

    #[test]
    fn test_valid_idn_punycode_domains() {
        // German umlaut: münchen.de -> xn--mnchen-3ya.de
        let d = DomainName::parse("münchen.de").unwrap();
        assert_eq!(d.as_ascii(), "xn--mnchen-3ya.de");
        assert_eq!(d.as_unicode(), "münchen.de");
        assert!(d.is_idn());

        // French accent: café.com -> xn--caf-dma.com
        let d2 = DomainName::parse("café.com").unwrap();
        assert_eq!(d2.as_ascii(), "xn--caf-dma.com");
        assert_eq!(d2.as_unicode(), "café.com");
        assert!(d2.is_idn());

        // Parsing directly in punycode form recovers unicode display
        let d3 = DomainName::parse("xn--caf-dma.com").unwrap();
        assert_eq!(d3.as_ascii(), "xn--caf-dma.com");
        assert_eq!(d3.as_unicode(), "café.com");
        assert!(d3.is_idn());
    }

    #[test]
    fn test_anti_injection_and_forbidden_characters() {
        // Nginx config injection characters
        assert_eq!(
            DomainName::parse("example.com; server { ... }"),
            Err(DomainValidationError::InvalidCharacters)
        );
        assert_eq!(
            DomainName::parse("example.com\nnewline.com"),
            Err(DomainValidationError::InvalidCharacters)
        );
        assert_eq!(
            DomainName::parse("example.com\0null"),
            Err(DomainValidationError::InvalidCharacters)
        );
        assert_eq!(
            DomainName::parse("test$variable.com"),
            Err(DomainValidationError::InvalidCharacters)
        );
        assert_eq!(
            DomainName::parse("test/slash.com"),
            Err(DomainValidationError::InvalidCharacters)
        );
        assert_eq!(
            DomainName::parse("test\"quote.com"),
            Err(DomainValidationError::InvalidCharacters)
        );
    }

    #[test]
    fn test_label_and_length_boundaries() {
        // Empty
        assert_eq!(DomainName::parse(""), Err(DomainValidationError::Empty));
        assert_eq!(DomainName::parse("   "), Err(DomainValidationError::Empty));

        // Too few labels (single label)
        assert_eq!(
            DomainName::parse("localhost"),
            Err(DomainValidationError::TooFewLabels)
        );

        // Consecutive dots
        assert!(matches!(
            DomainName::parse("example..com"),
            Err(DomainValidationError::InvalidLabel { .. })
        ));

        // Leading / trailing hyphens in label
        assert!(matches!(
            DomainName::parse("-example.com"),
            Err(DomainValidationError::InvalidLabel { .. })
        ));
        assert!(matches!(
            DomainName::parse("example-.com"),
            Err(DomainValidationError::InvalidLabel { .. })
        ));

        // Numeric TLD
        assert!(matches!(
            DomainName::parse("example.123"),
            Err(DomainValidationError::NumericTld(_))
        ));

        // Label too long (> 63 chars)
        let long_label = "a".repeat(64);
        assert!(matches!(
            DomainName::parse(&format!("{}.com", long_label)),
            Err(DomainValidationError::InvalidLabel { .. })
        ));
    }
}
