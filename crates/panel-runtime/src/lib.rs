//! `panel-runtime`: Runtime management for PHP-FPM, Node.js, Python, and Reverse Proxies.

pub mod error;
pub mod fpm;
pub mod node;
pub mod php;
pub mod python;

pub use error::RuntimeError;
pub use fpm::{
    DEFAULT_HARDENED_DISABLE_FUNCTIONS, DEFAULT_PHP_CONFIG_BASE, DEFAULT_PHP_LOG_BASE,
    DEFAULT_PHP_RUNTIME_BASE, DEFAULT_PING_PATH, DEFAULT_PM_MAX_CHILDREN, DEFAULT_PM_MAX_REQUESTS,
    DEFAULT_PM_PROCESS_IDLE_TIMEOUT_SECS, DEFAULT_REQUEST_SLOWLOG_TIMEOUT_SECS,
    DEFAULT_STATUS_PATH, PhpFpmMasterConfig, PhpFpmPoolConfig, validate_fpm_identifier,
};
pub use node::{
    DEFAULT_NODE_BASE_DIR, DEFAULT_NODE_INDEX_URL, DEFAULT_NODE_MIRROR, NodeInstallOptions,
    NodeInstalledVersion, NodeLtsStatus, NodeVersionEntry, NodeVersionSummary, compute_sha256,
    enable_corepack_shims, extract_node_tarball, find_shasum256_in_manifest,
    get_offline_node_catalog, list_installed_node_versions, normalize_node_version,
    parse_nodejs_index_json, raw_node_version_tag, verify_sha256,
};
pub use php::{
    CURATED_PHP_EXTENSIONS, PhpExtensionDefinition, PhpExtensionInfo, PhpVersion, PhpVersionInfo,
    find_curated_extension, is_sury_repo_installed,
};
pub use python::{
    DEFAULT_PYTHON_BASE_DIR, DEFAULT_PYTHON_SOCKET_DIR, DEFAULT_PYTHON_VERSION, DEFAULT_UV_BIN,
    PythonFrameworkPreset, PythonInstalledVersion, PythonServerType, PythonVersionEntry,
    build_python_exec_start, build_uv_pip_install_pyproject_cmd,
    build_uv_pip_install_requirements_cmd, build_uv_python_install_cmd, build_uv_venv_cmd,
    calculate_default_python_workers, get_curated_python_catalog, list_installed_python_versions,
    normalize_python_version,
};
