//! PHP-FPM Slowlog Parser.
//!
//! Parses slow execution trace blocks from per-tenant slowlog files
//! (`/var/log/panel/<user>/php-fpm-<ver>-slow.log`).
//!
//! Extracts:
//! - Pool name and process ID (PID)
//! - Script filename entry point
//! - Call stack frames (address, function signature, file path, line number)
//! - Top offending function and execution timestamp

use crate::error::AnalyticsError;
use crate::models::{PhpSlowlogEntry, PhpSlowlogFrame};
use chrono::{DateTime, NaiveDateTime, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Parser for PHP-FPM slowlog files.
pub struct PhpSlowlogParser;

impl PhpSlowlogParser {
    /// Parses all complete slowlog entries from a string content.
    pub fn parse_str(content: &str) -> Vec<PhpSlowlogEntry> {
        let mut entries = Vec::new();
        let mut current_lines = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Check if this line starts a new slowlog block: e.g. "[11-Oct-2026 14:23:45]  [pool p_alice-8.3] pid 12345"
            if is_header_line(trimmed) && !current_lines.is_empty() {
                if let Some(entry) = Self::parse_block(&current_lines) {
                    entries.push(entry);
                }
                current_lines.clear();
            }

            current_lines.push(trimmed.to_string());
        }

        if let Some(entry) = Self::parse_block(&current_lines) {
            entries.push(entry);
        }

        entries
    }

    /// Parses all entries from a slowlog file path.
    pub fn parse_file(path: &Path) -> Result<Vec<PhpSlowlogEntry>, AnalyticsError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            AnalyticsError::System(format!(
                "Failed to read PHP slowlog file {}: {e}",
                path.display()
            ))
        })?;
        Ok(Self::parse_str(&content))
    }

    /// Incrementally parses new slowlog entries from a file starting at a given byte offset.
    /// Returns the parsed entries and the new byte offset.
    pub fn parse_from_offset(
        path: &Path,
        offset: u64,
    ) -> Result<(Vec<PhpSlowlogEntry>, u64), AnalyticsError> {
        if !path.exists() {
            return Ok((Vec::new(), offset));
        }

        let mut file = File::open(path).map_err(|e| {
            AnalyticsError::System(format!(
                "Failed to open slowlog file {}: {e}",
                path.display()
            ))
        })?;

        let meta = file.metadata().map_err(|e| {
            AnalyticsError::System(format!(
                "Failed to get metadata for {}: {e}",
                path.display()
            ))
        })?;

        // If file was rotated (size smaller than offset), reset to 0
        let seek_pos = if meta.len() < offset { 0 } else { offset };
        file.seek(SeekFrom::Start(seek_pos)).map_err(|e| {
            AnalyticsError::System(format!(
                "Failed to seek slowlog file {}: {e}",
                path.display()
            ))
        })?;

        let mut reader = BufReader::new(file);
        let mut lines = Vec::new();
        let mut line = String::new();

        while reader
            .read_line(&mut line)
            .map_err(|e| AnalyticsError::System(format!("Failed to read line: {e}")))?
            > 0
        {
            lines.push(line.clone());
            line.clear();
        }

        let full_text = lines.concat();
        let entries = Self::parse_str(&full_text);
        let new_offset = meta.len();

        Ok((entries, new_offset))
    }

    /// Parses a single slice of lines belonging to one slowlog record block.
    fn parse_block(lines: &[String]) -> Option<PhpSlowlogEntry> {
        if lines.is_empty() {
            return None;
        }

        let header = &lines[0];
        let (timestamp, pool, pid) = parse_header(header)?;

        let mut script_filename = String::new();
        let mut frames = Vec::new();
        let mut frame_index = 0;

        for line in &lines[1..] {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(script) = trimmed.strip_prefix("script_filename = ") {
                script_filename = script.trim().to_string();
                continue;
            }

            // Stack frame line
            if let Some(frame) = parse_frame(trimmed, frame_index) {
                frames.push(frame);
                frame_index += 1;
            }
        }

        let top_function = frames.first().map(|f| f.function.clone());
        let tenant_user = parse_tenant_user_from_pool(&pool);

        Some(PhpSlowlogEntry {
            id: None,
            timestamp,
            pool,
            pid,
            script_filename,
            top_function,
            frames,
            duration_secs: Some(5.0), // Standard DEFAULT_REQUEST_SLOWLOG_TIMEOUT_SECS
            site_id: None,
            tenant_user,
        })
    }
}

