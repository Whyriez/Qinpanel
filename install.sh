#!/usr/bin/env bash
# Qinpanel production install/upgrade. Never run on a development host.
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
# Keep installer subprocess messages in English without changing the server locale.
export LC_ALL=C
set -Eeuo pipefail
# Never trace generated credentials, including when invoked with bash -x.
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT_EXPLICIT=false
[[ -z "${PANEL_PUBLIC_PORT:-}" ]] || PORT_EXPLICIT=true
PANEL_PUBLIC_PORT="${PANEL_PUBLIC_PORT:-17890}"
UNATTENDED=false
FORCE_OS=false
SKIP_FIREWALL=false
UPGRADE=false
BACKUP_DIR=""
STAGING_DIR=""
STATE_SAVED=false
STATE_CHECKED=false
TRANSACTION=false
API_WAS_ACTIVE=false
AGENT_WAS_ACTIVE=false
NGINX_WAS_ACTIVE=false
API_WAS_ENABLED=false
AGENT_WAS_ENABLED=false
NGINX_WAS_ENABLED=false
ADMIN_USERNAME=""
ADMIN_PASSWORD=""
RELEASE_VERSION=""
PACKAGE_FAMILY=deb
RESTORE_DIGEST=""
MANAGED_FILES=(
    /usr/local/bin/panel-api /usr/local/bin/panel-agent /usr/local/bin/panelctl
    /etc/systemd/system/panel-api.service /etc/systemd/system/panel-agent.service
    /etc/systemd/system/panel-proc-hardening.service
    /etc/systemd/system/panel-fpm@.service.d/10-panel-runtime-order.conf
    /etc/systemd/system/site-.service.d/10-panel-runtime-order.conf
    /etc/nginx/sites-available/nusa-panel.conf /etc/nginx/sites-enabled/nusa-panel.conf
    /etc/nginx/conf.d/00-qpanel-reject-unknown.conf
    /etc/nginx/conf.d/default.conf /etc/nginx/sites-enabled/default
    /etc/nginx/nginx.conf
    /usr/lib/openssh/sftp-server
    /etc/panel/public-port
    /etc/panel/installed-release
    /etc/systemd/system/panel-update.service
    /etc/systemd/system/qonpanel-server-backup.service /usr/local/libexec/qonpanel-server-backup
    /usr/local/libexec/panel-update-worker /usr/local/libexec/panel-bootstrap
    /usr/local/libexec/qonpanel-selinux
    /usr/local/libexec/qonpanel-object-storage
    /usr/local/libexec/qonpanel-mail-auth
)

fail() { echo "[ERROR] $*" >&2; return 1; }

install_terminal_is_tty() { [[ -t 1 ]]; }

setup_installation_style() {
    INSTALL_BOLD=''; INSTALL_ACCENT=''; INSTALL_SUCCESS=''; INSTALL_DIM=''; INSTALL_RESET=''
    if install_terminal_is_tty && [[ "${NO_COLOR+x}" != x && "${TERM:-dumb}" != dumb ]]; then
        INSTALL_BOLD=$'\033[1m'; INSTALL_ACCENT=$'\033[36m'; INSTALL_SUCCESS=$'\033[32m'
        INSTALL_DIM=$'\033[2m'; INSTALL_RESET=$'\033[0m'
    fi
}

print_installation_rule() {
    local columns="${COLUMNS:-80}" width=64 rule
    if [[ "$columns" =~ ^[0-9]{1,3}$ ]]; then
        width=$((10#$columns - 4))
        (( width <= 64 )) || width=64
        (( width >= 20 )) || width=20
    fi
    printf -v rule '%*s' "$width" ''
    printf '  %s%s%s\n' "$INSTALL_DIM" "${rule// /-}" "$INSTALL_RESET"
}

print_installation_banner() {
    setup_installation_style
    printf '\n  %s%sQinpanel Installer%s\n' "$INSTALL_BOLD" "$INSTALL_ACCENT" "$INSTALL_RESET"
    printf '  Hosting control panel\n'
    print_installation_rule
    printf '\n'
}

print_installation_step() {
    printf '  %s[%s/7]%s %s\n' "$INSTALL_ACCENT" "$1" "$INSTALL_RESET" "$2"
}

print_summary_section() {
    printf '\n  %s%s%s\n' "$INSTALL_BOLD" "$1" "$INSTALL_RESET"
}

print_summary_field() {
    printf '  %-12s : %s\n' "$1" "$2"
}
print_help() {
    cat <<'EOF'
Usage: ./install.sh [options]
  --port <PORT>    Public port 1024..65535, excluding 7700 (default: 17890)
  --unattended     Run without interactive prompts
  --upgrade        Upgrade with a recovery snapshot and rollback
  --restore-digest <SHA256>  Approve one migration archive for restore
  --force-os       Skip the distribution version check
  --skip-firewall  Leave firewall rules unchanged
  -h, --help       Show this help
Fresh installations generate an administrator account. Upgrades keep existing accounts.
Administration help: panelctl --help and panelctl admin --help.
PANEL_PUBLIC_PORT also sets the public port. The internal API stays on 127.0.0.1:7700.
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --port)
                [[ $# -ge 2 ]] || { fail '--port requires a value'; return 1; }
                PANEL_PUBLIC_PORT="$2"; PORT_EXPLICIT=true; shift 2 ;;
            --restore-digest)
                [[ $# -ge 2 && "$2" =~ ^[a-f0-9]{64}$ ]] || { fail '--restore-digest requires a 64-character lowercase SHA-256 digest'; return 1; }
                RESTORE_DIGEST="$2"; shift 2 ;;
            --unattended) UNATTENDED=true; shift ;;
            --force-os) FORCE_OS=true; shift ;;
            --skip-firewall) SKIP_FIREWALL=true; shift ;;
            --upgrade) UPGRADE=true; shift ;;
            -h|--help) print_help; return 2 ;;
            *) fail "Unknown option: $1"; return 1 ;;
        esac
    done
}

