//! Fast, path-confined file searching engine for `panel-fs`.
//!
//! Complies with PRD P5-T8:
//! - Search by file name (substring, extension filtering)
//! - Search inside file text content (with line number and snippet preview)
//! - Enforces hard limits: maximum results cap and execution timeout
//! - Strictly confined within tenant root jail
//! - Skips `.panel-trash`, `.panel-upload-tmp`, binary files, and files exceeding size limit

use crate::error::FsError;
use crate::fs::ConfinedFs;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Default maximum number of results returned by search.
pub const DEFAULT_MAX_SEARCH_RESULTS: usize = 100;

/// Default maximum time allocated for a search before timing out.
pub const DEFAULT_SEARCH_TIMEOUT_SECS: u64 = 15;

/// Default maximum file size for searching text content (5 MB).
pub const DEFAULT_MAX_FILE_SIZE_FOR_CONTENT: u64 = 5 * 1024 * 1024;

/// Maximum number of content matches kept per file to prevent report bloat.
pub const MAX_MATCHES_PER_FILE: usize = 10;

/// Maximum snippet length in characters for a single matched line.
pub const MAX_SNIPPET_LENGTH: usize = 200;

/// Details of a line match within a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMatch {
    /// Line number (1-indexed).
    pub line_number: usize,
    /// Snippet of the matched line text.
    pub line_text: String,
}

/// A matched file or directory in search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResultItem {
    /// Relative path from the confined root.
    pub path: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last modified timestamp in epoch seconds.
    pub modified_at: Option<i64>,
    /// Content matches if content search was enabled and matches were found.
    pub matches: Vec<ContentMatch>,
}

/// Parameters for configuring a search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOptions {
    /// Query string to search for in filename or content.
    pub query: String,
    /// If true, searches inside file contents in addition to filenames.
    pub search_content: bool,
    /// If true, performs case-sensitive matching.
    pub case_sensitive: bool,
    /// Optional extension filter (e.g. "php", "js", "html" without dot).
    pub extension: Option<String>,
    /// Maximum number of search results to return.
    pub max_results: usize,
    /// Maximum execution time before interrupting search.
    pub timeout: Duration,
    /// Maximum size of files allowed for content inspection.
    pub max_file_size_for_content: u64,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            search_content: false,
            case_sensitive: false,
            extension: None,
            max_results: DEFAULT_MAX_SEARCH_RESULTS,
            timeout: Duration::from_secs(DEFAULT_SEARCH_TIMEOUT_SECS),
            max_file_size_for_content: DEFAULT_MAX_FILE_SIZE_FOR_CONTENT,
        }
    }
}

/// Comprehensive report returned after executing a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchReport {
    /// Matching items discovered during search.
    pub items: Vec<SearchResultItem>,
    /// Total number of files and directories scanned.
    pub total_scanned: usize,
    /// Indicates whether search stopped early because `max_results` was reached.
    pub limit_reached: bool,
    /// Indicates whether search stopped early because `timeout` expired.
    pub timed_out: bool,
    /// Total elapsed time in milliseconds.
    pub duration_ms: u64,
}

/// Checks if a file appears to be binary by checking the first 512 bytes for null bytes.
fn is_binary_file(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return true;
    };
    let mut buffer = [0u8; 512];
    let Ok(n) = file.read(&mut buffer) else {
        return true;
    };
    buffer[..n].contains(&0)
}

/// Scans a file's content line-by-line for matches within constraints.
fn scan_file_content(
    fs: &ConfinedFs,
    rel_path: &Path,
    file_size: u64,
    options: &SearchOptions,
    query_lower: &str,
) -> Vec<ContentMatch> {
    if !options.search_content || file_size == 0 || file_size > options.max_file_size_for_content {
        return Vec::new();
    }

    let Ok(file_confined) = fs.resolve(rel_path) else {
        return Vec::new();
    };

    let abs_file = file_confined.resolved_absolute();
    if is_binary_file(abs_file) {
        return Vec::new();
    }

    let Ok(file) = File::open(abs_file) else {
        return Vec::new();
    };

    let reader = BufReader::new(file);
    let mut matches = Vec::new();

    for (idx, line_res) in reader.lines().enumerate() {
        let Ok(line) = line_res else {
            continue;
        };

        let line_match = if options.case_sensitive {
            line.contains(&options.query)
        } else {
            line.to_lowercase().contains(query_lower)
        };

        if line_match {
            let snippet = if line.chars().count() > MAX_SNIPPET_LENGTH {
                line.chars().take(MAX_SNIPPET_LENGTH).collect::<String>()
            } else {
                line
            };

            matches.push(ContentMatch {
                line_number: idx + 1,
                line_text: snippet.trim().to_string(),
            });

            if matches.len() >= MAX_MATCHES_PER_FILE {
                break;
            }
        }
    }

    matches
}

