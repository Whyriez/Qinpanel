//! Path confinement resolution and validation engine.
//!
//! Enforces strict boundary containment ensuring no operation can escape
//! the designated root directory via `..`, symlinks, absolute path injection,
//! null bytes, or unicode normalization tricks.

use crate::error::FsError;
use crate::policy::SymlinkPolicy;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

const MAX_PATH_LENGTH: usize = 4096;
const MAX_SYMLINK_HOPS: usize = 16;

/// A path checked against a root at resolution time, not an open filesystem capability.
/// Production operations run inside a jailed, unprivileged worker as a separate boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedPath {
    root: PathBuf,
    relative: PathBuf,
    resolved_absolute: PathBuf,
}

impl ConfinedPath {
    /// Returns the canonicalized root path of the jail.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the normalized relative path inside the jail.
    pub fn relative(&self) -> &Path {
        &self.relative
    }

    /// Returns the relative path formatted as a normalized POSIX-style string (e.g. "sites/site1/index.html").
    pub fn relative_posix(&self) -> String {
        let parts: Vec<String> = self
            .relative
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                _ => None,
            })
            .collect();
        parts.join("/")
    }

    /// Returns the absolute path on the host filesystem.
    pub fn resolved_absolute(&self) -> &Path {
        &self.resolved_absolute
    }

    /// Returns true if this path represents the root jail itself.
    pub fn is_root(&self) -> bool {
        self.relative.as_os_str().is_empty()
    }
}

/// Validates raw user input strings for security violations before filesystem operations.
pub fn validate_raw_path_str(raw: &str) -> Result<(), FsError> {
    if raw.contains('\0') {
        return Err(FsError::InvalidPath(
            "Path contains forbidden null byte (\\0)".to_string(),
        ));
    }

    if raw.len() > MAX_PATH_LENGTH {
        return Err(FsError::InvalidPath(format!(
            "Path exceeds maximum length of {MAX_PATH_LENGTH} characters"
        )));
    }

    // Reject ASCII control characters (0x00..=0x1F and 0x7F)
    if raw.chars().any(|c| (c as u32) < 0x20 || c == '\x7F') {
        return Err(FsError::InvalidPath(
            "Path contains illegal ASCII control characters".to_string(),
        ));
    }

    // Reject Unicode fullwidth slash tricks (U+FF0F fullwidth solidus, U+FF3C fullwidth reverse solidus)
    if raw.contains('\u{FF0F}') || raw.contains('\u{FF3C}') {
        return Err(FsError::InvalidPath(
            "Path contains illegal fullwidth slash character".to_string(),
        ));
    }

    Ok(())
}

/// Normalizes untrusted relative user path into safe path components.
///
/// Prevents `..` traversal above the virtual root:
/// If a `..` component would pop above the top of the virtual stack, an error is returned.
pub fn normalize_virtual_components(raw_path: &Path) -> Result<PathBuf, FsError> {
    let mut normalized_components = Vec::new();

    for comp in raw_path.components() {
        match comp {
            Component::Prefix(_) => {
                return Err(FsError::InvalidPath(
                    "Filesystem drive prefixes are forbidden".to_string(),
                ));
            }
            Component::RootDir => {
                // Leading root slashes are treated as the root of the jail
                normalized_components.clear();
            }
            Component::CurDir => {
                // Ignore '.'
            }
            Component::ParentDir => {
                if normalized_components.pop().is_none() {
                    return Err(FsError::PathTraversalAttempt {
                        path: raw_path.to_string_lossy().to_string(),
                        detail: "Attempted '..' traversal above confined root".to_string(),
                    });
                }
            }
            Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                // Reject Windows reserved device names if exact match
                let base_name = seg_str.split('.').next().unwrap_or("").to_ascii_uppercase();
                if matches!(
                    base_name.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                ) {
                    return Err(FsError::InvalidPath(format!(
                        "Reserved OS device name '{seg_str}' is forbidden"
                    )));
                }

                normalized_components.push(seg.to_os_string());
            }
        }
    }

    let mut result = PathBuf::new();
    for seg in normalized_components {
        result.push(seg);
    }
    Ok(result)
}