validate_port() {
    [[ "$PANEL_PUBLIC_PORT" =~ ^[0-9]{4,5}$ ]] || { fail 'Port must be an integer from 1024 to 65535'; return 1; }
    PANEL_PUBLIC_PORT=$((10#$PANEL_PUBLIC_PORT))
    (( PANEL_PUBLIC_PORT >= 1024 && PANEL_PUBLIC_PORT <= 65535 && PANEL_PUBLIC_PORT != 7700 )) || {
        fail 'Port must be 1024..65535 and cannot use internal port 7700'; return 1;
    }
}

check_port_available() {
    local listeners
    listeners="$(ss -H -lntp "sport = :${PANEL_PUBLIC_PORT}")"
    [[ -z "$listeners" ]] && return 0
    # A repeated install may reuse only this panel's existing Nginx listener.
    if [[ -f /etc/nginx/sites-available/nusa-panel.conf && -L /etc/nginx/sites-enabled/nusa-panel.conf ]] &&
        [[ "$(readlink /etc/nginx/sites-enabled/nusa-panel.conf)" == /etc/nginx/sites-available/nusa-panel.conf ]] &&
        grep -Eq "^[[:space:]]*listen ${PANEL_PUBLIC_PORT};" /etc/nginx/sites-available/nusa-panel.conf &&
        ! grep -qv '"nginx"' <<< "$listeners"; then
        return 0
    fi
    fail "Public port ${PANEL_PUBLIC_PORT} is in use. Choose another --port; existing services were not stopped."
}

supported_distribution() {
    [[ ( "$1" == ubuntu && ( "$2" == 22.04 || "$2" == 24.04 ) ) ||
       ( "$1" == debian && "$2" == 12 ) ||
       ( "$1" == almalinux && "$2" =~ ^(9|10)(\.[0-9]+)?$ ) ]]
}

check_platform() {
    PACKAGE_FAMILY=deb
    [[ "$(id -u)" == 0 ]] || { fail 'Run this installer as root'; return 1; }
    [[ -r /etc/os-release ]] || { fail 'Cannot read /etc/os-release'; return 1; }
    . /etc/os-release
    if [[ "$FORCE_OS" != true ]]; then
        supported_distribution "$ID" "$VERSION_ID" || {
            fail 'Supported distributions: Ubuntu 22.04/24.04, Debian 12, AlmaLinux 9/10'; return 1;
        }
    fi
    if [[ "$ID" == almalinux ]]; then PACKAGE_FAMILY=rpm; fi
    [[ -f /sys/fs/cgroup/cgroup.controllers ]] || { fail 'The kernel must use cgroup v2'; return 1; }
}

install_packages() {
    # Application upgrades do not upgrade/restart host packages or tenant runtimes.
    if [[ "$UPGRADE" == true ]]; then
        local dependency
        for dependency in nginx nft curl tar sqlite3 systemctl setfacl flock ss runuser usermod; do
            command -v "$dependency" >/dev/null || { fail "Missing upgrade prerequisite: $dependency"; return 1; }
        done
        return 0
    fi
    if [[ "$PACKAGE_FAMILY" == rpm ]]; then
        # curl-minimal already provides HTTPS in AlmaLinux base images. Preserve
        # the installed provider instead of requesting its conflicting variant.
        if ! command -v curl >/dev/null; then
            dnf -y install curl-minimal
        fi
        dnf -y --setopt=keepcache=True install nginx nftables tar ca-certificates openssl \
            sqlite systemd acl util-linux iproute shadow-utils python3 gnupg2 \
            openssh-server git unzip zstd quota xfsprogs procps-ng xz \
            policycoreutils-python-utils dnf-plugins-core python3-dnf-plugin-versionlock
        dnf config-manager --set-enabled crb
        dnf -y install epel-release
        return 0
    fi
    export DEBIAN_FRONTEND=noninteractive
    if ! apt-get update -qq; then
        printf '%s\n' '[ERROR] Repository metadata could not be refreshed.' \
            'If APT reports "not valid yet", check the server UTC time and NTP synchronization:' \
            '  date -u' '  timedatectl status' \
            'Correct the server clock, then rerun the installer. Repository date/signature checks remain enabled.' >&2
        return 1
    fi
    apt-get install -y --no-install-recommends nginx nftables curl tar ca-certificates openssl \
        sqlite3 systemd acl util-linux iproute2 passwd python3 gpg dpkg-repack \
        openssh-server git unzip zstd quota xfsprogs procps xz-utils
}

find_release() {
    BIN_SOURCE="${SCRIPT_DIR}/bin"
    [[ -d "$BIN_SOURCE" ]] || BIN_SOURCE="${SCRIPT_DIR}/target/release"
    local binary path missing=0
    for path in UPGRADE_FROM templates/systemd/panel-api.service \
        templates/systemd/panel-agent.service \
        templates/systemd/panel-proc-hardening.service \
        templates/systemd/panel-runtime-order.conf \
        templates/systemd/panel-update.service scripts/bootstrap-install.sh scripts/panel-update-worker.sh \
        templates/systemd/qonpanel-server-backup.service scripts/server-backup-worker.py \
        templates/nginx/nusa-panel.conf \
        templates/nginx/nusa-reject-unknown.conf templates/nginx/almalinux.conf scripts/platform-selinux.py scripts/mail-auth.py scripts/object-storage-worker.py; do
        if [[ ! -f "$SCRIPT_DIR/$path" || ! -r "$SCRIPT_DIR/$path" || ! -s "$SCRIPT_DIR/$path" ]]; then
            printf '[ERROR] Required package file is missing or empty: %s\n' "$path" >&2
            missing=1
        fi
    done
    for binary in panel-api panel-agent panelctl; do
        if [[ ! -x "$BIN_SOURCE/$binary" ]]; then
            printf '[ERROR] Required package binary is missing: %s\n' "$BIN_SOURCE/$binary" >&2
            missing=1
        fi
    done
    [[ "$missing" == 0 ]] || {
        fail 'The release package is incomplete. Download a corrected release; do not bypass this check.'
        return 1
    }
    # Check package completeness before executing even the version commands.
    local output version
    for binary in panel-api panel-agent panelctl; do
        output="$("$BIN_SOURCE/$binary" --version)"
        printf '%s\n' "$output"
        version="${output##* }"
        [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { fail 'Invalid package binary version'; return 1; }
        [[ -z "$RELEASE_VERSION" || "$RELEASE_VERSION" == "v$version" ]] || { fail 'The packaged API, agent and CLI versions do not match'; return 1; }
        RELEASE_VERSION="v$version"
    done
    [[ -z "${NUSA_PANEL_VERSION:-}" || "$NUSA_PANEL_VERSION" == "$RELEASE_VERSION" ]] || { fail 'The binary version does not match the release tag'; return 1; }
}

acquire_install_lock() {
    exec 9>/run/lock/panel-install.lock
    flock -n 9 || { fail 'Another installation or upgrade is already running'; return 1; }
}
record_installed_release() {
    local release_digest="${NUSA_PANEL_RELEASE_SHA256:--}"
    [[ "$release_digest" == - || "$release_digest" =~ ^[a-f0-9]{64}$ ]] || { fail 'Invalid release digest'; return 1; }
    printf '%s %s\n' "$RELEASE_VERSION" "$release_digest" > /etc/panel/.installed-release.new
    chmod 0644 /etc/panel/.installed-release.new
    mv -f /etc/panel/.installed-release.new /etc/panel/installed-release
}
validate_upgrade() {
    local minimum installed=v0.1.0 ignored
    read -r minimum < "$SCRIPT_DIR/UPGRADE_FROM"
    [[ "$minimum" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { fail 'Invalid package upgrade compatibility version'; return 1; }
    [[ "$(printf '%s\n' "$minimum" "$RELEASE_VERSION" | sort -V | head -n 1)" == "$minimum" ]] || { fail 'The upgrade compatibility version exceeds the package version'; return 1; }
    if [[ -f /var/lib/panel/panel.db ]]; then UPGRADE=true; fi
    if [[ "$UPGRADE" == true ]]; then
        [[ -f /var/lib/panel/panel.db ]] || { fail 'An upgrade requires an existing installation'; return 1; }
        local admin
        admin="$(runuser -u panel-api -- sqlite3 /var/lib/panel/panel.db "SELECT EXISTS(SELECT 1 FROM users WHERE role='admin');")"
        [[ "$admin" == 1 ]] || { fail 'Upgrade stopped: no existing administrator account was found'; return 1; }
    fi
    if [[ -r /etc/panel/installed-release ]]; then
        read -r installed ignored < /etc/panel/installed-release
        [[ "$installed" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { fail 'Invalid installed version record'; return 1; }
        [[ "$(printf '%s\n' "$installed" "$RELEASE_VERSION" | sort -V | head -n 1)" == "$installed" ]] || { fail 'Downgrades are not supported; use the matching recovery snapshot to restore an earlier version'; return 1; }
    fi
    if [[ "$UPGRADE" == true && "$(printf '%s\n' "$minimum" "$installed" | sort -V | head -n 1)" != "$minimum" ]]; then
        fail "This package requires Qinpanel $minimum or later. Apply a supported intermediate release first."
        return 1
    fi
}

begin_transaction() {
    BACKUP_DIR="$(mktemp -d /var/backups/panel/install-XXXXXXXX)"
    chmod 0700 "$BACKUP_DIR"
    mkdir -p "$BACKUP_DIR/files"
    local path destination
    for path in "${MANAGED_FILES[@]}"; do
        if [[ -e "$path" || -L "$path" ]]; then
            # Coreutils 9.1 (Debian 12) mishandles -a --parents with absolute paths.
            # Build the destination explicitly; preserve file metadata and symlinks.
            destination="$BACKUP_DIR/files$path"
            mkdir -p -- "${destination%/*}" || {
                fail "Could not create the snapshot directory for: $path"; return 1;
            }
            cp -a -- "$path" "$destination" || {
                fail "Could not snapshot file: $path"; return 1;
            }
        fi
    done
    # Snapshot existing panel config for review/recovery; tenant vhosts are never replaced.
    [[ ! -d /etc/panel ]] || cp -a /etc/panel "$BACKUP_DIR/panel-config"
    if systemctl is-active --quiet panel-api.service >/dev/null 2>&1; then API_WAS_ACTIVE=true; fi
    if systemctl is-active --quiet panel-agent.service >/dev/null 2>&1; then AGENT_WAS_ACTIVE=true; fi
    if systemctl is-active --quiet nginx.service >/dev/null 2>&1; then NGINX_WAS_ACTIVE=true; fi
    if systemctl is-enabled --quiet panel-api.service >/dev/null 2>&1; then API_WAS_ENABLED=true; fi
    if systemctl is-enabled --quiet panel-agent.service >/dev/null 2>&1; then AGENT_WAS_ENABLED=true; fi
    if systemctl is-enabled --quiet nginx.service >/dev/null 2>&1; then NGINX_WAS_ENABLED=true; fi
    TRANSACTION=true
    # Stop the old API first: v0.1.0 incorrectly owned /run/panel's lifecycle.
    if systemctl cat panel-api.service >/dev/null 2>&1; then systemctl stop panel-api.service; fi
    if systemctl cat panel-agent.service >/dev/null 2>&1; then systemctl stop panel-agent.service; fi
    if [[ -d /var/lib/panel ]]; then
        cp -a /var/lib/panel "$BACKUP_DIR/state"
        STATE_SAVED=true
    fi
    STATE_CHECKED=true
}

rollback() {
    echo '[ERROR] Deployment failed; restoring the pre-installation snapshot.' >&2
    local failed=false path
    systemctl stop panel-api.service panel-agent.service || failed=true
    if [[ -f /usr/local/libexec/qonpanel-selinux ]]; then
        /usr/bin/python3 /usr/local/libexec/qonpanel-selinux --rollback || failed=true
    fi
    for path in "${MANAGED_FILES[@]}"; do
        rm -f "$path" || failed=true
        if [[ -e "$BACKUP_DIR/files$path" || -L "$BACKUP_DIR/files$path" ]]; then
            cp -a "$BACKUP_DIR/files$path" "$path" || failed=true
        fi
    done
    if [[ "$STATE_CHECKED" == true ]]; then
        # Core and operational state must roll back together; tenant files and analytics stay intact.
        for path in panel.db panel.db-wal panel.db-shm panel.operations.db panel.operations.db-wal panel.operations.db-shm; do
            rm -f "/var/lib/panel/$path" || failed=true
            if [[ "$STATE_SAVED" == true && -f "$BACKUP_DIR/state/$path" ]]; then
                cp -a "$BACKUP_DIR/state/$path" "/var/lib/panel/$path" || failed=true
            fi
        done
    fi
    systemctl daemon-reload || failed=true
    [[ "$API_WAS_ENABLED" == true ]] || systemctl disable panel-api.service >/dev/null 2>&1 || true
    [[ "$AGENT_WAS_ENABLED" == true ]] || systemctl disable panel-agent.service >/dev/null 2>&1 || true
    [[ "$NGINX_WAS_ENABLED" == true ]] || systemctl disable nginx.service >/dev/null 2>&1 || true
    if [[ "$AGENT_WAS_ACTIVE" == true ]]; then systemctl start panel-agent.service || failed=true; fi
    if [[ "$API_WAS_ACTIVE" == true ]]; then systemctl start panel-api.service || failed=true; fi
    if [[ "$NGINX_WAS_ACTIVE" == true ]]; then
        nginx -t && systemctl reload nginx || failed=true
    else
        systemctl stop nginx || failed=true
    fi
    echo "Recovery snapshot: $BACKUP_DIR" >&2
    [[ "$failed" == false ]] || echo '[ERROR] Recovery is incomplete; inspect the service journal and recovery snapshot.' >&2
}

on_error() {
    local code="$1"
    trap - ERR INT TERM
    set +e
    [[ "$TRANSACTION" != true ]] || rollback
    [[ -z "$STAGING_DIR" ]] || rm -rf "$STAGING_DIR"
    ADMIN_USERNAME=""; ADMIN_PASSWORD=""
    echo '[ERROR] Installation failed. Inspect: journalctl -u panel-api -u panel-agent -u nginx --no-pager -n 100' >&2
    exit "$code"
}

setup_identity() {
    if [[ "$PACKAGE_FAMILY" == rpm ]]; then
        getent group www-data >/dev/null || groupadd -r www-data
        getent group adm >/dev/null || groupadd -r adm
        if ! getent passwd www-data >/dev/null; then
            if getent passwd 33 >/dev/null; then fail 'UID 33 is already used; cannot provision the isolated web identity'; return 1; fi
            useradd -r -u 33 -g www-data -d /var/lib/nginx -s /usr/sbin/nologin www-data
        fi
        [[ "$(id -u www-data)" == 33 ]] || { fail 'www-data must have UID 33 for the loopback isolation policy'; return 1; }
        install -d -m 0755 /usr/lib/openssh
        if [[ ! -e /usr/lib/openssh/sftp-server && ! -L /usr/lib/openssh/sftp-server ]]; then
            ln -s /usr/libexec/openssh/sftp-server /usr/lib/openssh/sftp-server
        fi
        [[ "$(readlink -f /usr/lib/openssh/sftp-server)" == /usr/libexec/openssh/sftp-server ]] || { fail 'Custom SFTP compatibility path was preserved'; return 1; }
    fi
    getent group panel >/dev/null || groupadd -r panel
    getent group panel-tenants >/dev/null || groupadd -r panel-tenants
    # Stable private database-tool identity also exists on a fresh restore target.
    getent group panel-db-web >/dev/null || groupadd -r panel-db-web
    if ! id panel-db-web >/dev/null 2>&1; then
        useradd -r -g panel-db-web -d /var/lib/panel-db-web -s /usr/sbin/nologin panel-db-web
    fi
    [[ "$(id -u panel-db-web)" != 0 ]] || { fail 'The panel-db-web account must not have UID 0'; return 1; }
    if ! id panel-api >/dev/null 2>&1; then
        useradd -r -g panel -d /var/lib/panel -s /usr/sbin/nologin -c 'Qinpanel API' panel-api
    else
        [[ "$(id -u panel-api)" != 0 ]] || { fail 'The panel-api account must not have UID 0'; return 1; }
        usermod -g panel panel-api
    fi
    install -d -o root -g root -m 0755 /etc/panel /etc/panel/nginx
    install -d -o panel-api -g panel -m 0750 /var/lib/panel /var/lib/panel/backups
    install -d -o root -g panel -m 0755 /var/log/panel
    # /run/panel is created and secured by panel-agent.service on every boot.
    chown -R panel-api:panel /var/lib/panel
    local file
    for file in /var/lib/panel/panel.db{,-wal,-shm} /var/lib/panel/panel.operations.db{,-wal,-shm}; do
        [[ ! -f "$file" ]] || chmod 0640 "$file"
    done
}

configure_rpm_nginx() {
    [[ "$PACKAGE_FAMILY" == rpm ]] || return 0
    if ! grep -Fxq '# Managed by QonPanel: AlmaLinux nginx baseline.' /etc/nginx/nginx.conf; then
        local expected actual
        expected="$( { rpm -q --dump nginx nginx-core 2>/dev/null || true; } | awk '$1 == "/etc/nginx/nginx.conf" {print $4; exit}')"
        [[ "$expected" =~ ^[a-f0-9]{64}$ ]] || { fail 'Cannot verify the packaged AlmaLinux nginx.conf; custom configuration preserved'; return 1; }
        actual="$(sha256sum /etc/nginx/nginx.conf)"; actual="${actual%% *}"
        [[ "$actual" == "$expected" ]] || { fail 'Custom nginx.conf preserved; configure sites-enabled/* and conf.d/*.conf explicitly'; return 1; }
        install -m 0644 "$SCRIPT_DIR/templates/nginx/almalinux.conf" /etc/nginx/nginx.conf
    fi
    install -d -o www-data -g www-data -m 0750 /var/lib/nginx/tmp /var/cache/nginx
}

configure_platform_security() {
    [[ "$PACKAGE_FAMILY" == rpm ]] || return 0
    /usr/bin/python3 /usr/local/libexec/qonpanel-selinux --apply
}

commit_platform_security() {
    [[ "$PACKAGE_FAMILY" == rpm ]] || return 0
    /usr/bin/python3 /usr/local/libexec/qonpanel-selinux --commit
}

install_binaries_and_units() {
    local binary
    for binary in panel-api panel-agent panelctl; do
        install -o root -g root -m 0755 "$BIN_SOURCE/$binary" "/usr/local/bin/.$binary.new"
        mv -f "/usr/local/bin/.$binary.new" "/usr/local/bin/$binary"
    done
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-api.service" /etc/systemd/system/panel-api.service
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-agent.service" /etc/systemd/system/panel-agent.service
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-proc-hardening.service" /etc/systemd/system/panel-proc-hardening.service
    # Template and dash-prefix drop-ins also cover units generated by old builds.
    install -d -o root -g root -m 0755 /etc/systemd/system/panel-fpm@.service.d /etc/systemd/system/site-.service.d
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-runtime-order.conf" /etc/systemd/system/panel-fpm@.service.d/10-panel-runtime-order.conf
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-runtime-order.conf" /etc/systemd/system/site-.service.d/10-panel-runtime-order.conf
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/panel-update.service" /etc/systemd/system/panel-update.service
    install -o root -g root -m 0644 "$SCRIPT_DIR/templates/systemd/qonpanel-server-backup.service" /etc/systemd/system/qonpanel-server-backup.service
    install -d -o root -g root -m 0755 /usr/local/libexec
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/server-backup-worker.py" /usr/local/libexec/qonpanel-server-backup
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/platform-selinux.py" /usr/local/libexec/qonpanel-selinux
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/mail-auth.py" /usr/local/libexec/qonpanel-mail-auth
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/object-storage-worker.py" /usr/local/libexec/qonpanel-object-storage
    # Atomic replacement keeps a running updater's script inode intact.
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/panel-update-worker.sh" /usr/local/libexec/.panel-update-worker.new
    mv -f /usr/local/libexec/.panel-update-worker.new /usr/local/libexec/panel-update-worker
    install -o root -g root -m 0755 "$SCRIPT_DIR/scripts/bootstrap-install.sh" /usr/local/libexec/.panel-bootstrap.new
    mv -f /usr/local/libexec/.panel-bootstrap.new /usr/local/libexec/panel-bootstrap
    systemctl daemon-reload
}

initialize_database() {
    ADMIN_USERNAME=""; ADMIN_PASSWORD=""
    local result
    if ! result="$(runuser -u panel-api -- /usr/local/bin/panelctl --db-path /var/lib/panel/panel.db admin bootstrap --installer-output)"; then
        fail 'Account initialization or SQLite migration failed as panel-api; check panelctl and database access'
        return 1
    fi
    read_bootstrap_result "$result"
    if [[ "$UPGRADE" == true && -n "$ADMIN_USERNAME" ]]; then fail 'An upgrade must not create a new administrator account'; return 1; fi
    unset result
    chown -R panel-api:panel /var/lib/panel
    local file
    for file in /var/lib/panel/panel.db{,-wal,-shm} /var/lib/panel/panel.operations.db{,-wal,-shm}; do
        [[ ! -f "$file" ]] || chmod 0640 "$file"
    done
}

# Fixed protocol from the packaged CLI; never eval output or persist plaintext secrets.
read_bootstrap_result() {
    ADMIN_USERNAME=""; ADMIN_PASSWORD=""
    if [[ "$1" == preserved ]]; then return 0; fi
    if [[ "$1" =~ ^created\ (admin_[a-f0-9]{12})\ (Qp![a-f0-9]{32})$ ]]; then
        ADMIN_USERNAME="${BASH_REMATCH[1]}"
        ADMIN_PASSWORD="${BASH_REMATCH[2]}"
    else
        fail 'Invalid panelctl bootstrap response; installation stopped'
        return 1
    fi
}

print_installation_summary() {
    local server_ip="$1" completion='Installation complete'
    [[ "$UPGRADE" != true ]] || completion='Upgrade complete'
    setup_installation_style
    printf '\n  %s%sQinpanel%s | %s%s%s\n' "$INSTALL_BOLD" "$INSTALL_ACCENT" "$INSTALL_RESET" "$INSTALL_SUCCESS" "$completion" "$INSTALL_RESET"
    print_installation_rule
    [[ -z "$RELEASE_VERSION" ]] || print_summary_field 'Version' "$RELEASE_VERSION"
    print_summary_field 'Panel URL' "http://$server_ip:$PANEL_PUBLIC_PORT"
    print_summary_field 'Local checks' 'Passed: API, agent, Nginx and SQLite'
    printf '  Checks run on this server; public access is not verified.\n'
    printf '  For NAT or tunnels, use your configured public panel URL.\n'
    if [[ -n "$BACKUP_DIR" ]]; then
        print_summary_section 'Recovery snapshot'
        printf '  %s\n' "$BACKUP_DIR"
        printf '  Installer recovery copy; not a full website backup.\n'
    fi
    if [[ -n "$ADMIN_USERNAME" && -n "$ADMIN_PASSWORD" ]]; then
        print_summary_section 'Administrator account'
        print_summary_field 'Username' "$ADMIN_USERNAME"
        print_summary_field 'Password' "$ADMIN_PASSWORD"
        printf '  Save these credentials now in a password manager.\n'
        printf '  This generated password is shown only once.\n'
    else
        print_summary_section 'Administrator account'
        printf '  Existing administrator accounts and sign-in details are unchanged.\n'
        printf '  No new password was generated.\n'
    fi
    print_summary_section 'Next steps'
    printf '  1. Open the panel URL and sign in.\n'
    printf '  2. Review your credentials in Profile & security (/profile).\n'
    printf '  3. Enable two-factor authentication (2FA).\n'
    print_summary_section 'CLI reference'
    print_summary_field 'All commands' 'panelctl --help'
    print_summary_field 'Account help' 'panelctl admin --help'
    print_summary_field 'List admins' 'sudo panelctl admin list'
    print_summary_field 'Diagnostics' 'sudo panelctl doctor'
    print_summary_field 'Backup help' 'panelctl backup --help'
    print_summary_section 'Password recovery'
    printf '  sudo panelctl admin reset-password --username USERNAME\n'
    printf '  Replace USERNAME with an account from the admin list.\n'
    printf '\n  Recovery help: panelctl admin reset-password --help\n'
    print_installation_rule
    printf '\n'
    ADMIN_USERNAME=""; ADMIN_PASSWORD=""
}

render_panel_site() {
    sed "s/@PANEL_PUBLIC_PORT@/${PANEL_PUBLIC_PORT}/g" "$SCRIPT_DIR/templates/nginx/nusa-panel.conf"
}

is_packaged_nginx_default() {
    local path="$1" digest metadata package
    [[ -f "$path" && ! -L "$path" ]] || return 1
    digest="$(md5sum -- "$path")"; digest="${digest%% *}"
    for package in nginx-common nginx; do
        metadata="$(dpkg-query -W '-f=${Conffiles}\n' "$package" 2>/dev/null)" || continue
        if awk -v path="$path" -v digest="$digest" 'NF == 2 && $1 == path && $2 == digest { found=1 } END { exit !found }' <<< "$metadata"; then
            return 0
        fi
    done
    return 1
}

install_nginx_proxy() {
    install_nginx_proxy_at /etc/nginx /etc/panel/public-port
}

# Internal helper with explicit roots for rootless filesystem fixtures, not CLI/API inputs.
install_nginx_proxy_at() {
    local nginx_root="$1" port_file="$2" reject quarantine directives
    local disable_sites_default=false disable_conf_default=false
    reject="$nginx_root/conf.d/00-qpanel-reject-unknown.conf"
    if [[ -e "$reject" || -L "$reject" ]]; then
        [[ ! -L "$reject" ]] && cmp -s <(sed -E '1s/^# Managed by (Qpanel|Nusa Panel):/# Managed by QonPanel:/' "$reject") "$SCRIPT_DIR/templates/nginx/nusa-reject-unknown.conf" || {
            fail 'The Qinpanel catch-all contains operator changes; the file was not overwritten'; return 1;
        }
    fi
    if [[ -e "$nginx_root/sites-enabled/default" || -L "$nginx_root/sites-enabled/default" ]]; then
      if [[ -L "$nginx_root/sites-enabled/default" ]] &&
       [[ "$(readlink -f "$nginx_root/sites-enabled/default")" == "$nginx_root/sites-available/default" ]] &&
       is_packaged_nginx_default "$nginx_root/sites-available/default"; then
        disable_sites_default=true
      else
        fail 'The default Nginx vhost was modified or its package checksum is unavailable. Disable it explicitly; existing files were preserved'; return 1
      fi
    fi
    if [[ -e "$nginx_root/conf.d/default.conf" || -L "$nginx_root/conf.d/default.conf" ]]; then
      if is_packaged_nginx_default "$nginx_root/conf.d/default.conf"; then disable_conf_default=true
      else fail 'conf.d/default.conf is not a verified package default; the custom file was preserved'; return 1
      fi
    fi
    install -d -m 0755 "$nginx_root/conf.d" "$nginx_root/sites-available" "$nginx_root/sites-enabled"
    STAGING_DIR="$(mktemp -d "$nginx_root/.nusa-stage-XXXXXXXX")"
    mkdir "$STAGING_DIR/sites-enabled" "$STAGING_DIR/sites-available" "$STAGING_DIR/conf.d"
    cp -a "$nginx_root/sites-enabled/." "$STAGING_DIR/sites-enabled/"
    cp -a "$nginx_root/sites-available/." "$STAGING_DIR/sites-available/"
    cp -a "$nginx_root/conf.d/." "$STAGING_DIR/conf.d/"
    if [[ "$disable_sites_default" == true ]]; then rm -f "$STAGING_DIR/sites-enabled/default"; fi
    if [[ "$disable_conf_default" == true ]]; then rm -f "$STAGING_DIR/conf.d/default.conf"; fi
    cp "$SCRIPT_DIR/templates/nginx/nusa-reject-unknown.conf" "$STAGING_DIR/conf.d/00-qpanel-reject-unknown.conf"
    rm -f "$STAGING_DIR/sites-enabled/nusa-panel.conf"
    render_panel_site > "$STAGING_DIR/nusa-panel.conf"
    ln -s "$STAGING_DIR/nusa-panel.conf" "$STAGING_DIR/sites-enabled/nusa-panel.conf"
    # Validate against the real global configuration and every existing tenant site.
    # Refuse unsupported custom include layouts instead of overwriting global nginx.conf.
    directives="$(sed 's/#.*//' "$nginx_root/nginx.conf")"
    grep -Eq "include[[:space:]]+$nginx_root/sites-enabled/\*;" <<< "$directives" &&
    grep -Eq "include[[:space:]]+$nginx_root/conf.d/\*.conf;" <<< "$directives" || {
        fail 'Nginx must include sites-enabled/* and conf.d/*.conf; the custom configuration was preserved'; return 1;
    }
    sed -e "s|include[[:space:]]\+$nginx_root/sites-enabled/\*;|include $STAGING_DIR/sites-enabled/*;|g" \
        -e "s|include[[:space:]]\+$nginx_root/conf.d/\*.conf;|include $STAGING_DIR/conf.d/*.conf;|g" \
        "$nginx_root/nginx.conf" > "$STAGING_DIR/nginx.conf"
    nginx -t -c "$STAGING_DIR/nginx.conf" || return 1
    install -m 0644 "$STAGING_DIR/nusa-panel.conf" "$nginx_root/sites-available/.nusa-panel.new"
    mv -f "$nginx_root/sites-available/.nusa-panel.new" "$nginx_root/sites-available/nusa-panel.conf"
    rm -f "$nginx_root/sites-enabled/.nusa-panel.new"
    ln -s "$nginx_root/sites-available/nusa-panel.conf" "$nginx_root/sites-enabled/.nusa-panel.new"
    mv -Tf "$nginx_root/sites-enabled/.nusa-panel.new" "$nginx_root/sites-enabled/nusa-panel.conf"
    install -m 0644 "$STAGING_DIR/conf.d/00-qpanel-reject-unknown.conf" "$reject.new"
    mv -f "$reject.new" "$reject"
    if [[ "$disable_sites_default" == true ]]; then
        quarantine="$(mktemp "$nginx_root/sites-available/default.qpanel-disabled-XXXXXXXX")"
        mv -f "$nginx_root/sites-enabled/default" "$quarantine"
    fi
    if [[ "$disable_conf_default" == true ]]; then
        quarantine="$(mktemp "$nginx_root/conf.d/default.conf.qpanel-disabled-XXXXXXXX")"
        mv -f "$nginx_root/conf.d/default.conf" "$quarantine"
    fi
    nginx -t || return 1
    printf '%s\n' "$PANEL_PUBLIC_PORT" > "$port_file"
    chmod 0644 "$port_file"
    rm -rf "$STAGING_DIR"
    STAGING_DIR=""
}

activate_nginx_proxy() {
    nginx -t
    systemctl enable nginx.service
    if systemctl is-active --quiet nginx.service; then
        systemctl reload nginx.service
    else
        systemctl start nginx.service
    fi
}

configure_firewall() {
    [[ "$SKIP_FIREWALL" == false ]] || return 0
    if command -v ufw >/dev/null && LC_ALL=C ufw status | grep -q '^Status: active'; then
        ufw allow "${PANEL_PUBLIC_PORT}/tcp" comment 'Qinpanel public proxy'
    elif command -v firewall-cmd >/dev/null && firewall-cmd --state >/dev/null 2>&1; then
        firewall-cmd --permanent --add-port="${PANEL_PUBLIC_PORT}/tcp"
        firewall-cmd --add-port="${PANEL_PUBLIC_PORT}/tcp"
    elif nft list ruleset 2>/dev/null | grep -Eq 'hook input|policy drop'; then
        echo "[WARNING] Custom nftables rules detected. Allow ${PANEL_PUBLIC_PORT}/tcp if needed; existing rules were not changed." >&2
    fi
    echo "Allow ${PANEL_PUBLIC_PORT}/tcp in your provider firewall or security group. Keep internal port 7700 private."
}

wait_health() {
    local url="$1" attempts="${2:-75}" attempt status
    for ((attempt=1; attempt<=attempts; attempt++)); do
        if status="$(curl --noproxy '*' --connect-timeout 1 --max-time 2 -fsS \
            --output /dev/null --write-out '%{http_code}' "$url" 2>/dev/null)" && [[ "$status" == 200 ]]; then
            return 0
        fi
        if (( attempt < attempts )); then sleep 0.2; fi
    done
    fail "Health check timeout after $attempts attempts: $url"
}

validate_api_binding() {
    local bindings
    if ! bindings="$(ss -H -lnt 'sport = :7700' 2>/dev/null | awk 'NF {print $4}')"; then
        fail 'Could not inspect the API listener with ss (port 7700)'
        return 1
    fi
    # Accept multiple entries for the expected address, but no other interface.
    if grep -Fxq '127.0.0.1:7700' <<< "$bindings" &&
        ! grep -Fxqv '127.0.0.1:7700' <<< "$bindings"; then
        return 0
    fi
    {
        printf '[ERROR] API listener validation failed.\nExpected:\n  127.0.0.1:7700\nDetected:\n'
        printf '%s\n' "${bindings:-<none>}" | sed 's/^/  /'
    } >&2
    return 1
}

wait_agent_ready() {
    local attempt
    for attempt in {1..30}; do
        if [[ -S /run/panel/agent.sock ]] &&
            runuser -u panel-api -- /usr/local/bin/panelctl ipc-check >/dev/null 2>&1; then return 0; fi
        sleep 1
    done
    fail 'The agent is not ready to accept RPC from the panel-api identity'
}

validate_health() {
    local service
    for service in panel-agent panel-api nginx; do
        systemctl is-active --quiet "$service" || { fail "Service is not active: $service"; return 1; }
    done
    [[ -S /run/panel/agent.sock ]] || { fail 'The agent socket is unavailable'; return 1; }
    [[ "$(stat -c '%U:%G:%a' /run/panel)" == root:panel:750 ]] || { fail 'Invalid permissions on /run/panel'; return 1; }
    [[ "$(stat -c '%U:%G:%a' /run/panel/agent.sock)" == root:panel:660 ]] || { fail 'Invalid permissions on agent.sock'; return 1; }
    runuser -u panel-api -- test -w /run/panel/agent.sock
    runuser -u panel-api -- /usr/local/bin/panelctl ipc-check
    [[ "$(runuser -u panel-api -- sqlite3 /var/lib/panel/panel.db 'PRAGMA quick_check;')" == ok ]] || {
        fail 'SQLite integrity check failed'; return 1;
    }
    local file
    [[ "$(stat -c '%U:%G:%a' /var/lib/panel)" == panel-api:panel:750 ]] || { fail 'Invalid permissions on the SQLite directory'; return 1; }
    for file in /var/lib/panel/panel.db{,-wal,-shm} /var/lib/panel/panel.operations.db{,-wal,-shm}; do
        if [[ -e "$file" && "$(stat -c '%U:%G' "$file")" != panel-api:panel ]]; then
            fail "Invalid SQLite file ownership: $file"; return 1
        fi
    done
    wait_health "http://127.0.0.1:${PANEL_PUBLIC_PORT}/healthz"
}

main() {
    # Preserve custom port on repeat install/upgrade unless explicitly overridden.
    local args_status=0
    parse_args "$@" || args_status=$?
    [[ "$args_status" != 2 ]] || return 0
    [[ "$args_status" == 0 ]] || return "$args_status"
    print_installation_banner
    print_installation_step 1 'Checking installation requirements'
    if [[ "$PORT_EXPLICIT" != true && -r /etc/panel/public-port ]]; then
        read -r PANEL_PUBLIC_PORT < /etc/panel/public-port
    fi
    validate_port
    check_platform
    acquire_install_lock
    find_release
    validate_upgrade
    print_installation_step 2 'Preparing system packages'
    install_packages
    print_installation_step 3 'Creating a recovery snapshot'
    check_port_available
    install -d -o root -g root -m 0700 /var/backups/panel
    trap 'on_error $?' ERR
    trap 'on_error 130' INT
    trap 'on_error 143' TERM
    begin_transaction
    print_installation_step 4 'Installing panel components'
    setup_identity
    install_binaries_and_units
    configure_rpm_nginx
    initialize_database
    install_nginx_proxy
    configure_platform_security
    print_installation_step 5 'Starting and checking local services'
    systemctl enable panel-agent.service panel-api.service
    systemctl start panel-agent.service
    wait_agent_ready
    systemctl start panel-api.service
    # Type=simple reports started before bind/migrations finish: poll readiness first.
    wait_health http://127.0.0.1:7700/healthz
    validate_api_binding
    activate_nginx_proxy
    validate_health
    print_installation_step 6 'Applying panel network rules'
    configure_firewall
    print_installation_step 7 'Finalizing installation'
    record_installed_release
    if [[ -n "$RESTORE_DIGEST" ]]; then
        /usr/bin/python3 /usr/local/libexec/qonpanel-server-backup --approve "$RESTORE_DIGEST"
    fi
    commit_platform_security
    TRANSACTION=false
    trap - ERR INT TERM
    local server_ip
    server_ip="$(hostname -I 2>/dev/null | awk '{print $1}')" || server_ip="127.0.0.1"
    server_ip="${server_ip:-127.0.0.1}"
    [[ "$server_ip" != *:* ]] || server_ip="[$server_ip]"
    print_installation_summary "$server_ip"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi
