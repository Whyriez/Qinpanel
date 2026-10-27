//! Cron expression validation, systemd OnCalendar conversion, and human translation.

use crate::CoreError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedCron {
    pub minute: String,
    pub hour: String,
    pub day_of_month: String,
    pub month: String,
    pub day_of_week: String,
}

impl ParsedCron {
    pub fn raw_expression(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.minute, self.hour, self.day_of_month, self.month, self.day_of_week
        )
    }
}

/// Validates a standard 5-part cron expression: `min hour dom month dow`.
pub fn validate_cron_expression(expr: &str) -> Result<ParsedCron, CoreError> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(CoreError::Validation(format!(
            "Ekspresi cron harus memiliki 5 bagian (menit jam tgl bln hari), ditemukan {} bagian",
            parts.len()
        )));
    }

    validate_field(parts[0], 0, 59, "menit", &[])?;
    validate_field(parts[1], 0, 23, "jam", &[])?;
    validate_field(parts[2], 1, 31, "hari dalam bulan", &[])?;
    validate_field(
        parts[3],
        1,
        12,
        "bulan",
        &[
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ],
    )?;
    validate_field(
        parts[4],
        0,
        7,
        "hari dalam minggu",
        &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
    )?;

    Ok(ParsedCron {
        minute: parts[0].to_string(),
        hour: parts[1].to_string(),
        day_of_month: parts[2].to_string(),
        month: parts[3].to_string(),
        day_of_week: parts[4].to_string(),
    })
}

fn validate_field(
    field: &str,
    min: u32,
    max: u32,
    name: &str,
    names_allowlist: &[&str],
) -> Result<(), CoreError> {
    if field == "*" {
        return Ok(());
    }

    // Comma-separated list
    for sub in field.split(',') {
        let sub = sub.trim();
        if sub.is_empty() {
            return Err(CoreError::Validation(format!(
                "Bagian {} mengandung item kosong dalam daftar",
                name
            )));
        }

        if let Some((base, step_str)) = sub.split_once('/') {
            let step = step_str.parse::<u32>().map_err(|_| {
                CoreError::Validation(format!(
                    "Step '{}' pada {} bukan angka yang valid",
                    step_str, name
                ))
            })?;
            if step == 0 {
                return Err(CoreError::Validation(format!(
                    "Step pada {} tidak boleh bernilai 0",
                    name
                )));
            }
            if base != "*" {
                validate_single_or_range(base, min, max, name, names_allowlist)?;
            }
        } else {
            validate_single_or_range(sub, min, max, name, names_allowlist)?;
        }
    }

    Ok(())
}

fn validate_single_or_range(
    val: &str,
    min: u32,
    max: u32,
    name: &str,
    names_allowlist: &[&str],
) -> Result<(), CoreError> {
    if let Some((start_str, end_str)) = val.split_once('-') {
        let start = parse_val(start_str, min, max, name, names_allowlist)?;
        let end = parse_val(end_str, min, max, name, names_allowlist)?;
        if start > end {
            return Err(CoreError::Validation(format!(
                "Rentang {}-{} pada {} tidak valid (awal lebih besar dari akhir)",
                start_str, end_str, name
            )));
        }
    } else {
        parse_val(val, min, max, name, names_allowlist)?;
    }
    Ok(())
}

fn parse_val(
    val: &str,
    min: u32,
    max: u32,
    name: &str,
    names_allowlist: &[&str],
) -> Result<u32, CoreError> {
    let lower = val.to_lowercase();
    if let Some(pos) = names_allowlist.iter().position(|&n| n == lower) {
        return Ok(pos as u32 + 1);
    }

    match val.parse::<u32>() {
        Ok(num) => {
            if num < min || num > max {
                Err(CoreError::Validation(format!(
                    "Nilai {} pada {} di luar rentang yang diizinkan ({}-{})",
                    num, name, min, max
                )))
            } else {
                Ok(num)
            }
        }
        Err(_) => Err(CoreError::Validation(format!(
            "Nilai '{}' pada {} bukan angka atau alias yang valid",
            val, name
        ))),
    }
}

