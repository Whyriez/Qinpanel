//! Only reconstructed, bounded diagnostics may leave the privileged RPC boundary.
use crate::agent_client::AgentClientError;

#[derive(Debug)]
pub struct PublicationError(String);

impl PublicationError {
    pub fn unavailable() -> Self {
        Self("Publikasi Tunnel belum tersedia. Periksa koneksi panel-api ke panel-agent dan paket yang terpasang.".into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }

    pub fn from_agent(error: AgentClientError) -> Self {
        let AgentClientError::AgentError(message) = error else {
            return Self::unavailable();
        };
        let message = message
            .strip_prefix("Invalid input: ")
            .or_else(|| message.strip_prefix("Security violation: "))
            .unwrap_or(&message);
        Self(safe_message(message))
    }
}

fn safe_message(message: &str) -> String {
    if message.len() <= 512
        && let Some(detail) = message
            .strip_prefix("Cloudflare menolak permintaan (HTTP ")
            .and_then(|v| {
                v.strip_suffix("). Periksa izin token, akses akun/domain, dan konflik DNS.")
            })
        && let Some((status, codes)) = detail.split_once(", kode ")
        && status.len() == 3
        && let Ok(status) = status.parse::<u16>()
        && (100..=599).contains(&status)
        && let Ok(codes) = serde_json::from_str::<Vec<u32>>(codes)
        && codes.len() <= 8
        && codes.iter().all(|code| *code <= 1_000_000)
    {
        let guidance = match status {
            401 | 403 => {
                "Periksa token management: Account Cloudflare Tunnel Edit, Zone DNS Edit dan Zone Read untuk akun serta zona storage. DNS Settings Edit bukan izin mengubah record DNS."
            }
            404 => {
                "Periksa akun, zona dan Tunnel yang dipilih; resource mungkin tidak tersedia atau tidak dapat diakses oleh token."
            }
            429 => "Batas permintaan Cloudflare tercapai. Tunggu sebelum mengirim aktivasi baru.",
            _ => "Periksa izin akun/zona, batas provider dan konflik DNS sebelum mencoba kembali.",
        };
        return format!(
            "Cloudflare menolak publikasi storage (HTTP {status}, kode {codes:?}). {guidance}"
        );
    }
    let safe = match message {
        "Hubungkan akun Cloudflare terlebih dahulu." => {
            "Akun management Cloudflare belum terhubung pada agent. Hubungkan melalui pengaturan Cloudflare panel."
        }
        "Siapkan origin storage dan konektor Tunnel lokal yang aktif terlebih dahulu." => {
            "Pemeriksaan ulang origin storage atau konektor Tunnel lokal belum siap. Pastikan konektor untuk Tunnel yang dipilih aktif pada server ini."
        }
        "Hubungkan tunnel ke panel terlebih dahulu." => {
            "Tunnel yang dipilih belum dikelola panel. Hubungkan Tunnel tersebut melalui pengaturan Cloudflare."
        }
        "Domain bukan milik akun Cloudflare yang terhubung." => {
            "Zona storage tidak berada dalam akun Cloudflare yang terhubung. Periksa pemilihan akun dan zona."
        }
        "Hostname sudah memiliki record DNS. Hapus atau ubah record tersebut secara eksplisit sebelum menerbitkan tunnel." => {
            "Hostname storage sudah mempunyai record DNS. Tinjau record untuk hostname tersebut; panel tidak menimpanya otomatis."
        }
        "Hostname sudah dipakai konfigurasi tunnel di luar panel." => {
            "Hostname storage sudah dipakai rute Tunnel di luar panel. Tinjau rute tersebut sebelum mengirim aktivasi baru."
        }
        "Hostname sudah diterbitkan. Lepaskan dahulu untuk mengganti rute."
        | "Hostname sudah memiliki rute berbeda; konfigurasi dipertahankan." => {
            "Hostname storage sudah terdaftar pada rute berbeda di inventaris panel. Tinjau rute yang tercatat sebelum menggantinya."
        }
        "Rute atau DNS storage berubah di luar panel. Periksa konflik sebelum melanjutkan." => {
            "Rute atau DNS storage yang tercatat di panel berbeda dari Cloudflare. Menghapus hostname di Cloudflare saja tidak memperbarui inventaris panel; tinjau rute yang tercatat."
        }
        "Catch-all tunnel harus menolak hostname tidak dikenal, misalnya http_status:404. Konfigurasi tunnel dipertahankan; jangan arahkan catch-all ke origin web." => {
            "Catch-all Tunnel harus menolak hostname tidak dikenal, misalnya http_status:404. Konfigurasi yang ada dipertahankan."
        }
        "Tunnel memuat wildcard atau rute tanpa hostname yang dapat mengekspos origin. Gunakan rute hostname eksplisit; konfigurasi tidak diubah." => {
            "Tunnel memuat wildcard atau rute tanpa hostname eksplisit. Perbaiki batas rute sebelum menerbitkan storage; konfigurasi dipertahankan."
        }
        "Cloudflare tidak dapat dihubungi. Periksa koneksi server lalu coba lagi."
        | "Permintaan Cloudflare gagal atau melewati batas waktu. Muat ulang status sebelum mencoba kembali." => {
            "Agent belum dapat menghubungi API Cloudflare atau permintaan melewati batas waktu. Muat ulang status sebelum mencoba kembali."
        }
        "DNS gagal dibuat dan konfigurasi tunnel gagal dipulihkan. Periksa rute tunnel sebelum mencoba kembali." => {
            "Pembuatan DNS dan pemulihan konfigurasi Tunnel gagal. Periksa rute Cloudflare sebelum mengirim operasi berikutnya."
        }
        "Inventaris gagal disimpan; perubahan DNS dan tunnel telah dibatalkan." => {
            "Inventaris panel gagal disimpan; perubahan DNS dan Tunnel telah dibatalkan."
        }
        "Inventaris gagal disimpan dan rollback tidak lengkap. Periksa DNS serta rute tunnel sebelum mencoba kembali." => {
            "Inventaris panel gagal disimpan dan rollback belum lengkap. Periksa DNS serta rute Tunnel sebelum mencoba kembali."
        }
        _ => {
            "Agent menolak publikasi Tunnel; rincian aman belum tersedia. Periksa kesesuaian paket API/agent dan status Cloudflare. Jangan menghapus DNS atau membuat token baru hanya berdasarkan pesan ini."
        }
    };
    safe.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_denial_keeps_only_bounded_status_and_numeric_codes() {
        let error = PublicationError::from_agent(AgentClientError::AgentError("Invalid input: Cloudflare menolak permintaan (HTTP 403, kode [10000, 9109]). Periksa izin token, akses akun/domain, dan konflik DNS.".into()));
        assert!(error.message().contains("HTTP 403, kode [10000, 9109]"));
        assert!(error.message().contains("Zone DNS Edit dan Zone Read"));
        for status in [404, 429, 503] {
            let message = format!(
                "Cloudflare menolak permintaan (HTTP {status}, kode []). Periksa izin token, akses akun/domain, dan konflik DNS."
            );
            assert!(safe_message(&message).contains(&format!("HTTP {status}")));
        }
    }

    #[test]
    fn unknown_augmented_or_malformed_errors_never_expose_raw_details() {
        let valid = "Cloudflare menolak permintaan (HTTP 403, kode [10000]). Periksa izin token, akses akun/domain, dan konflik DNS.";
        for message in [
            "PRIVATE_SECRET".into(),
            format!("{valid} PRIVATE_SECRET"),
            format!("PRIVATE_SECRET {valid}"),
            valid.replace("[10000]", "[\"PRIVATE_SECRET\"]"),
            valid.replace("[10000]", "[999999999]"),
            valid.replace("[10000]", "[-1]"),
            valid.replace("[10000]", "[1,2,3,4,5,6,7,8,9]"),
            valid.replace("403", "999"),
            "PRIVATE_SECRET".repeat(100),
        ] {
            let error = PublicationError::from_agent(AgentClientError::AgentError(message));
            assert!(!error.message().contains("PRIVATE_SECRET"));
            assert!(!error.message().contains("HTTP"));
        }
    }

    #[test]
    fn exact_conflicts_and_connector_errors_remain_distinct() {
        for (message, expected) in [
            (
                "Hostname sudah memiliki record DNS. Hapus atau ubah record tersebut secara eksplisit sebelum menerbitkan tunnel.",
                "record DNS",
            ),
            (
                "Hostname sudah dipakai konfigurasi tunnel di luar panel.",
                "rute Tunnel di luar panel",
            ),
            (
                "Siapkan origin storage dan konektor Tunnel lokal yang aktif terlebih dahulu.",
                "konektor Tunnel lokal",
            ),
            (
                "Hubungkan tunnel ke panel terlebih dahulu.",
                "belum dikelola panel",
            ),
            (
                "Rute atau DNS storage berubah di luar panel. Periksa konflik sebelum melanjutkan.",
                "inventaris panel",
            ),
        ] {
            assert!(safe_message(message).contains(expected));
            let error = PublicationError::from_agent(AgentClientError::AgentError(format!(
                "Security violation: {message}"
            )));
            assert!(error.message().contains(expected));
            assert!(!safe_message(&format!("{message} PRIVATE_SECRET")).contains("PRIVATE_SECRET"));
        }
    }
}
