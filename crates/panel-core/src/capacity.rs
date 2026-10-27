//! Measured capacity and deterministic, read-only package sizing.
//! No credentials, provider calls, paths supplied by an actor, or configuration writes.
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MIB: u64 = 1_048_576;
pub const SNAPSHOT_TTL_SECONDS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CapacityMetric {
    pub value: Option<u64>,
    pub sources: Vec<String>,
    pub unavailable_reason: Option<String>,
}

impl CapacityMetric {
    pub fn measured(value: u64, sources: Vec<String>) -> Self {
        Self {
            value: Some(value),
            sources,
            unavailable_reason: None,
        }
    }

    pub fn unavailable(reason: &str, sources: Vec<String>) -> Self {
        Self {
            value: None,
            sources,
            unavailable_reason: Some(reason.into()),
        }
    }

    fn required(&self) -> Result<u64, CapacityError> {
        match (self.value, &self.unavailable_reason) {
            (Some(value), None) if !self.sources.is_empty() => Ok(value),
            _ => Err(CapacityError::Unavailable),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CapacitySnapshot {
    pub schema_version: u32,
    pub observed_at: u64,
    /// Parent of panel-u<uid>.slice, not the agent service's private cgroup.
    pub scope: String,
    pub cpu_quota_percent: CapacityMetric,
    pub memory_limit_bytes: CapacityMetric,
    pub memory_available_bytes: CapacityMetric,
    pub disk_total_bytes: CapacityMetric,
    pub disk_available_bytes: CapacityMetric,
    pub inodes_total: CapacityMetric,
    pub inodes_available: CapacityMetric,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PackageUse {
    Personal,
    MultiProject,
    MultiTenant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageDemand {
    pub usage: PackageUse,
    pub projects_per_tenant: u32,
    /// Number of equal shares to size, not a count inferred by the model.
    pub tenant_slots: u32,
}

impl PackageDemand {
    pub fn validate(&self) -> Result<(), CapacityError> {
        if !(1..=256).contains(&self.projects_per_tenant)
            || !(1..=10_000).contains(&self.tenant_slots)
            || match self.usage {
                PackageUse::Personal => self.projects_per_tenant != 1 || self.tenant_slots != 1,
                PackageUse::MultiProject => self.projects_per_tenant < 2 || self.tenant_slots != 1,
                PackageUse::MultiTenant => self.tenant_slots < 2,
            }
        {
            Err(CapacityError::InvalidDemand)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackagePreset {
    pub rules_version: u32,
    pub observed_at: u64,
    pub expires_at: u64,
    pub demand: PackageDemand,
    pub cpu_quota_percent: u64,
    pub memory_limit_bytes: u64,
    pub memory_high_bytes: u64,
    pub disk_limit_bytes: u64,
    pub inode_limit: u64,
    pub tasks_max: u64,
    pub io_weight: u64,
    pub max_sites: u32,
    pub max_upload_bytes: u64,
    pub memory_headroom_bytes: u64,
    pub cpu_headroom_percent: u64,
    pub disk_headroom_bytes: u64,
    /// Informational ceiling for NEW capacity, never an apply authorization.
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum CapacityError {
    #[error("Data kapasitas belum tersedia atau belum terverifikasi.")]
    Unavailable,
    #[error("Data kapasitas tidak konsisten.")]
    InvalidMeasurement,
    #[error("Snapshot kapasitas sudah kedaluwarsa atau waktunya tidak valid.")]
    Stale,
    #[error("Kebutuhan paket tidak valid.")]
    InvalidDemand,
    #[error("Kapasitas tersisa tidak cukup untuk profil konservatif.")]
    Insufficient,
}

/// Sizes additional capacity using currently available resources. This does not
/// reclaim existing tenant usage or account for future plan reservations.
pub fn recommend_package(
    snapshot: &CapacitySnapshot,
    demand: PackageDemand,
    now: u64,
) -> Result<PackagePreset, CapacityError> {
    if snapshot.schema_version != 1 || snapshot.scope != "panel.slice" {
        return Err(CapacityError::InvalidMeasurement);
    }
    if snapshot.observed_at == 0
        || now < snapshot.observed_at
        || now - snapshot.observed_at > SNAPSHOT_TTL_SECONDS
    {
        return Err(CapacityError::Stale);
    }
    demand.validate()?;
    let cpu = snapshot.cpu_quota_percent.required()?;
    let memory_total = snapshot.memory_limit_bytes.required()?;
    let memory_free = snapshot.memory_available_bytes.required()?;
    let disk_total = snapshot.disk_total_bytes.required()?;
    let disk_free = snapshot.disk_available_bytes.required()?;
    let inodes_total = snapshot.inodes_total.required()?;
    let inodes_free = snapshot.inodes_available.required()?;
    if memory_total == 0
        || disk_total == 0
        || inodes_total == 0
        || memory_free > memory_total
        || disk_free > disk_total
        || inodes_free > inodes_total
    {
        return Err(CapacityError::InvalidMeasurement);
    }
    // Available memory already excludes current usage. Only EXTRA headroom is
    // subtracted here; shared database/mail usage must not be subtracted twice.
    let memory_headroom = (memory_total / 10).max(256 * MIB);
    let disk_headroom = (disk_total / 10).max(512 * MIB);
    let inode_headroom = (inodes_total / 10).max(1000);
    let slots = u64::from(demand.tenant_slots);
    let memory = memory_free.saturating_sub(memory_headroom) / slots / MIB * MIB;
    let disk = disk_free.saturating_sub(disk_headroom) / slots / MIB * MIB;
    let inodes = inodes_free.saturating_sub(inode_headroom) / slots;
    let cpu_headroom = (cpu / 10).max(10);
    let cpu = cpu.saturating_sub(cpu_headroom) / slots;
    if memory < 256 * MIB || disk < 512 * MIB || inodes < 1000 || cpu < 10 {
        return Err(CapacityError::Insufficient);
    }
    let expires_at = snapshot
        .observed_at
        .checked_add(SNAPSHOT_TTL_SECONDS)
        .ok_or(CapacityError::InvalidMeasurement)?;
    let mut notices = snapshot.notices.clone();
    notices.push("instantaneous_new_capacity_only".into());
    notices.push("not_a_reservation_or_apply_authorization".into());
    if demand.usage == PackageUse::MultiTenant {
        notices.push("equal_shares_without_existing_plan_commitments".into());
    }
    Ok(PackagePreset {
        rules_version: 1,
        observed_at: snapshot.observed_at,
        expires_at,
        demand: demand.clone(),
        cpu_quota_percent: cpu,
        memory_limit_bytes: memory,
        memory_high_bytes: memory / 4 * 3,
        disk_limit_bytes: disk,
        inode_limit: inodes,
        tasks_max: (64 + u64::from(demand.projects_per_tenant) * 32).min(1024),
        io_weight: 100,
        max_sites: demand.projects_per_tenant,
        max_upload_bytes: (disk / 10).min(100 * MIB),
        memory_headroom_bytes: memory_headroom,
        cpu_headroom_percent: cpu_headroom,
        disk_headroom_bytes: disk_headroom,
        notices,
    })
}

/// Linux cpulist format. Reject duplicate/overlapping ranges and unbounded input.
pub fn parse_cpu_list(text: &str) -> Result<Vec<(u32, u32)>, CapacityError> {
    if text.len() > 65_536 || text.trim().is_empty() {
        return Err(CapacityError::InvalidMeasurement);
    }
    let mut ranges = Vec::new();
    for part in text.trim().split(',') {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (start, end),
            None => (part, part),
        };
        let start = start
            .parse::<u32>()
            .map_err(|_| CapacityError::InvalidMeasurement)?;
        let end = end
            .parse::<u32>()
            .map_err(|_| CapacityError::InvalidMeasurement)?;
        if start > end || end > 65_535 {
            return Err(CapacityError::InvalidMeasurement);
        }
        ranges.push((start, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|r| r[0].1 >= r[1].0) {
        return Err(CapacityError::InvalidMeasurement);
    }
    Ok(ranges)
}

pub fn intersect_cpu_count(left: &[(u32, u32)], right: &[(u32, u32)]) -> u64 {
    let (mut i, mut j, mut count) = (0, 0, 0u64);
    while let (Some(&(a, b)), Some(&(c, d))) = (left.get(i), right.get(j)) {
        let lo = a.max(c);
        let hi = b.min(d);
        if lo <= hi {
            count = count.saturating_add(u64::from(hi) - u64::from(lo) + 1);
        }
        if b < d {
            i += 1;
        } else {
            j += 1;
        }
    }
    count
}

pub fn parse_cpu_max(text: &str) -> Result<Option<u64>, CapacityError> {
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 2 {
        return Err(CapacityError::InvalidMeasurement);
    }
    let period = fields[1]
        .parse::<u64>()
        .map_err(|_| CapacityError::InvalidMeasurement)?;
    if period == 0 {
        return Err(CapacityError::InvalidMeasurement);
    }
    if fields[0] == "max" {
        return Ok(None);
    }
    let quota = fields[0]
        .parse::<u64>()
        .map_err(|_| CapacityError::InvalidMeasurement)?;
    if quota == 0 {
        return Err(CapacityError::InvalidMeasurement);
    }
    quota
        .checked_mul(100)
        .map(|value| Some(value / period))
        .ok_or(CapacityError::InvalidMeasurement)
}

pub fn parse_limit(text: &str) -> Result<Option<u64>, CapacityError> {
    if text.trim() == "max" {
        Ok(None)
    } else {
        text.trim()
            .parse()
            .map(Some)
            .map_err(|_| CapacityError::InvalidMeasurement)
    }
}

pub fn parse_memory(text: &str) -> Result<(u64, u64), CapacityError> {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let slot = match fields.first().copied() {
            Some("MemTotal:") => &mut total,
            Some("MemAvailable:") => &mut available,
            _ => continue,
        };
        if slot.is_some() || fields.len() != 3 || fields[2] != "kB" {
            return Err(CapacityError::InvalidMeasurement);
        }
        *slot = Some(
            fields[1]
                .parse::<u64>()
                .ok()
                .and_then(|v| v.checked_mul(1024))
                .ok_or(CapacityError::InvalidMeasurement)?,
        );
    }
    match (total, available) {
        (Some(total), Some(available)) if total > 0 && available <= total => Ok((total, available)),
        _ => Err(CapacityError::InvalidMeasurement),
    }
}

#[cfg(test)]
mod tests;
