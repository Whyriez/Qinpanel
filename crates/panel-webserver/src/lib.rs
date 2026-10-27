//! `panel-webserver`: Nginx template renderer, atomic apply, and rollback mechanisms.

pub mod baseline;
pub mod error;
pub mod error_pages;
pub mod pipeline;
pub mod placeholder;
pub mod rules;
pub mod template;

pub use baseline::NginxBaselineConfig;
pub use error::WebserverError;
pub use pipeline::{ApplyResult, NginxApplyPipeline, ReloadDebouncer};
pub use rules::generate_site_rules_nginx_block;
pub use template::{
    DomainRedirectConfig, FastCgiCacheConfig, HeaderConfig, NginxTemplateRenderer, PhpPreset,
    PhpVhostContext, StaticVhostContext, render_placeholder_html, validate_php_vhost_inputs,
    validate_vhost_inputs,
};
