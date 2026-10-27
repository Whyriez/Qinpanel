//! Standalone welcome page used by every default site runtime.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::LazyLock;

const BRAND_MARK: &str = include_str!("../../../web/public/brand/qonpanel-mark.svg");
const TEMPLATE: &str = include_str!("placeholder.html");
const FONT_LICENSE: &str = include_str!("../../../web/public/fonts/plusjakartasans-OFL.txt");
static FONT: LazyLock<String> = LazyLock::new(|| {
    STANDARD.encode(include_bytes!(
        "../../../web/public/fonts/plus-jakarta-sans-latin.woff2"
    ))
});

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Render text fields without interpreting their contents as markup or template syntax.
pub fn render_welcome_html(domain: &str, runtime: &str, instruction: &str) -> String {
    // Expand only tokens in the source, never tokens introduced by caller data.
    TEMPLATE
        .split("{{")
        .map(|part| match part.split_once("}}") {
            Some(("domain", rest)) => format!("{}{rest}", escape_html(domain)),
            Some(("runtime", rest)) => format!("{}{rest}", escape_html(runtime)),
            Some(("instruction", rest)) => format!("{}{rest}", escape_html(instruction)),
            Some(("brand_mark", rest)) => format!(
                "{}{rest}",
                BRAND_MARK.replacen("<svg ", "<svg class=\"mark\" aria-hidden=\"true\" ", 1)
            ),
            Some(("font", rest)) => format!("{}{rest}", FONT.as_str()),
            Some(("font_license", rest)) => format!("{FONT_LICENSE}{rest}"),
            _ => part.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_text_and_does_not_expand_tokens_from_input() {
        let html = render_welcome_html(
            "<script>alert('x')</script>{{runtime}}",
            "PHP & <8.4>",
            "Replace \"index.html\"",
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;{{runtime}}"));
        assert!(html.contains("PHP &amp; &lt;8.4&gt;"));
        assert!(html.contains("Replace &quot;index.html&quot;"));
    }

    #[test]
    fn welcome_page_is_standalone_and_accessible() {
        let html = render_welcome_html("contoh.id", "HTML statis", "Ganti index.html.");
        assert!(html.contains("<html lang=\"id\">"));
        assert!(html.contains("<h1>contoh.id</h1>"));
        assert!(html.contains("<ol>"));
        assert!(html.contains("prefers-color-scheme: dark"));
        assert!(html.contains("data:font/woff2;base64,"));
        assert!(!html.contains("src=\"http") && !html.contains("href=\"http"));
        assert!(!html.contains("url('http"));
        assert!(html.contains("SIL OPEN FONT LICENSE"));
        assert!(!html.contains("<script") && !html.contains("{{"));
        assert!(!html.contains("gradient(") && !html.contains("backdrop-filter"));
    }
}
