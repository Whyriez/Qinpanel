<picture>
  <source media="(prefers-color-scheme: dark)" srcset="web/public/brand/qinpanel-wordmark-dark.svg">
  <img src="web/public/brand/qinpanel-wordmark-light.svg" alt="Qinpanel" width="244" height="48">
</picture>

# Qinpanel

Panel hosting Rust dengan API non-root, agent privileged dan isolasi per tenant.
Versi development saat ini **0.1.0**.

Panel menggunakan Nginx pada `http://SERVER_IP:17890`, diteruskan ke
`panel-api` di `127.0.0.1:7700`. Port 80/443 tetap untuk website tenant.

## Instalasi

Build dijalankan di GitHub Actions, termasuk ketika repository privat. Distribusi
publik memakai Supabase Storage melalui `download.arelqon.com`, dengan paket
API/agent/CLI/frontend lengkap dan checksum. Setelah bucket, domain dan publikasi
rilis diaktifkan, bootstrap tersedia tanpa token GitHub. Jalankan satu perintah
berikut dari terminal server sebagai root:

```bash
bash <(curl -fsSL https://download.arelqon.com/qonpanel/install.sh || wget -qO- https://download.arelqon.com/qonpanel/install.sh) --unattended
```

Gunakan Bash karena bootstrap memakai fitur Bash. Jika unduhan melalui curl
gagal, perintah mencoba wget, lalu menjalankan installer.

Jika paket release sudah diunduh dan diekstrak:

```bash
sudo bash install.sh --unattended
# Port publik alternatif:
sudo bash install.sh --unattended --port 19090
```

Instalasi baru membuat username dan password administrator otomatis. Kredensial
ditampilkan sekali setelah pemeriksaan kesehatan berhasil. Simpan kredensial,
buka URL panel, lalu ubah melalui **Profil & keamanan** dan aktifkan 2FA.

## Pengelolaan akun

Pemulihan akun tersedia melalui SSH:

```bash
sudo panelctl admin list
sudo panelctl admin reset-password --username NAMA
panelctl admin reset-password --help
panelctl admin --help
panelctl --help
```

## Pembaruan dan backup

Pembaruan tersedia di **Pengaturan server > Pembaruan**. Instalasi existing
dapat memakai bootstrap dengan `--upgrade`:

```bash
bash <(curl -fsSL https://download.arelqon.com/qonpanel/install.sh || wget -qO- https://download.arelqon.com/qonpanel/install.sh) --unattended --upgrade
```

Upgrade mempertahankan akun, website, data dan runtime tenant. Push source saja
tidak memperbarui arsip distribusi; gunakan paket release lengkap yang
memuat API, agent, CLI dan frontend dari versi yang sesuai.

Instalasi lama yang masih memakai GitHub perlu satu kali upgrade melalui bootstrap
domain sebelum repository dijadikan privat. Paket baru mematok sumber distribusi
yang sama untuk pemeriksaan versi dan unduhan updater.

Backup dan pindah server tersedia di **Pengaturan server > Backup dan migrasi**.
Arsip terenkripsi mencakup tenant, website, database lokal, email dan konfigurasi
untuk pemulihan pada instalasi baru. Backup, pemeriksaan arsip, dan restore
muncul di **Tugas** dengan progres tahapan. Website, database, email dan panel
dijeda sementara selama snapshot agar data konsisten; pemantauan tersambung
kembali setelah layanan aktif. Bantuan CLI: `panelctl backup --help`.

## Asisten paket hosting dengan AI

Atur provider Gemini atau OpenAI, model, API key dan batas permintaan di
**Pengaturan server > Integrasi AI**. Key disimpan terenkripsi; fitur nonaktif
secara default dan hanya dapat dikelola administrator.

Di **Paket hosting**, pilih **Bantu atur dengan AI**, jelaskan kebutuhan dan tinjau
preview. AI memilih profil; server menghitung resource dari kapasitas terverifikasi.
Gunakan **Isi formulir paket baru**, lalu periksa dan simpan secara manual.
Analisis tidak mengubah tenant atau paket existing. Penggunaan API mengikuti
tagihan provider dan memerlukan persetujuan pengiriman data pada formulir.

## Object storage S3