/// Converts standard 5-part cron syntax to systemd `OnCalendar` specification.
pub fn cron_to_systemd_calendar(expr: &str) -> Result<String, CoreError> {
    let parsed = validate_cron_expression(expr)?;

    // Day of week conversion
    let dow_part = match parsed.day_of_week.as_str() {
        "*" => None,
        "0" | "7" => Some("Sun"),
        "1" => Some("Mon"),
        "2" => Some("Tue"),
        "3" => Some("Wed"),
        "4" => Some("Thu"),
        "5" => Some("Fri"),
        "6" => Some("Sat"),
        other => {
            // If already string or complex, normalize
            let lower = other.to_lowercase();
            if lower.contains("mon") {
                Some("Mon")
            } else if lower.contains("tue") {
                Some("Tue")
            } else if lower.contains("wed") {
                Some("Wed")
            } else if lower.contains("thu") {
                Some("Thu")
            } else if lower.contains("fri") {
                Some("Fri")
            } else if lower.contains("sat") {
                Some("Sat")
            } else if lower.contains("sun") {
                Some("Sun")
            } else {
                None
            }
        }
    };

    // Date part: Year-Month-Day
    let month_str = if parsed.month == "*" {
        "*".to_string()
    } else if let Ok(m) = parsed.month.parse::<u32>() {
        format!("{:02}", m)
    } else {
        parsed.month.clone()
    };

    let dom_str = if parsed.day_of_month == "*" {
        "*".to_string()
    } else if let Ok(d) = parsed.day_of_month.parse::<u32>() {
        format!("{:02}", d)
    } else {
        parsed.day_of_month.clone()
    };

    let date_part = format!("*-{}-{}", month_str, dom_str);

    // Time part: Hour:Minute:00
    let hour_str = if parsed.hour == "*" {
        "*".to_string()
    } else if let Some(step) = parsed.hour.strip_prefix("*/") {
        format!("00/{}:00", step)
    } else if let Ok(h) = parsed.hour.parse::<u32>() {
        format!("{:02}", h)
    } else {
        parsed.hour.clone()
    };

    let min_str = if parsed.minute == "*" {
        "*".to_string()
    } else if let Some(step) = parsed.minute.strip_prefix("*/") {
        format!("00/{}:00", step)
    } else if let Ok(m) = parsed.minute.parse::<u32>() {
        format!("{:02}", m)
    } else {
        parsed.minute.clone()
    };

    let time_part = if min_str.contains('/') {
        if hour_str == "*" {
            format!("*:{}", min_str)
        } else {
            format!("{}:{}", hour_str, min_str)
        }
    } else {
        format!("{}:{}:00", hour_str, min_str)
    };

    if let Some(dow) = dow_part {
        Ok(format!("{} {} {}", dow, date_part, time_part))
    } else {
        Ok(format!("{} {}", date_part, time_part))
    }
}

/// Translates cron expression to Indonesian human-readable language.
pub fn cron_to_human_id(expr: &str) -> String {
    let parsed = match validate_cron_expression(expr) {
        Ok(p) => p,
        Err(_) => return "Jadwal kustom tidak valid".to_string(),
    };

    // Common presets
    if parsed.minute == "*"
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Setiap menit".to_string();
    }
    if parsed.minute.starts_with("*/")
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        let step = parsed.minute.trim_start_matches("*/");
        return format!("Setiap {} menit", step);
    }
    if parsed.minute == "0"
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Setiap jam pada menit ke-0".to_string();
    }
    if parsed.minute == "0"
        && parsed.hour.starts_with("*/")
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        let step = parsed.hour.trim_start_matches("*/");
        return format!("Setiap {} jam", step);
    }
    if parsed.minute == "0"
        && parsed.hour == "0"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Setiap hari tengah malam (00:00)".to_string();
    }
    if parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
        && let (Ok(h), Ok(m)) = (parsed.hour.parse::<u32>(), parsed.minute.parse::<u32>())
    {
        return format!("Setiap hari pada {:02}:{:02}", h, m);
    }
    if parsed.day_of_month == "*" && parsed.month == "*" {
        let dow_name = match parsed.day_of_week.as_str() {
            "0" | "7" | "sun" => "Minggu",
            "1" | "mon" => "Senin",
            "2" | "tue" => "Selasa",
            "3" | "wed" => "Rabu",
            "4" | "thu" => "Kamis",
            "5" | "fri" => "Jumat",
            "6" | "sat" => "Sabtu",
            _ => "hari tertentu",
        };
        if let (Ok(h), Ok(m)) = (parsed.hour.parse::<u32>(), parsed.minute.parse::<u32>()) {
            return format!("Setiap hari {} pada {:02}:{:02}", dow_name, h, m);
        }
    }
    if parsed.month == "*"
        && parsed.day_of_week == "*"
        && let (Ok(d), Ok(h), Ok(m)) = (
            parsed.day_of_month.parse::<u32>(),
            parsed.hour.parse::<u32>(),
            parsed.minute.parse::<u32>(),
        )
    {
        return format!("Setiap tanggal {} setiap bulan pada {:02}:{:02}", d, h, m);
    }

    format!(
        "Jadwal kustom: menit {}, jam {}, tanggal {}, bulan {}, hari {}",
        parsed.minute, parsed.hour, parsed.day_of_month, parsed.month, parsed.day_of_week
    )
}

