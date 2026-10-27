//! Python Runtime & Environment Manager: uv integration, virtualenv per site,
//! curated Python releases catalog (3.10..3.13), framework presets (Django, Flask, FastAPI),
//! and WSGI/ASGI server command builders.

use crate::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Default installation base directory for Python runtimes installed via `uv`.
pub const DEFAULT_PYTHON_BASE_DIR: &str = "/opt/panel/runtimes/python";

/// Default recommended Python version.
pub const DEFAULT_PYTHON_VERSION: &str = "3.12.9";

/// Default Unix domain socket directory for Python site processes.
pub const DEFAULT_PYTHON_SOCKET_DIR: &str = "/run/panel/python";

/// Default uv binary path.
pub const DEFAULT_UV_BIN: &str = "uv";
pub const PYTHON_DOWNLOADS_JSON: &str = "https://raw.githubusercontent.com/astral-sh/uv/main/crates/uv-python-managed/download-metadata.json";

pub fn validate_python_version(version: &str) -> Result<(), RuntimeError> {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts[0] != "3"
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_digit())
                || part.parse::<u16>().is_err()
        })
    {
        return Err(RuntimeError::PythonVersionNotFound(version.into()));
    }
    Ok(())
}

/// Curated Python version metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonVersionEntry {
    pub version: String,
    pub major_minor: String,
    pub release_date: Option<String>,
    pub is_default: bool,
    pub is_lts_or_stable: bool,
    pub description: String,
}

/// Installed Python runtime version discovered on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonInstalledVersion {
    pub version: String,
    pub install_path: String,
    pub bin_path: String,
    pub is_default: bool,
    pub created_at: Option<String>,
}

/// Framework preset for Python web applications.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonFrameworkPreset {
    Django,
    Flask,
    FastApi,
    Generic,
}

impl PythonFrameworkPreset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Django => "django",
            Self::Flask => "flask",
            Self::FastApi => "fastapi",
            Self::Generic => "generic",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "django" => Self::Django,
            "flask" => Self::Flask,
            "fastapi" => Self::FastApi,
            _ => Self::Generic,
        }
    }

    /// Returns default entrypoint and recommended server type for the preset.
    pub fn defaults(&self) -> (&'static str, PythonServerType) {
        match self {
            Self::Django => ("wsgi:application", PythonServerType::GunicornWsgi),
            Self::Flask => ("app:app", PythonServerType::GunicornWsgi),
            Self::FastApi => ("main:app", PythonServerType::UvicornAsgi),
            Self::Generic => ("app:app", PythonServerType::GunicornWsgi),
        }
    }
}

/// Server runtime type for hosting Python web applications.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonServerType {
    /// Gunicorn with standard sync/gevent/threads WSGI workers.
    GunicornWsgi,
    /// Uvicorn standalone ASGI server.
    UvicornAsgi,
    /// Gunicorn managing Uvicorn ASGI worker processes (`-k uvicorn.workers.UvicornWorker`).
    GunicornUvicornWorker,
}

impl PythonServerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GunicornWsgi => "gunicorn_wsgi",
            Self::UvicornAsgi => "uvicorn_asgi",
            Self::GunicornUvicornWorker => "gunicorn_uvicorn_worker",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "uvicorn" | "uvicorn_asgi" | "asgi" => Self::UvicornAsgi,
            "gunicorn_uvicorn" | "gunicorn_uvicorn_worker" => Self::GunicornUvicornWorker,
            _ => Self::GunicornWsgi,
        }
    }
}

/// Normalizes user version input (e.g., "python3.12", "v3.12.9", "3.12")
/// to a full canonical version string (e.g., "3.12.9").
pub fn normalize_python_version(input: &str) -> String {
    let trimmed = input.trim();
    let lower = trimmed.to_lowercase();
    let stripped = if let Some(rest) = lower.strip_prefix("python") {
        rest
    } else if let Some(rest) = lower.strip_prefix('v') {
        rest
    } else {
        trimmed
    };

    match stripped {
        "3.10" => "3.10.16".to_string(),
        "3.11" => "3.11.11".to_string(),
        "3.12" => "3.12.9".to_string(),
        "3.13" => "3.13.2".to_string(),
        other => other.to_string(),
    }
}