/// Checks whether a file name matches an optional extension filter.
fn matches_file_extension(name: &str, ext_filter: Option<&str>) -> bool {
    match ext_filter {
        Some(wanted_ext) => {
            let file_ext = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            file_ext.as_deref() == Some(wanted_ext)
        }
        None => true,
    }
}

/// Executes a search within `fs` rooted at `start_path`.
pub fn search_confined(
    fs: &ConfinedFs,
    start_path: &Path,
    options: &SearchOptions,
) -> Result<SearchReport, FsError> {
    let start_instant = Instant::now();
    let query = &options.query;
    if query.trim().is_empty() {
        return Ok(SearchReport {
            items: Vec::new(),
            total_scanned: 0,
            limit_reached: false,
            timed_out: false,
            duration_ms: start_instant.elapsed().as_millis() as u64,
        });
    }

    let query_lower = query.to_lowercase();
    let ext_filter = options
        .extension
        .as_ref()
        .map(|e| e.trim_start_matches('.').to_lowercase());

    // Resolve starting directory within confinement
    let start_confined = fs.resolve(start_path)?;
    let start_abs = start_confined.resolved_absolute();

    if !start_abs.exists() {
        return Err(FsError::NotFound {
            path: start_path.display().to_string(),
        });
    }

    let mut items = Vec::new();
    let mut total_scanned = 0usize;
    let mut limit_reached = false;
    let mut timed_out = false;

    // Queue of directories to scan (stores relative virtual paths)
    let mut dir_queue: VecDeque<PathBuf> = VecDeque::new();
    let root_rel = if start_path == Path::new("") || start_path == Path::new(".") {
        PathBuf::new()
    } else {
        start_path.to_path_buf()
    };
    dir_queue.push_back(root_rel);

    while let Some(current_dir) = dir_queue.pop_front() {
        if start_instant.elapsed() >= options.timeout {
            timed_out = true;
            break;
        }

        let list_res = match fs.list_dir(
            &current_dir,
            crate::entry::ListOptions {
                limit: 10_000,
                offset: 0,
                sort_by: crate::entry::SortBy::Name,
                sort_order: crate::entry::SortOrder::Asc,
                show_hidden: true,
                search_query: None,
                extension: None,
            },
        ) {
            Ok(res) => res,
            Err(_) => continue, // Skip unreadable directories
        };

        for entry in list_res.entries {
            total_scanned += 1;

            if start_instant.elapsed() >= options.timeout {
                timed_out = true;
                break;
            }

            let entry_name = &entry.name;
            if entry_name == ".panel-trash" || entry_name == ".panel-upload-tmp" {
                continue;
            }

            let entry_rel_path = if current_dir.as_os_str().is_empty() {
                PathBuf::from(entry_name)
            } else {
                current_dir.join(entry_name)
            };
            let entry_rel_str = entry_rel_path.to_string_lossy().replace('\\', "/");

            let name_matches = if options.case_sensitive {
                entry_name.contains(query)
            } else {
                entry_name.to_lowercase().contains(&query_lower)
            };

            if entry.is_dir {
                if name_matches && ext_filter.is_none() {
                    items.push(SearchResultItem {
                        path: entry_rel_str.clone(),
                        is_dir: true,
                        size_bytes: entry.size_bytes,
                        modified_at: entry.modified_at,
                        matches: Vec::new(),
                    });

                    if items.len() >= options.max_results {
                        limit_reached = true;
                        break;
                    }
                }
                dir_queue.push_back(entry_rel_path);
            } else {
                if !matches_file_extension(entry_name, ext_filter.as_deref()) {
                    continue;
                }

                let content_matches =
                    scan_file_content(fs, &entry_rel_path, entry.size_bytes, options, &query_lower);

                if name_matches || !content_matches.is_empty() {
                    items.push(SearchResultItem {
                        path: entry_rel_str,
                        is_dir: false,
                        size_bytes: entry.size_bytes,
                        modified_at: entry.modified_at,
                        matches: content_matches,
                    });

                    if items.len() >= options.max_results {
                        limit_reached = true;
                        break;
                    }
                }
            }
        }

        if limit_reached || timed_out {
            break;
        }
    }

    Ok(SearchReport {
        items,
        total_scanned,
        limit_reached,
        timed_out,
        duration_ms: start_instant.elapsed().as_millis() as u64,
    })
}