/// Translates cron expression to English human-readable language.
pub fn cron_to_human_en(expr: &str) -> String {
    let parsed = match validate_cron_expression(expr) {
        Ok(p) => p,
        Err(_) => return "Invalid custom schedule".to_string(),
    };

    // Common presets
    if parsed.minute == "*"
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Every minute".to_string();
    }
    if parsed.minute.starts_with("*/")
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        let step = parsed.minute.trim_start_matches("*/");
        return format!("Every {} minutes", step);
    }
    if parsed.minute == "0"
        && parsed.hour == "*"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Every hour at minute 0".to_string();
    }
    if parsed.minute == "0"
        && parsed.hour.starts_with("*/")
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        let step = parsed.hour.trim_start_matches("*/");
        return format!("Every {} hours", step);
    }
    if parsed.minute == "0"
        && parsed.hour == "0"
        && parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
    {
        return "Every day at midnight (00:00)".to_string();
    }
    if parsed.day_of_month == "*"
        && parsed.month == "*"
        && parsed.day_of_week == "*"
        && let (Ok(h), Ok(m)) = (parsed.hour.parse::<u32>(), parsed.minute.parse::<u32>())
    {
        return format!("Every day at {:02}:{:02}", h, m);
    }
    if parsed.day_of_month == "*" && parsed.month == "*" {
        let dow_name = match parsed.day_of_week.as_str() {
            "0" | "7" | "sun" => "Sunday",
            "1" | "mon" => "Monday",
            "2" | "tue" => "Tuesday",
            "3" | "wed" => "Wednesday",
            "4" | "thu" => "Thursday",
            "5" | "fri" => "Friday",
            "6" | "sat" => "Saturday",
            _ => "specified day",
        };
        if let (Ok(h), Ok(m)) = (parsed.hour.parse::<u32>(), parsed.minute.parse::<u32>()) {
            return format!("Every {} at {:02}:{:02}", dow_name, h, m);
        }
    }
    if parsed.month == "*"
        && parsed.day_of_week == "*"
        && let (Ok(d), Ok(h), Ok(m)) = (
            parsed.day_of_month.parse::<u32>(),
            parsed.hour.parse::<u32>(),
            parsed.minute.parse::<u32>(),
        )
    {
        return format!("On day {} of every month at {:02}:{:02}", d, h, m);
    }

    format!(
        "Custom: min {}, hour {}, day {}, month {}, weekday {}",
        parsed.minute, parsed.hour, parsed.day_of_month, parsed.month, parsed.day_of_week
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_cron_expressions() {
        assert!(validate_cron_expression("* * * * *").is_ok());
        assert!(validate_cron_expression("*/5 * * * *").is_ok());
        assert!(validate_cron_expression("0 2 * * *").is_ok());
        assert!(validate_cron_expression("15,45 1-5 * * mon-fri").is_ok());
        assert!(validate_cron_expression("0 0 1 1 *").is_ok());
    }

    #[test]
    fn test_invalid_cron_expressions() {
        assert!(validate_cron_expression("").is_err());
        assert!(validate_cron_expression("* * * *").is_err()); // 4 parts
        assert!(validate_cron_expression("* * * * * *").is_err()); // 6 parts
        assert!(validate_cron_expression("60 * * * *").is_err()); // minute > 59
        assert!(validate_cron_expression("* 24 * * *").is_err()); // hour > 23
        assert!(validate_cron_expression("* * 32 * *").is_err()); // dom > 31
        assert!(validate_cron_expression("* * * 13 *").is_err()); // month > 12
        assert!(validate_cron_expression("* * * * 8").is_err()); // dow > 7
        assert!(validate_cron_expression("*/0 * * * *").is_err()); // step 0
    }

    #[test]
    fn test_cron_to_systemd_calendar() {
        assert_eq!(
            cron_to_systemd_calendar("* * * * *").unwrap(),
            "*-*-* *:*:00"
        );
        assert_eq!(
            cron_to_systemd_calendar("*/5 * * * *").unwrap(),
            "*-*-* *:00/5:00"
        );
        assert_eq!(
            cron_to_systemd_calendar("0 2 * * *").unwrap(),
            "*-*-* 02:00:00"
        );
        assert_eq!(
            cron_to_systemd_calendar("30 4 * * 0").unwrap(),
            "Sun *-*-* 04:30:00"
        );
        assert_eq!(
            cron_to_systemd_calendar("0 0 1 * *").unwrap(),
            "*-*-01 00:00:00"
        );
    }

    #[test]
    fn test_human_translations() {
        assert_eq!(cron_to_human_id("* * * * *"), "Setiap menit");
        assert_eq!(cron_to_human_id("*/15 * * * *"), "Setiap 15 menit");
        assert_eq!(cron_to_human_id("0 2 * * *"), "Setiap hari pada 02:00");
        assert_eq!(
            cron_to_human_id("0 0 * * 0"),
            "Setiap hari Minggu pada 00:00"
        );
        assert_eq!(
            cron_to_human_id("0 0 1 * *"),
            "Setiap tanggal 1 setiap bulan pada 00:00"
        );

        assert_eq!(cron_to_human_en("* * * * *"), "Every minute");
        assert_eq!(cron_to_human_en("*/15 * * * *"), "Every 15 minutes");
        assert_eq!(cron_to_human_en("0 2 * * *"), "Every day at 02:00");
        assert_eq!(cron_to_human_en("0 0 * * 0"), "Every Sunday at 00:00");
    }
}