/// Checks if a line matches the slowlog header format: `[<date>]  [pool <name>] pid <num>`.
fn is_header_line(line: &str) -> bool {
    line.starts_with('[') && line.contains("[pool ") && line.contains(" pid ")
}

/// Parses the slowlog header line.
fn parse_header(line: &str) -> Option<(i64, String, u32)> {
    // 1. Timestamp in first brackets: "[11-Oct-2026 14:23:45]"
    let close_first = line.find(']')?;
    let date_str = &line[1..close_first].trim();
    let timestamp = parse_slowlog_date(date_str).unwrap_or_else(|| Utc::now().timestamp());

    // 2. Pool in second brackets: "[pool p_alice-8.3]"
    let pool_marker = "[pool ";
    let pool_start = line.find(pool_marker)? + pool_marker.len();
    let pool_end = line[pool_start..].find(']')? + pool_start;
    let pool = line[pool_start..pool_end].trim().to_string();

    // 3. PID: "pid 12345"
    let pid_marker = "pid ";
    let pid_start = line.find(pid_marker)? + pid_marker.len();
    let pid_str = line[pid_start..].trim();
    let pid = pid_str
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    Some((timestamp, pool, pid))
}

/// Parses slowlog date format `11-Oct-2026 14:23:45` or fallback formats.
fn parse_slowlog_date(s: &str) -> Option<i64> {
    // PHP-FPM standard: "11-Oct-2026 14:23:45"
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%d-%b-%Y %H:%M:%S") {
        return Some(dt.and_utc().timestamp());
    }
    // Alternative ISO: "2026-10-11 14:23:45"
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp());
    }
    // RFC 3339
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    None
}

/// Parses a stack trace frame line:
/// e.g. `[0x00007f3b8c2d1234] sleep() /home/p_alice/sites/heavy.php:14`
/// or `[0x00007f3b8c2d5678] App\Controller\IndexController->run() /app/Index.php:25`
/// or `sleep() /home/user/script.php:10`
fn parse_frame(line: &str, frame_index: usize) -> Option<PhpSlowlogFrame> {
    let mut rest = line;
    let mut address = None;

    // Optional address bracket: "[0x00007f3b8c2d1234]"
    if let Some(close_idx) = rest.find(']').filter(|_| rest.starts_with('[')) {
        let addr_str = rest[1..close_idx].trim();
        if addr_str.starts_with("0x") || addr_str.chars().all(|c| c.is_ascii_hexdigit()) {
            address = Some(addr_str.to_string());
            rest = rest[close_idx + 1..].trim();
        }
    }

    if rest.is_empty() || rest.starts_with("+++") {
        // e.g. "+++ dump failed"
        return None;
    }

    // Next, function name and file:line are separated by whitespace.
    // Last token is typically "/path/to/file.php:line"
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let (function, file, line_num) = if tokens.len() == 1 {
        (tokens[0].to_string(), String::new(), 0)
    } else {
        let last = tokens[tokens.len() - 1];
        if let Some((f_path, l_str)) = last.rsplit_once(':') {
            if let Ok(l) = l_str.parse::<u32>() {
                let func = tokens[..tokens.len() - 1].join(" ");
                (func, f_path.to_string(), l)
            } else {
                let func = tokens[..tokens.len() - 1].join(" ");
                (func, last.to_string(), 0)
            }
        } else {
            let func = tokens[..tokens.len() - 1].join(" ");
            (func, last.to_string(), 0)
        }
    };

    Some(PhpSlowlogFrame {
        frame_index,
        function,
        file,
        line: line_num,
        address,
    })
}

/// Extracts username from pool name like "p_alice-8.3" -> "p_alice".
fn parse_tenant_user_from_pool(pool: &str) -> Option<String> {
    let (user, _ver) = pool.rsplit_once('-')?;
    if user.is_empty() {
        None
    } else {
        Some(user.to_string())
    }
}
