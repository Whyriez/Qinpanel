//! Export a real rendered page for local visual review without running panel services.
fn main() {
    let domain = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "contoh.id".to_string());
    println!("{}", panel_webserver::render_placeholder_html(&domain));
}
