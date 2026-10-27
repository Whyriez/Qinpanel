//! Object storage domain contracts. No provider calls, secrets, or host mutations.
pub mod capability;
pub mod identity;
pub mod network;
pub mod quota;

use thiserror::Error;

/// Engine selected for the first managed adapter. Not an installation directive.
pub const MANAGED_ENGINE_VERSION: &str = "4.50";
pub const MANAGED_ENGINE_COMMIT: &str = "ec8f9793502c59ce3381cc4151bfc0806afd92e8";
pub const STORAGE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StorageError {
    #[error("Identifier storage tidak valid.")]
    InvalidIdentity,
    #[error("Nama bucket tidak valid atau dicadangkan.")]
    InvalidBucketName,
    #[error("Object key tidak termasuk format portabel yang didukung.")]
    InvalidObjectKey,
    #[error("Konfigurasi koneksi storage tidak valid.")]
    InvalidConnection,
    #[error("Akses storage atau scope project tidak sesuai.")]
    Forbidden,
    #[error("Kemampuan atau izin penyedia belum terverifikasi.")]
    Unverified,
    #[error("Operasi tidak didukung adapter penyedia.")]
    Unsupported,
    #[error("Bukti koneksi sudah kedaluwarsa atau tidak cocok dengan revisinya.")]
    StaleEvidence,
    #[error("Endpoint atau hasil DNS tidak diizinkan.")]
    ForbiddenEndpoint,
    #[error("Data kuota atau pemakaian belum tersedia.")]
    UnknownUsage,
    #[error("Kuota atau ukuran upload terlampaui.")]
    QuotaExceeded,
    #[error("Adapter belum membuktikan kuota keras pada jalur S3 langsung.")]
    HardQuotaUnavailable,
}

#[cfg(test)]
mod tests;
