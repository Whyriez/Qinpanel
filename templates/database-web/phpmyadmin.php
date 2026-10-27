<?php
// Loaded by the distro phpMyAdmin config only for private, authenticated FastCGI requests.
if (getenv('PANEL_DB_ENGINE') === 'mariadb' && getenv('PANEL_DB_PASSWORD') !== false) {
    $cfg['Servers'] = [1 => [
        'auth_type' => 'signon',
        'host' => 'localhost',
        'socket' => '/run/mysqld/mysqld.sock',
        'AllowNoPassword' => false,
        'SignonScript' => '/usr/share/panel/db-web/signon.php',
        'SignonURL' => getenv('PANEL_WEB_PREFIX'),
        'LogoutURL' => '/',
        'only_db' => [getenv('PANEL_DATABASE')],
        'AllowRoot' => false,
        'AllowDeny' => ['order' => 'explicit', 'rules' => ['allow ' . getenv('PANEL_DB_USER') . ' from all']],
    ]];
    $cfg['ServerDefault'] = 1;
    $cfg['AllowArbitraryServer'] = false;
    $cfg['PmaAbsoluteUri'] = (getenv('HTTPS') === 'on' ? 'https://' : 'http://')
        . $_SERVER['HTTP_HOST'] . getenv('PANEL_WEB_PREFIX');
    $cfg['TempDir'] = '/var/lib/panel-db-web';
    $cfg['SessionSavePath'] = '/var/lib/panel-db-web/' . getenv('PANEL_WEB_SESSION');
    $cfg['VersionCheck'] = false;
    $cfg['SendErrorReports'] = 'never';
    $cfg['AllowUserDropDatabase'] = false;
    $cfg['LoginCookieValidity'] = 1800;
}
