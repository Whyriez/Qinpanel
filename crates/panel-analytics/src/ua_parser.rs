//! Fast, zero-regex User-Agent parser and bot classifier.
//!
//! Classifies User-Agent strings into browser, OS, device type, and detects automated bot crawlers.

use serde::{Deserialize, Serialize};

/// Parsed metadata extracted from a User-Agent string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UaInfo {
    pub browser: String,
    pub os: String,
    pub device: String,
    pub is_bot: bool,
    pub bot_name: Option<String>,
}

impl Default for UaInfo {
    fn default() -> Self {
        Self {
            browser: "Other".to_string(),
            os: "Other".to_string(),
            device: "Desktop".to_string(),
            is_bot: false,
            bot_name: None,
        }
    }
}

pub struct UaParser;

impl UaParser {
    /// Parse a User-Agent string into structured `UaInfo`.
    pub fn parse(ua_opt: Option<&str>) -> UaInfo {
        let ua = match ua_opt {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return UaInfo::default(),
        };

        let ua_lower = ua.to_lowercase();

        // 1. Bot detection
        if let Some(bot_name) = Self::detect_bot(&ua_lower, ua) {
            return UaInfo {
                browser: bot_name.clone(),
                os: Self::detect_os(&ua_lower),
                device: "Bot".to_string(),
                is_bot: true,
                bot_name: Some(bot_name),
            };
        }

        // 2. Device classification
        let device = Self::detect_device(&ua_lower);

        // 3. Operating System
        let os = Self::detect_os(&ua_lower);

        // 4. Browser
        let browser = Self::detect_browser(&ua_lower);

        UaInfo {
            browser,
            os,
            device,
            is_bot: false,
            bot_name: None,
        }
    }

    fn detect_bot(lower: &str, raw: &str) -> Option<String> {
        let known_bots = [
            ("googlebot", "Googlebot"),
            ("bingbot", "Bingbot"),
            ("yandexbot", "YandexBot"),
            ("baiduspider", "Baiduspider"),
            ("duckduckbot", "DuckDuckBot"),
            ("ahrefsbot", "AhrefsBot"),
            ("semrushbot", "SemrushBot"),
            ("petalbot", "PetalBot"),
            ("mj12bot", "MJ12bot"),
            ("dotbot", "DotBot"),
            ("applebot", "Applebot"),
            ("facebookexternalhit", "FacebookBot"),
            ("twitterbot", "TwitterBot"),
            ("linkedinbot", "LinkedInBot"),
            ("discordbot", "DiscordBot"),
            ("telegrambot", "TelegramBot"),
            ("whatsapp", "WhatsApp"),
            ("uptimerobot", "UptimeRobot"),
            ("pingdom", "Pingdom"),
            ("better uptime", "BetterUptime"),
            ("curl/", "curl"),
            ("wget/", "Wget"),
            ("postmanruntime", "Postman"),
            ("python-requests", "PythonRequests"),
            ("python-urllib", "PythonUrllib"),
            ("go-http-client", "GoHttpClient"),
            ("headlesschrome", "HeadlessChrome"),
            ("k6/", "k6"),
            ("locust", "Locust"),
            ("sqlmap", "sqlmap"),
            ("nikto", "Nikto"),
        ];

        for (pattern, name) in known_bots {
            if lower.contains(pattern) {
                return Some(name.to_string());
            }
        }

        // Generic patterns
        if lower.contains("bot")
            || lower.contains("spider")
            || lower.contains("crawler")
            || lower.contains("scraper")
            || lower.contains("archiver")
        {
            // Try to extract word containing bot
            for part in raw.split(|c: char| c.is_whitespace() || c == ';' || c == '(' || c == ')') {
                let part_lower = part.to_lowercase();
                if part_lower.contains("bot") || part_lower.contains("crawler") {
                    let clean = part.split('/').next().unwrap_or(part);
                    if !clean.is_empty() {
                        return Some(clean.to_string());
                    }
                }
            }
            return Some("GenericBot".to_string());
        }

        None
    }

    fn detect_device(lower: &str) -> String {
        if lower.contains("ipad") || (lower.contains("android") && !lower.contains("mobile")) {
            "Tablet".to_string()
        } else if lower.contains("mobile") || lower.contains("iphone") || lower.contains("ipod") {
            "Mobile".to_string()
        } else {
            "Desktop".to_string()
        }
    }

    fn detect_os(lower: &str) -> String {
        if lower.contains("windows") {
            "Windows".to_string()
        } else if lower.contains("android") {
            "Android".to_string()
        } else if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
            "iOS".to_string()
        } else if lower.contains("mac os") || lower.contains("macintosh") {
            "macOS".to_string()
        } else if lower.contains("cros") {
            "ChromeOS".to_string()
        } else if lower.contains("linux") || lower.contains("x11") {
            "Linux".to_string()
        } else {
            "Other".to_string()
        }
    }

    fn detect_browser(lower: &str) -> String {
        if lower.contains("edg/") || lower.contains("edge/") {
            "Edge".to_string()
        } else if lower.contains("samsungbrowser/") {
            "Samsung Internet".to_string()
        } else if lower.contains("opr/") || lower.contains("opera/") {
            "Opera".to_string()
        } else if lower.contains("chrome/") || lower.contains("crios/") {
            "Chrome".to_string()
        } else if lower.contains("firefox/") || lower.contains("fxios/") {
            "Firefox".to_string()
        } else if lower.contains("safari/") && !lower.contains("chrome") {
            "Safari".to_string()
        } else {
            "Other".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chrome_desktop() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let info = UaParser::parse(Some(ua));
        assert_eq!(info.browser, "Chrome");
        assert_eq!(info.os, "Windows");
        assert_eq!(info.device, "Desktop");
        assert!(!info.is_bot);
        assert_eq!(info.bot_name, None);
    }

    #[test]
    fn test_parse_safari_iphone() {
        let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Mobile/15E148 Safari/604.1";
        let info = UaParser::parse(Some(ua));
        assert_eq!(info.browser, "Safari");
        assert_eq!(info.os, "iOS");
        assert_eq!(info.device, "Mobile");
        assert!(!info.is_bot);
    }

    #[test]
    fn test_parse_googlebot() {
        let ua = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
        let info = UaParser::parse(Some(ua));
        assert!(info.is_bot);
        assert_eq!(info.bot_name.as_deref(), Some("Googlebot"));
        assert_eq!(info.device, "Bot");
    }

    #[test]
    fn test_parse_curl() {
        let ua = "curl/7.88.1";
        let info = UaParser::parse(Some(ua));
        assert!(info.is_bot);
        assert_eq!(info.bot_name.as_deref(), Some("curl"));
        assert_eq!(info.device, "Bot");
    }
}
