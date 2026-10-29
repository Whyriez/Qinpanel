#!/usr/bin/env bash
# ==============================================================================
# Qinpanel - Clean Uninstallation Script
# https://github.com/Whyriez/Qinpanel
# ==============================================================================

export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
set -euo pipefail

PURGE=false
ASSUME_YES=false

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_help() {
    cat << EOF
Penggunaan: $0 [OPSI]

Opsi yang tersedia:
  --purge       Hapus seluruh berkas basis data, konfigurasi, log, dan cadangan sistem
  -y, --yes     Konfirmasi pencopotan secara otomatis tanpa prompt interaktif
  -h, --help    Tampilkan pesan bantuan ini

Contoh:
  sudo ./uninstall.sh
  sudo ./uninstall.sh --purge -y
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --purge)
            PURGE=true
            shift
            ;;
        -y|--yes)
            ASSUME_YES=true
            shift
            ;;
        -h|--help)
            print_help
            exit 0
            ;;
        *)
            echo -e "${RED}[ERROR] Opsi tidak dikenal: $1${NC}" >&2
            print_help
            exit 1
            ;;
    esac
done

if [[ "$(id -u)" -ne 0 ]]; then
    echo -e "${RED}[ERROR] Skrip ini harus dijalankan dengan hak akses root (sudo).${NC}" >&2
    exit 1
fi

if [[ "$ASSUME_YES" == "false" ]]; then
    echo -e "${YELLOW}PERINGATAN: Skrip ini akan menghentikan dan mencopot layanan Qinpanel.${NC}"
    if [[ "$PURGE" == "true" ]]; then
        echo -e "${RED}Mode --purge aktif: SEMUA basis data (/var/lib/panel), cadangan, dan konfigurasi AKAN DIHAPUS.${NC}"
    else
        echo -e "Direktori data (/var/lib/panel) dan cadangan akan tetap dipertahankan."
    fi
    read -r -p "Apakah Anda yakin ingin melanjutkan pencopotan? (y/N): " CONFIRM
    if [[ "$CONFIRM" != "y" && "$CONFIRM" != "Y" && "$CONFIRM" != "yes" ]]; then
        echo "Pencopotan dibatalkan."
        exit 0
    fi
fi

echo -e "${BLUE}[1/4] Menghentikan dan menonaktifkan layanan systemd...${NC}"
# Keep tenant integration with retained data; detach it before purging its target.
proxy_backup="$(mktemp -d)"
trap 'rm -rf "$proxy_backup"' EXIT
proxy_files=(/etc/nginx/sites-available/nusa-panel.conf /etc/nginx/sites-enabled/nusa-panel.conf)
if [[ "$PURGE" == true ]]; then proxy_files+=(/etc/nginx/conf.d/00-nusa-panel-sites.conf); fi
for proxy_file in "${proxy_files[@]}"; do
    if [[ -e "$proxy_file" || -L "$proxy_file" ]]; then cp -a --parents "$proxy_file" "$proxy_backup"; fi
done
rm -f "${proxy_files[@]}"
if ! nginx -t || { systemctl is-active --quiet nginx && ! systemctl reload nginx; }; then
    for proxy_file in "${proxy_files[@]}"; do
        if [[ -e "$proxy_backup$proxy_file" || -L "$proxy_backup$proxy_file" ]]; then cp -a "$proxy_backup$proxy_file" "$proxy_file"; fi
    done
    echo '[ERROR] Konfigurasi Nginx tidak valid; pencopotan dibatalkan dan proxy dipulihkan.' >&2
    exit 1
fi
systemctl stop panel-api.service panel-agent.service panel-proc-hardening.service panel-db-web.service >/dev/null 2>&1 || true
systemctl stop qonpanel-server-backup.service >/dev/null 2>&1 || true
systemctl disable --now qonpanel-object-storage.service >/dev/null 2>&1 || true
systemctl stop panel-update.service >/dev/null 2>&1 || true
systemctl disable panel-api.service panel-agent.service panel-proc-hardening.service panel-db-web.service >/dev/null 2>&1 || true

echo -e "${BLUE}[2/4] Menghapus unit file systemd...${NC}"
rm -f /etc/systemd/system/panel-api.service /etc/systemd/system/panel-agent.service /etc/systemd/system/panel-proc-hardening.service
rm -f /etc/systemd/system/qonpanel-server-backup.service /usr/local/libexec/qonpanel-server-backup /usr/local/libexec/qonpanel-selinux /usr/local/libexec/qonpanel-mail-auth
rm -f /etc/systemd/system/panel-update.service /usr/local/libexec/panel-update-worker /usr/local/libexec/panel-bootstrap
rm -f /etc/systemd/system/panel-fpm@.service.d/10-panel-runtime-order.conf /etc/systemd/system/site-.service.d/10-panel-runtime-order.conf
rm -f /etc/systemd/system/nginx.service.d/10-panel-startup.conf
rm -f /etc/systemd/system/panel-db-web.service /etc/phpmyadmin/conf.d/zz-panel.php /usr/lib/tmpfiles.d/panel-db-web.conf
rm -rf /usr/share/panel/db-web /run/panel-db-web /var/lib/panel-db-web
systemctl daemon-reload

if [[ -d /var/lib/qonpanel-storage ]]; then
    echo 'Qinpanel object storage was stopped. Its engine, service, certificates, keys and object data are retained, including with --purge.'
fi

echo -e "${BLUE}[3/4] Menghapus biner Qinpanel dari /usr/local/bin...${NC}"
rm -f /usr/local/bin/panel-api /usr/local/bin/panel-agent /usr/local/bin/panelctl
rm -rf /run/panel

if [[ "$PURGE" == "true" ]]; then
    echo -e "${BLUE}[4/4] Membersihkan seluruh data dan pengguna sistem (--purge)...${NC}"
    rm -rf /var/lib/panel /var/backups/panel /etc/panel /var/log/panel
    userdel panel-api >/dev/null 2>&1 || true
    userdel panel-db-web >/dev/null 2>&1 || true
    groupdel panel-db-web >/dev/null 2>&1 || true
    groupdel panel >/dev/null 2>&1 || true
    echo -e "      ${GREEN}[OK] Seluruh data, konfigurasi, dan pengguna dibersihkan.${NC}"
else
    echo -e "${BLUE}[4/4] Mempertahankan berkas data (/var/lib/panel, /var/backups/panel)...${NC}"
fi

echo ""
echo "================================================================================"
echo -e "         ${GREEN}[SUKSES] Pencopotan pemasangan Qinpanel telah selesai.${NC}       "
echo "================================================================================"
