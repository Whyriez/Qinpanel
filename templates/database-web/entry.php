<?php
declare(strict_types=1);

// The Unix socket is accessible to panel-api only. These are FastCGI parameters,
// never forwarded browser headers. No tenant input selects a server or PHP file.
$database = getenv('PANEL_DATABASE');
$prefix = getenv('PANEL_WEB_PREFIX');
$engine = getenv('PANEL_DB_ENGINE');
$identity = getenv('PANEL_WEB_SESSION');
if (!is_string($database) || !preg_match('/\A[a-zA-Z0-9_]+\z/', $database)
    || !is_string($identity) || !preg_match('/\A[a-f0-9]{64}\z/', $identity)
    || !is_string($prefix) || !preg_match('~\A/api/v1/databases/[1-9][0-9]*/web/\z~', $prefix)
    || !in_array($engine, ['mariadb', 'postgresql'], true)) {
    http_response_code(403);
    exit('Akses pengelola database ditolak.');
}
header('Cache-Control: no-store');
header('Referrer-Policy: no-referrer');
session_set_cookie_params(['path' => $prefix, 'httponly' => true,
    'secure' => getenv('HTTPS') === 'on', 'samesite' => 'Strict']);
// Separate storage namespaces bind library sessions to both the panel session and
// database, while allowing the libraries to rotate their own session identifiers.
$sessionDirectory = '/var/lib/panel-db-web/' . $identity;
if (!is_dir($sessionDirectory) && !mkdir($sessionDirectory, 0700, true) && !is_dir($sessionDirectory)) {
    http_response_code(503);
    exit('Penyimpanan sesi pengelola tidak tersedia.');
}
ini_set('session.save_path', $sessionDirectory);
ini_set('session.use_strict_mode', '1');
// Match the API allowlist, including phpMyAdmin's HTTPS and secure-prefix names.
$_COOKIE = array_filter($_COOKIE, static function ($key): bool {
    if (str_starts_with($key, 'adminer_')) { return true; }
    $name = str_starts_with($key, '__Secure-') ? substr($key, 9) : $key;
    return str_starts_with($name, 'pma') || in_array($name, ['phpMyAdmin', 'phpMyAdmin_https'], true);
}, ARRAY_FILTER_USE_KEY);

$asset = getenv('PANEL_WEB_ASSET') ?: '';
if ($engine === 'mariadb') {
    $_GET['db'] = $database;
    $_REQUEST['db'] = $database;
    if (isset($_POST['db'])) { $_POST['db'] = $database; }
    $isIndex = $asset === '' || $asset === 'index.php';
    if ($isIndex && empty($_GET['route']) && empty($_POST['route'])) {
        $_GET['route'] = $_REQUEST['route'] = '/database/structure';
    }
    // phpMyAdmin routes from globals, but PSR-7 factories may parse the URI instead.
    // Keep both views scoped; otherwise the controller sees no database and redirects
    // to route=/&reload=1, which routes back to the same database controller forever.
    $_SERVER['QUERY_STRING'] = http_build_query($_GET, '', '&', PHP_QUERY_RFC3986);
    $_SERVER['REQUEST_URI'] = $prefix . ($asset === '' ? 'index.php' : $asset)
        . '?' . $_SERVER['QUERY_STRING'];
    if ($isIndex) {
        chdir('/usr/share/phpmyadmin');
        require '/usr/share/phpmyadmin/index.php';
        exit;
    }
    if ($asset === 'js/messages.php') {
        chdir('/usr/share/phpmyadmin/js');
        require '/usr/share/phpmyadmin/js/messages.php';
        exit;
    }
    // Distro JavaScript packages may be symlinked from phpMyAdmin to /usr/share/javascript.
    $path = realpath('/usr/share/phpmyadmin/' . $asset);
    $extension = strtolower(pathinfo($asset, PATHINFO_EXTENSION));
    $types = ['css' => 'text/css', 'js' => 'application/javascript', 'png' => 'image/png',
        'gif' => 'image/gif', 'svg' => 'image/svg+xml', 'ico' => 'image/x-icon',
        'woff' => 'font/woff', 'woff2' => 'font/woff2', 'ttf' => 'font/ttf', 'map' => 'application/json'];
    $toolRoot = realpath('/usr/share/phpmyadmin');
    if (is_string($path) && ((is_string($toolRoot) && str_starts_with($path, $toolRoot . '/'))
        || str_starts_with($path, '/usr/share/javascript/')) && isset($types[$extension]) && is_file($path)) {
        header('Content-Type: ' . $types[$extension]);
        readfile($path);
        exit;
    }
    http_response_code(404);
    exit('Berkas pengelola tidak ditemukan.');
}
if ($asset !== '' && $asset !== 'index.php') {
    http_response_code(404);
    exit;
}

// Adminer 4/5 (distro) and 6 (namespaced) expose the same extension hooks.
function adminer_object() {
    if (!class_exists('PanelDatabaseAdminer', false)) {
        class_alias(class_exists('Adminer\\Adminer') ? 'Adminer\\Adminer' : 'Adminer', 'PanelDatabaseAdminerBase');
        class PanelDatabaseAdminer extends PanelDatabaseAdminerBase {
            function credentials() { return ['127.0.0.1', getenv('PANEL_DB_USER'), getenv('PANEL_DB_PASSWORD')]; }
            function database() { return getenv('PANEL_DATABASE'); }
            function databases($flush = true) { return [getenv('PANEL_DATABASE')]; }
            function login($login, $password) { return $login === getenv('PANEL_DB_USER') && hash_equals(getenv('PANEL_DB_PASSWORD'), $password); }
            function loginForm() { echo '<p>Sesi berakhir. Buka kembali pengelola dari panel.</p>'; }
            function loginFormField($name, $heading, $value) { return ''; }
        }
    }
    return new PanelDatabaseAdminer;
}
// Fix the driver, server and database even when a user edits the URL or form.
$_GET['pgsql'] = '127.0.0.1';
$_GET['username'] = getenv('PANEL_DB_USER');
$_GET['db'] = $database;
unset($_GET['server'], $_GET['sqlite'], $_GET['sqlite2'], $_GET['oracle'], $_GET['mssql']);
unset($_GET['mongo'], $_GET['elastic'], $_GET['clickhouse'], $_GET['simpledb']);
unset($_POST['auth']);
// Seed the library's login once per panel-bound session. Subsequent POSTs retain
// Adminer's own CSRF validation and request body (including multipart imports).
session_name('adminer_sid');
session_start();
$needsLogin = empty($_SESSION['pwds']['pgsql']['127.0.0.1'][getenv('PANEL_DB_USER')]);
if ($needsLogin && $_SERVER['REQUEST_METHOD'] !== 'GET') {
    http_response_code(409);
    exit('Sesi berakhir. Buka kembali pengelola dari panel sebelum mengirim perubahan.');
}
if (empty($_COOKIE['adminer_key'])) {
    $_COOKIE['adminer_key'] = bin2hex(random_bytes(32));
    setcookie('adminer_key', $_COOKIE['adminer_key'], ['path' => $prefix,
        'httponly' => true, 'secure' => getenv('HTTPS') === 'on', 'samesite' => 'Strict']);
}
if ($needsLogin) {
    $_POST['auth'] = ['driver' => 'pgsql', 'server' => '127.0.0.1',
        'username' => getenv('PANEL_DB_USER'), 'password' => getenv('PANEL_DB_PASSWORD'), 'db' => $database];
}
require '/usr/share/adminer/adminer.php';
