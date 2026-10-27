#!/usr/bin/env bash
# Real package/file checks in a disposable CI container. No panel daemons.
set -Eeuo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "${QONPANEL_DISPOSABLE_CI:-}" == 1 &&
   ( -f /.dockerenv || -f /run/.containerenv ) ]] || {
    echo 'This check requires an explicitly disposable CI container.' >&2; exit 1;
}
export LC_ALL=C
. /etc/os-release
if [[ "$ID" == almalinux ]]; then
    dnf -y install dnf-plugins-core
    dnf config-manager --set-enabled crb
    dnf -y install epel-release
    dnf -y install nginx nftables sqlite acl shadow-utils iproute openssh-server git unzip zstd quota xfsprogs procps-ng xz \
        policycoreutils-python-utils python3-dnf-plugin-versionlock python3-rpm \
        mariadb-server mariadb postgresql-server postgresql-contrib postfix postfix-lmdb dovecot opendkim libsodium \
        php-cli php-mysqlnd php-pgsql php-mbstring php-xml php-pecl-zip phpMyAdmin
    major="${VERSION_ID%%.*}"
    if [[ "$major" == 9 ]]; then key=2021; else key=2024; fi
    curl --fail --silent --show-error --proto '=https' "https://rpms.remirepo.net/RPM-GPG-KEY-remi$key" -o /etc/pki/rpm-gpg/QONPANEL-REMI
    rpm --import /etc/pki/rpm-gpg/QONPANEL-REMI
    printf '[qonpanel-remi-safe]\nname=Remi parallel PHP\nbaseurl=https://rpms.remirepo.net/enterprise/%s/safe/$basearch/\nenabled=1\ngpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/QONPANEL-REMI\n' "$major" > /etc/yum.repos.d/qonpanel-remi-safe.repo
    for version in 74 80 81 82 83 84 85; do
        packages=("php$version-php-fpm" "php$version-php-cli" "php$version-php-common"
            "php$version-php-mbstring" "php$version-php-xml" "php$version-php-pecl-zip" "php$version-php-mysqlnd")
        if [[ "$version" != 85 ]]; then packages+=("php$version-php-opcache"); fi
        dnf -y --setopt=keepcache=True install "${packages[@]}"
        "/opt/remi/php$version/root/usr/bin/php" --version
        test -x "/opt/remi/php$version/root/usr/sbin/php-fpm"
        test -f "/etc/opt/remi/php$version/php.ini"
    done
    test -f /usr/share/phpMyAdmin/index.php
    test -x /usr/libexec/openssh/sftp-server
    test -x /usr/bin/php-cgi
else
    export DEBIAN_FRONTEND=noninteractive
    printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d
    chmod 0755 /usr/sbin/policy-rc.d
    apt-get update -qq
    apt-get install -y --no-install-recommends nginx nftables sqlite3 acl passwd iproute2 git unzip zstd quota xfsprogs procps xz-utils \
        openssh-server python3 gpg dpkg-repack mariadb-server mariadb-client postgresql \
        postgresql-contrib postfix dovecot-core dovecot-imapd dovecot-lmtpd opendkim opendkim-tools
    test -x /usr/lib/openssh/sftp-server
fi
# Retain Argon2id even though RHEL compiles Dovecot without libsodium.
if [[ "$ID" == almalinux ]]; then
    postconf -m | grep -Fx lmdb
    python3 -I -c 'import seobject; assert callable(seobject.booleanRecords.get_all) and callable(seobject.booleanRecords.delete)'
    test -x /usr/libexec/dovecot/checkpassword-reply
    python3 scripts/mail-auth.py --check
    QONPANEL_TEST_SODIUM=1 python3 scripts/tests/mail-auth-test.py
else
    doveadm pw -l | tr ' ' '\n' | grep -Fx ARGON2ID
    python3 scripts/tests/mail-auth-test.py
fi
python3 scripts/tests/server-backup-test.py
python3 scripts/tests/platform-support-test.py
echo 'Package layout and hermetic upgrade/restore checks passed; service/SELinux acceptance is separate.'