/// Resolves an untrusted user path against a confined root jail.
///
/// Ensures:
/// 1. The input string passes strict character and length checks.
/// 2. Normalized virtual path never escapes above the root.
/// 3. Step-by-step traversal enforces symlink policies (`DenyAll` or `ResolveBeneath`).
/// 4. Symlinks pointing outside root are rejected immediately.
/// 5. Post-resolution canonicalization verifies containment within `root`.
fn is_symlink_loop_io_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(40 | 62 | 1460 | 1921))
        || e.to_string().to_lowercase().contains("too many")
        || e.to_string().to_lowercase().contains("symbolic link")
}

pub fn resolve_confined_path(
    root: &Path,
    user_path: &Path,
    policy: SymlinkPolicy,
) -> Result<ConfinedPath, FsError> {
    let raw_str = user_path.to_string_lossy();
    validate_raw_path_str(&raw_str)?;

    let root_canonical = root.canonicalize().map_err(|e| {
        FsError::io(
            format!("Root directory {}", root.display()),
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Failed to canonicalize root {}: {e}", root.display()),
            ),
        )
    })?;

    if !root_canonical.is_dir() {
        return Err(FsError::NotADirectory {
            path: root_canonical.to_string_lossy().to_string(),
        });
    }

    let normalized_relative = normalize_virtual_components(user_path)?;

    // Step-by-step path traversal with symlink inspection
    let mut current_physical = root_canonical.clone();
    let mut visited_symlinks = HashSet::new();

    let components: Vec<_> = normalized_relative.components().collect();

    for (idx, comp) in components.iter().enumerate() {
        let seg = match comp {
            Component::Normal(s) => s,
            _ => continue,
        };

        let next_candidate = current_physical.join(seg);

        // Check symlink metadata without following links
        match std::fs::symlink_metadata(&next_candidate) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    match policy {
                        SymlinkPolicy::DenyAll => {
                            return Err(FsError::SymlinkForbidden {
                                path: next_candidate.to_string_lossy().to_string(),
                            });
                        }
                        SymlinkPolicy::ResolveBeneath => {
                            let mut current_symlink = next_candidate;
                            loop {
                                if visited_symlinks.contains(&current_symlink) {
                                    return Err(FsError::SymlinkLoop {
                                        path: current_symlink.to_string_lossy().to_string(),
                                    });
                                }
                                if visited_symlinks.len() >= MAX_SYMLINK_HOPS {
                                    return Err(FsError::SymlinkLoop {
                                        path: format!(
                                            "Too many symlink hops (exceeded {MAX_SYMLINK_HOPS}) resolving {}",
                                            current_symlink.display()
                                        ),
                                    });
                                }
                                visited_symlinks.insert(current_symlink.clone());

                                let link_target =
                                    std::fs::read_link(&current_symlink).map_err(|e| {
                                        FsError::io(
                                            current_symlink.to_string_lossy().to_string(),
                                            e,
                                        )
                                    })?;

                                // Resolve link target
                                let resolved_target = if link_target.is_absolute() {
                                    // Absolute link target on host
                                    match link_target.canonicalize() {
                                        Ok(canon) => {
                                            if !canon.starts_with(&root_canonical) {
                                                return Err(FsError::SymlinkEscape {
                                                    path: current_symlink
                                                        .to_string_lossy()
                                                        .to_string(),
                                                    target: canon.to_string_lossy().to_string(),
                                                });
                                            }
                                            canon
                                        }
                                        Err(e) if is_symlink_loop_io_error(&e) => {
                                            return Err(FsError::SymlinkLoop {
                                                path: current_symlink.to_string_lossy().to_string(),
                                            });
                                        }
                                        Err(_) => {
                                            return Err(FsError::SymlinkEscape {
                                                path: current_symlink.to_string_lossy().to_string(),
                                                target: link_target.to_string_lossy().to_string(),
                                            });
                                        }
                                    }
                                } else {
                                    // Relative link target: resolve relative to parent dir of current symlink
                                    let parent =
                                        current_symlink.parent().unwrap_or(&root_canonical);
                                    let candidate_target = parent.join(&link_target);
                                    match candidate_target.canonicalize() {
                                        Ok(canon) => {
                                            if !canon.starts_with(&root_canonical) {
                                                return Err(FsError::SymlinkEscape {
                                                    path: current_symlink
                                                        .to_string_lossy()
                                                        .to_string(),
                                                    target: canon.to_string_lossy().to_string(),
                                                });
                                            }
                                            canon
                                        }
                                        Err(e) if is_symlink_loop_io_error(&e) => {
                                            return Err(FsError::SymlinkLoop {
                                                path: current_symlink.to_string_lossy().to_string(),
                                            });
                                        }
                                        Err(_) => {
                                            // Target doesn't exist yet or broken link
                                            let normalized =
                                                normalize_virtual_components(&link_target)?;
                                            let joined = parent.join(normalized);
                                            if !joined.starts_with(&root_canonical) {
                                                return Err(FsError::SymlinkEscape {
                                                    path: current_symlink
                                                        .to_string_lossy()
                                                        .to_string(),
                                                    target: link_target
                                                        .to_string_lossy()
                                                        .to_string(),
                                                });
                                            }
                                            joined
                                        }
                                    }
                                };

                                match std::fs::symlink_metadata(&resolved_target) {
                                    Ok(m) if m.file_type().is_symlink() => {
                                        current_symlink = resolved_target;
                                        continue;
                                    }
                                    _ => {
                                        current_physical = resolved_target;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    current_physical = next_candidate;
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    // Path does not exist yet.
                    // If this is the last component, that's valid (e.g. creating a new file or directory).
                    // If not the last component, intermediate directory is missing.
                    if idx == components.len() - 1 {
                        current_physical = next_candidate;
                    } else {
                        // Intermediate component not found
                        current_physical = next_candidate;
                    }
                } else {
                    return Err(FsError::io(next_candidate.to_string_lossy().to_string(), e));
                }
            }
        }
    }

    // Defense-in-depth: If the resolved path exists on disk, canonicalize and verify boundary
    if current_physical.exists() {
        let canonical_check = match current_physical.canonicalize() {
            Ok(c) => c,
            Err(e) if is_symlink_loop_io_error(&e) => {
                return Err(FsError::SymlinkLoop {
                    path: current_physical.to_string_lossy().to_string(),
                });
            }
            Err(e) => {
                return Err(FsError::io(
                    current_physical.to_string_lossy().to_string(),
                    e,
                ));
            }
        };
        if !canonical_check.starts_with(&root_canonical) {
            return Err(FsError::PathTraversalAttempt {
                path: user_path.to_string_lossy().to_string(),
                detail: format!(
                    "Resolved canonical path '{}' escapes confined root '{}'",
                    canonical_check.display(),
                    root_canonical.display()
                ),
            });
        }
        current_physical = canonical_check;
    } else {
        // If it doesn't exist yet, verify parent directory boundary
        if let Some(parent) = current_physical.parent()
            && parent.exists()
        {
            let parent_canon = match parent.canonicalize() {
                Ok(c) => c,
                Err(e) if is_symlink_loop_io_error(&e) => {
                    return Err(FsError::SymlinkLoop {
                        path: parent.to_string_lossy().to_string(),
                    });
                }
                Err(e) => {
                    return Err(FsError::io(parent.to_string_lossy().to_string(), e));
                }
            };
            if !parent_canon.starts_with(&root_canonical) {
                return Err(FsError::PathTraversalAttempt {
                    path: user_path.to_string_lossy().to_string(),
                    detail: format!(
                        "Parent directory '{}' escapes confined root '{}'",
                        parent_canon.display(),
                        root_canonical.display()
                    ),
                });
            }
        }
    }

    Ok(ConfinedPath {
        root: root_canonical,
        relative: normalized_relative,
        resolved_absolute: current_physical,
    })
}