/// Returns the curated list of recommended official Python versions.
pub fn get_curated_python_catalog() -> Vec<PythonVersionEntry> {
    vec![
        PythonVersionEntry {
            version: "3.12.9".to_string(),
            major_minor: "3.12".to_string(),
            release_date: Some("2025-02-04".to_string()),
            is_default: true,
            is_lts_or_stable: true,
            description: "Python 3.12 (Rekomendasi / Default Qinpanel, stabil & cepat)".to_string(),
        },
        PythonVersionEntry {
            version: "3.13.2".to_string(),
            major_minor: "3.13".to_string(),
            release_date: Some("2025-02-04".to_string()),
            is_default: false,
            is_lts_or_stable: true,
            description: "Python 3.13 (Fitur terbaru, dukungan free-threaded awal)".to_string(),
        },
        PythonVersionEntry {
            version: "3.11.11".to_string(),
            major_minor: "3.11".to_string(),
            release_date: Some("2024-12-03".to_string()),
            is_default: false,
            is_lts_or_stable: true,
            description: "Python 3.11 (Sangat stabil, kompatibilitas package luas)".to_string(),
        },
        PythonVersionEntry {
            version: "3.10.16".to_string(),
            major_minor: "3.10".to_string(),
            release_date: Some("2024-12-03".to_string()),
            is_default: false,
            is_lts_or_stable: false,
            description: "Python 3.10 (Dukungan jangka panjang / legacy projects)".to_string(),
        },
    ]
}

/// Scans the Python runtime directory for installed versions.
pub fn python_binary_candidates(version_dir: &Path, children: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![version_dir.to_path_buf()];
    let version = version_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    for child in children {
        if child.parent() == Some(version_dir)
            && child
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("cpython-{version}-")))
        {
            roots.push(child.clone());
        }
    }
    roots
        .into_iter()
        .flat_map(|root| [root.join("bin/python3"), root.join("bin/python")])
        .collect()
}