Buka **Storage S3** pada menu utama atau tab **Storage** di detail project.
Administrator menambahkan koneksi AWS S3, Cloudflare R2, Supabase S3, atau
endpoint S3-compatible HTTPS publik, memilih tenant dan bucket, lalu menguji
koneksi. Gunakan access key dan secret key S3 khusus bucket dari penyedia;
key distribusi rilis Supabase tidak digunakan untuk koneksi project.

Tenant dapat menelusuri folder/prefix, mengunggah file hingga **32 MiB**,
mengunduh dan menghapus object dengan konfirmasi. Binding project menyediakan
contoh konfigurasi SDK tanpa menampilkan secret atau mengubah `.env` otomatis.
Pembuatan bucket memerlukan izin provider dan konfirmasi admin. Melepas koneksi
tidak menghapus bucket atau object di penyedia.

AWS S3/R2 menggunakan conditional PUT untuk mencegah overwrite secara default.
Provider lain memerlukan persetujuan overwrite sebelum upload. Key terenkripsi
di server; izin, pemakaian, kuota, CORS dan tagihan mengikuti penyedia. Pada
Supabase, generated S3 access keys melewati RLS dan dapat mengakses semua bucket:
gunakan proyek terpisah atau kredensial session/RLS sesuai kebutuhan isolasi.

Storage Qinpanel menyediakan engine S3 lokal melalui **Aktifkan storage Qinpanel**.
Untuk sertifikatnya, pilih **Generate otomatis · Let's Encrypt** setelah mengatur
DNS-01 Cloudflare di Pengaturan server → DNS dan Cloudflare. Metode ini memakai
domain storage terpisah, mendukung akses melalui IP publik maupun Tunnel, dan
menjadwalkan perpanjangan tanpa menyalin PEM/private key. Sertifikat sendiri
tetap dapat dipasang. Kegagalan penerbitan/pemasangan menjeda perpanjangan dan
memerlukan pemeriksaan serta Generate ulang. Kuota keras per bucket dan
multipart/resume belum tersedia.
Backup panel mencakup metadata koneksi, bukan isi object eksternal. Instalasi
existing menerima fitur melalui upgrade paket API/agent/frontend yang sesuai;
compile/test Rust dan pengujian provider nyata tetap harus lolos sebelum rilis.

## Data dan pengembangan

Untuk penerbit: buat bucket publik `qonpanel-releases` pada proyek Supabase
`rrewfkbaipgbvakaekcy` dengan batas file yang cukup untuk tarball. Simpan
`SUPABASE_SERVICE_ROLE_KEY` sebagai GitHub Actions **Secret**, lalu aktifkan
proxy domain pada `distribution/wrangler.toml` menggunakan Cloudflare Workers
(`npx wrangler deploy --config distribution/wrangler.toml`). Key tidak diperlukan
oleh Worker atau installer. Repository variable `QONPANEL_RELEASE_BASE_URL`,
`SUPABASE_URL` dan `SUPABASE_BUCKET` dapat mengganti nilai bawaan.

Jalankan workflow **Qinpanel release** dengan opsi publish atau push tag versi.
Paket diverifikasi dari Supabase dan domain sebelum `latest.json` dipromosikan.
Build baru memakai `qinpanel-vVERSI-TARGET.tar.gz`. Asset GitHub Release hanya
memuat paket utama kedua arsitektur dan `SHA256SUMS`. Supabase juga hanya
menerima paket `qinpanel-*`; build baru tidak membuat paket `nusa-panel-*`
atau `qonpanel-*`. Artifact build lama dengan nama tersebut ditolak sebelum
upload. Installer/updater tetap dapat membaca rilis historis yang sudah ada.
Versi yang telah dipublikasikan tidak dapat ditimpa dengan konten berbeda;
gunakan versi baru. Bucket hanya untuk artifact distribusi: unduhan publik,
sedangkan penulisan melalui secret CI. Source Rust dan dokumen internal tidak
dipublikasikan; JavaScript frontend dan script installer memang bagian paket.

SQLite panel berada di `/var/lib/panel/panel.db`, operasi/job dan audit di
`/var/lib/panel/panel.operations.db`, serta metrik di `/var/lib/panel/analytics.db`.
Operations dan analitik memiliki writer tunggal serta reader terpisah. Upgrade
mengimpor job/audit lama sekali; pemulihan pasangan database dilakukan offline.

Development dan integration test sistem dilakukan di VM Ubuntu 24.04.
Jangan menjalankan `panel-agent` di laptop atau host pengembang karena agent
mengubah konfigurasi sistem.
