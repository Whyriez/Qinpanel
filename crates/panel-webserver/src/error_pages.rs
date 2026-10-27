//! Default error pages shared by the panel and generated tenant vhosts.

pub const NOT_FOUND_HTML: &str = include_str!("../../../web/public/errors/404.html");

/// An inline response avoids depending on tenant docroot permissions or files.
pub fn render_default_not_found() -> String {
    let html = NOT_FOUND_HTML
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("")
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('$', "\\$");
    format!(
        r#"    # QonPanel default 404
    error_page 404 @qpanel_not_found;

    location @qpanel_not_found {{
        default_type text/html;
        charset utf-8;
        recursive_error_pages off;
        return 404 '{html}';
    }}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branded_page_keeps_404_status_without_external_assets_or_request_data() {
        let conf = render_default_not_found();
        assert!(conf.contains("error_page 404 @qpanel_not_found;"));
        assert!(conf.contains("return 404 '<!doctype html>"));
        assert!(!conf.contains("=200"));
        assert!(NOT_FOUND_HTML.contains("Qinpanel"));
        assert!(NOT_FOUND_HTML.contains("href=\"/\""));
        assert!(!NOT_FOUND_HTML.contains("nginx"));
        // The SVG namespace identifies the vector format and does not fetch a resource.
        let asset_urls = NOT_FOUND_HTML.replace("http://www.w3.org/2000/svg", "");
        assert!(!asset_urls.contains("http://") && !asset_urls.contains("https://"));
        assert!(!conf.contains("$request") && !conf.contains("$host"));
        // Keep the quoted parameter below Nginx's config parser buffer size.
        assert!(conf.lines().all(|line| line.len() < 4096));
    }
}