pub fn list_installed_python_versions(
    base_dir: &Path,
) -> Result<Vec<PythonInstalledVersion>, RuntimeError> {
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(base_dir).map_err(|e| RuntimeError::Io(e.to_string()))?;
    let mut installed = Vec::new();

    for entry_res in entries {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let children: Vec<_> = fs::read_dir(&path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default();
        let Some(bin_path) = python_binary_candidates(&path, &children)
            .into_iter()
            .find(|candidate| candidate.is_file())
        else {
            continue;
        };

        let normalized = normalize_python_version(&dir_name);
        let is_default = normalized == DEFAULT_PYTHON_VERSION;

        installed.push(PythonInstalledVersion {
            version: normalized,
            install_path: path.to_string_lossy().to_string(),
            bin_path: bin_path.to_string_lossy().to_string(),
            is_default,
            created_at: None,
        });
    }

    installed.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(installed)
}

/// Calculates the default number of WSGI/ASGI workers based on CPU core count
/// and tenant plan limitations.
///
/// Formula: `(2 * cpu_cores) + 1`, capped by `max_plan_processes` if specified,
/// clamped between 1 and 16.
pub fn calculate_default_python_workers(cpu_cores: u32, max_plan_processes: Option<u32>) -> u32 {
    let cores = cpu_cores.max(1);
    let mut workers = (2 * cores) + 1;

    if let Some(limit) = max_plan_processes.filter(|&l| l > 0) {
        workers = workers.min(limit);
    }

    workers.clamp(1, 16)
}

/// Builds the `uv python install` CLI argument list.
pub fn build_uv_python_install_cmd(
    uv_bin: &str,
    version: &str,
    install_dir: Option<&str>,
) -> Vec<String> {
    let mut cmd = vec![
        uv_bin.to_string(),
        "python".to_string(),
        "install".to_string(),
        version.to_string(),
    ];
    if let Some(dir) = install_dir {
        cmd.push("--install-dir".to_string());
        cmd.push(dir.to_string());
    }
    cmd
}

/// Builds the `uv venv` CLI argument list to initialize a virtual environment.
pub fn build_uv_venv_cmd(uv_bin: &str, venv_dir: &str, python_bin: &str) -> Vec<String> {
    vec![
        uv_bin.to_string(),
        "venv".to_string(),
        venv_dir.to_string(),
        "--python".to_string(),
        python_bin.to_string(),
    ]
}

/// Builds the `uv pip install -r requirements.txt` CLI argument list for a venv.
pub fn build_uv_pip_install_requirements_cmd(
    uv_bin: &str,
    venv_dir: &str,
    req_file_path: &str,
) -> Vec<String> {
    let venv_python = PathBuf::from(venv_dir).join("bin").join("python");
    vec![
        uv_bin.to_string(),
        "pip".to_string(),
        "install".to_string(),
        "-r".to_string(),
        req_file_path.to_string(),
        "--python".to_string(),
        venv_python.to_string_lossy().to_string(),
    ]
}

/// Builds the `uv pip install -e <project_dir>` CLI argument list for a pyproject.toml app.
pub fn build_uv_pip_install_pyproject_cmd(
    uv_bin: &str,
    venv_dir: &str,
    project_dir: &str,
) -> Vec<String> {
    let venv_python = PathBuf::from(venv_dir).join("bin").join("python");
    vec![
        uv_bin.to_string(),
        "pip".to_string(),
        "install".to_string(),
        "-e".to_string(),
        project_dir.to_string(),
        "--python".to_string(),
        venv_python.to_string_lossy().to_string(),
    ]
}

/// Constructs the systemd ExecStart command for launching a Python site via Gunicorn or Uvicorn.
pub fn build_python_exec_start(
    venv_dir: &str,
    server_type: PythonServerType,
    entrypoint: &str,
    workers: u32,
    socket_path: &str,
) -> String {
    let venv_path = Path::new(venv_dir);
    match server_type {
        PythonServerType::GunicornWsgi => {
            let gunicorn_bin = venv_path.join("bin").join("gunicorn");
            format!(
                "{} --workers {} --bind unix:{} {}",
                gunicorn_bin.to_string_lossy().replace('\\', "/"),
                workers,
                socket_path.replace('\\', "/"),
                entrypoint
            )
        }
        PythonServerType::UvicornAsgi => {
            let uvicorn_bin = venv_path.join("bin").join("uvicorn");
            format!(
                "{} --workers {} --uds {} {}",
                uvicorn_bin.to_string_lossy().replace('\\', "/"),
                workers,
                socket_path.replace('\\', "/"),
                entrypoint
            )
        }
        PythonServerType::GunicornUvicornWorker => {
            let gunicorn_bin = venv_path.join("bin").join("gunicorn");
            format!(
                "{} -k uvicorn.workers.UvicornWorker --workers {} --bind unix:{} {}",
                gunicorn_bin.to_string_lossy().replace('\\', "/"),
                workers,
                socket_path.replace('\\', "/"),
                entrypoint
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_uv_managed_interpreter_inside_version_directory() {
        let root = tempdir().unwrap();
        let binary = root
            .path()
            .join("3.12.9/cpython-3.12.9-linux-x86_64-gnu/bin/python3");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"fixture interpreter").unwrap();
        let installed = list_installed_python_versions(root.path()).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].version, "3.12.9");
        assert_eq!(PathBuf::from(&installed[0].bin_path), binary);
    }

    #[test]
    fn test_normalize_python_version() {
        assert_eq!(normalize_python_version("3.12"), "3.12.9");
        assert_eq!(normalize_python_version("python3.11"), "3.11.11");
        assert_eq!(normalize_python_version("v3.10"), "3.10.16");
        assert_eq!(normalize_python_version("3.13"), "3.13.2");
        assert_eq!(normalize_python_version("3.12.4"), "3.12.4");
        assert_eq!(normalize_python_version("  v3.12.9  "), "3.12.9");
    }

    #[test]
    fn test_get_curated_catalog() {
        let catalog = get_curated_python_catalog();
        assert_eq!(catalog.len(), 4);
        assert!(
            catalog
                .iter()
                .any(|e| e.version == "3.12.9" && e.is_default)
        );
        assert!(catalog.iter().any(|e| e.version == "3.13.2"));
        assert!(catalog.iter().any(|e| e.version == "3.11.11"));
        assert!(catalog.iter().any(|e| e.version == "3.10.16"));
    }

    #[test]
    fn test_framework_presets() {
        assert_eq!(
            PythonFrameworkPreset::from_str_loose("django"),
            PythonFrameworkPreset::Django
        );
        assert_eq!(
            PythonFrameworkPreset::from_str_loose("FastAPI"),
            PythonFrameworkPreset::FastApi
        );
        assert_eq!(
            PythonFrameworkPreset::from_str_loose("unknown"),
            PythonFrameworkPreset::Generic
        );

        let (ep, srv) = PythonFrameworkPreset::Django.defaults();
        assert_eq!(ep, "wsgi:application");
        assert_eq!(srv, PythonServerType::GunicornWsgi);

        let (ep2, srv2) = PythonFrameworkPreset::FastApi.defaults();
        assert_eq!(ep2, "main:app");
        assert_eq!(srv2, PythonServerType::UvicornAsgi);
    }

    #[test]
    fn test_calculate_default_python_workers() {
        // 1 core -> 3 workers
        assert_eq!(calculate_default_python_workers(1, None), 3);
        // 2 cores -> 5 workers
        assert_eq!(calculate_default_python_workers(2, None), 5);
        // 4 cores with plan limit of 4
        assert_eq!(calculate_default_python_workers(4, Some(4)), 4);
        // 0 cores fallback
        assert_eq!(calculate_default_python_workers(0, None), 3);
    }

    #[test]
    fn test_command_builders() {
        let install_cmd = build_uv_python_install_cmd(
            "/usr/bin/uv",
            "3.12.9",
            Some("/opt/panel/runtimes/python/3.12.9"),
        );
        assert_eq!(
            install_cmd,
            vec![
                "/usr/bin/uv",
                "python",
                "install",
                "3.12.9",
                "--install-dir",
                "/opt/panel/runtimes/python/3.12.9"
            ]
        );

        let venv_cmd = build_uv_venv_cmd(
            "/usr/bin/uv",
            "/home/p_alice/sites/app/.venv",
            "/opt/panel/runtimes/python/3.12.9/bin/python3",
        );
        assert_eq!(
            venv_cmd,
            vec![
                "/usr/bin/uv",
                "venv",
                "/home/p_alice/sites/app/.venv",
                "--python",
                "/opt/panel/runtimes/python/3.12.9/bin/python3"
            ]
        );

        let req_cmd = build_uv_pip_install_requirements_cmd(
            "/usr/bin/uv",
            "/app/.venv",
            "/app/requirements.txt",
        );
        assert_eq!(req_cmd[0], "/usr/bin/uv");
        assert_eq!(req_cmd[1], "pip");
        assert_eq!(req_cmd[2], "install");
        assert_eq!(req_cmd[3], "-r");
        assert_eq!(req_cmd[4], "/app/requirements.txt");
    }

    #[test]
    fn test_build_python_exec_start() {
        let cmd = build_python_exec_start(
            "/home/p_alice/sites/example.com/.venv",
            PythonServerType::GunicornWsgi,
            "myproject.wsgi:application",
            3,
            "/run/panel/python/site-42.sock",
        );
        assert_eq!(
            cmd,
            "/home/p_alice/sites/example.com/.venv/bin/gunicorn --workers 3 --bind unix:/run/panel/python/site-42.sock myproject.wsgi:application"
        );

        let uvicorn_cmd = build_python_exec_start(
            "/home/p_alice/sites/example.com/.venv",
            PythonServerType::UvicornAsgi,
            "main:app",
            4,
            "/run/panel/python/site-42.sock",
        );
        assert_eq!(
            uvicorn_cmd,
            "/home/p_alice/sites/example.com/.venv/bin/uvicorn --workers 4 --uds /run/panel/python/site-42.sock main:app"
        );
    }

    #[test]
    fn test_list_installed_python_versions() {
        let dir = tempdir().unwrap();
        let py312_bin = dir.path().join("3.12.9").join("bin");
        fs::create_dir_all(&py312_bin).unwrap();
        fs::write(py312_bin.join("python3"), b"").unwrap();

        let list = list_installed_python_versions(dir.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, "3.12.9");
        assert!(list[0].is_default);
    }
}
