/* Frontend fixtures only: never starts panel-api, panel-agent or host services. */
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const base = process.env.NUSA_UI_URL || "http://127.0.0.1:5180";
const output = path.join(__dirname, "../artifacts/site-detail");
const domain = "testing.incogmsg.my.id";
const site = {
    id: 42,
    user_id: 2,
    primary_domain: domain,
    kind: "php",
    version: "8.3",
    preset: "generic",
    docroot: `/home/p_customer/sites/${domain}/public`,
    status: "active",
    spa_fallback: false,
    force_https: true,
    created_at: 1770000000,
    updated_at: 1770000000,
    domains: [],
};
const overview = {
    site_id: 42,
    domain,
    status: {
        webserver_status: "active",
        runtime_status: "running",
        healthy: true,
        uptime_secs: 162250,
        pid: 22800,
    },
    quick_actions_available: ["restart", "reload", "clear_cache"],
    resources: {
        cpu_usage_usec: 150000,
        cpu_percent: 2.8,
        memory_bytes: 134217728,
        memory_limit_bytes: 1073741824,
        pids_current: 8,
        pids_limit: 100,
    },
    ssl: {
        has_certificate: true,
        issuer: "Let's Encrypt",
        valid_until: 1800000000,
        days_remaining: 72,
        auto_renew: true,
        status: "valid",
        force_https: true,
    },
    runtime: {
        kind: "php",
        version: "8.3",
        preset: "generic",
        entrypoint_or_target: site.docroot,
    },
    storage: {
        docroot_bytes: 157286400,
        databases_bytes: 52428800,
        backups_bytes: 314572800,
        total_site_bytes: 524288000,
        plan_disk_limit_bytes: 10737418240,
        usage_percentage: 4.9,
    },
    warnings: [],
    latest_deployment: null,
    latest_backup: null,
};
let checks = 0;
const check = (value, message) => {
    assert.ok(value, message);
    console.log(`ok ${++checks}: ${message}`);
};
(async () => {
    await fs.mkdir(output, { recursive: true });
    const browser = await chromium.launch({
        headless: true,
        channel: process.env.NUSA_BROWSER_CHANNEL || "msedge",
    });
    try {
        const page = await browser.newPage({
            viewport: { width: 1440, height: 1100 },
            reducedMotion: "reduce",
        });
        page.setDefaultTimeout(10000);
        const errors = [],
            actions = [],
            calls = [];
        let listFailure = false,
            overviewFailure = false,
            actionFailure = false,
            rich = false,
            zero = false;
        page.on("pageerror", (error) => errors.push(error.message));
        await page.addInitScript(() => {
            localStorage.setItem("nusa_theme", "light");
        });
        await page.route("**/api/v1/**", async (route) => {
            const request = route.request(),
                url = new URL(request.url());
            calls.push(url.pathname);
            let body = {},
                status = 200;
            if (url.pathname === "/api/v1/auth/me")
                body = {
                    user: { id: 2, username: "customer", role: "user" },
                    csrf_token: "fixture",
                };
            else if (url.pathname === "/api/v1/auth/csrf")
                body = { csrf_token: "fixture" };
            else if (url.pathname === "/api/v1/sites") {
                status = listFailure ? 503 : 200;
                body = listFailure
                    ? { error: "Daftar situs sementara tidak tersedia." }
                    : [site];
            } else if (url.pathname === "/api/v1/sites/42") body = site;
            else if (url.pathname === "/api/v1/sites/42/ssl")
                body = { site_id: 42, has_ssl: false, certificate: null };
            else if (url.pathname === "/api/v1/sites/42/overview") {
                status = overviewFailure ? 503 : 200;
                body = overviewFailure
                    ? { error: "Ringkasan sementara tidak tersedia." }
                    : structuredClone(overview);
                if (!overviewFailure && rich) {
                    body.warnings = [
                        {
                            id: "ssl-expiry",
                            level: "warning",
                            title: "Sertifikat segera kedaluwarsa",
                            message:
                                "Periksa perpanjangan sertifikat sebelum masa berlakunya habis.",
                            action_tab: "ssl",
                        },
                    ];
                    body.latest_deployment = {
                        deployment_id: 3,
                        release_id: "release-2026-10-16",
                        commit_hash: "abc123def456",
                        commit_message:
                            "Perbarui halaman katalog dan proses checkout",
                        commit_author: "customer",
                        branch: "main",
                        trigger_type: "manual",
                        status: "success",
                        duration_ms: 18250,
                        created_at: 1770000000,
                        finished_at: 1770000018,
                        can_rollback: true,
                    };
                    body.latest_backup = {
                        backup_id: 5,
                        filename: "testing.incogmsg.my.id-2026-10-16.tar.gz",
                        size_bytes: 314572800,
                        status: "completed",
                        includes_files: true,
                        includes_databases: true,
                        created_at: 1770000000,
                    };
                }
                if (!overviewFailure && zero) {
                    body.resources.memory_bytes = 0;
                    body.resources.pids_current = 0;
                    Object.assign(body.storage, {
                        docroot_bytes: 0,
                        databases_bytes: 0,
                        backups_bytes: 0,
                        total_site_bytes: 0,
                        usage_percentage: 0,
                    });
                }
            } else if (url.pathname === "/api/v1/sites/42/action") {
                actions.push(JSON.parse(request.postData()).action);
                status = actionFailure ? 500 : 200;
                body = actionFailure
                    ? { error: "Aksi tidak dapat dijalankan." }
                    : {
                          success: true,
                          action: actions.at(-1),
                          message: "Layanan berhasil dimuat ulang.",
                      };
            } else if (url.pathname.includes("timeseries"))
                body = { points: [] };
            else if (url.pathname === "/api/v1/jobs")
                body = { items: [], total: 0, total_pages: 1 };
            else if (url.pathname.includes("notifications"))
                body = {
                    notifications: [],
                    count: 0,
                    unread_count: 0,
                    total: 0,
                };
            else if (url.pathname === "/api/v1/system/info")
                body = {
                    hostname: "fixture",
                    os_name: "Ubuntu",
                    os_version: "24.04",
                    uptime_secs: 100,
                    agent_connected: true,
                    panel_version: "0.1.1",
                };
            else if (url.pathname === "/api/v1/system/metrics")
                body = {
                    state: "waiting",
                    sample: null,
                    sampling_interval_secs: 10,
                };
            else {
                status = 404;
                body = { error: "No fixture" };
            }
            await route.fulfill({
                status,
                contentType: "application/json",
                body: JSON.stringify(body),
            });
        });
        const open = async (target) => {
            await page.goto(base + target, { waitUntil: "domcontentloaded" });
        };
        const ready = () =>
            page
                .getByRole("heading", {
                    name: "Penggunaan sumber daya",
                    exact: true,
                })
                .waitFor();
        await open("/dashboard");
        await page.getByRole("button", { name: domain, exact: true }).click();
        await ready();
        check(new URL(page.url()).pathname === `/sites/${domain}/overview`, "dashboard opens the domain URL");
        await open("/sites");
        await page.getByRole("button", { name: domain, exact: true }).click();
        await ready();
        check(
            new URL(page.url()).pathname === `/sites/${domain}/overview`,
            "site list opens the domain URL",
        );
        check(
            await page
                .locator(".project-panel")
                .first()
                .evaluate((el) => {
                    const css = getComputedStyle(el);
                    return (
                        css.backgroundColor === "rgb(255, 255, 255)" &&
                        css.backdropFilter === "none" &&
                        css.boxShadow === "none"
                    );
                }),
            "light panels are solid white without blur or shadows",
        );
        check(
            await page
                .locator(".project-service-status h2")
                .evaluate(
                    (el) => getComputedStyle(el).color === "rgb(24, 38, 47)",
                ),
            "light heading uses readable theme ink",
        );
        check(
            (await page
                .getByText("Belum ada deployment", { exact: true })
                .isVisible()) &&
                (await page
                    .getByText("Belum ada cadangan", { exact: true })
                    .isVisible()),
            "empty history has useful, readable states",
        );
        check(
            (await page
                .locator('.project-meter[aria-label="Penggunaan memori"]')
                .getAttribute("aria-valuenow")) === "12.5",
            "memory meter reflects real usage",
        );
        check(
            (await page.locator(".project-service-actions button").count()) ===
                3,
            "service controls match API available actions",
        );
        await page.getByRole('button', { name: 'Segarkan ringkasan' }).focus();
        await page.keyboard.press('Tab');
        check(await page.getByRole('link', { name: 'Kunjungi situs', exact: true }).evaluate(el => el === document.activeElement && el.matches(':focus-visible')), 'overview controls have keyboard access and visible focus');
        await page.screenshot({
            path: path.join(output, "desktop-light.png"),
            fullPage: true,
        });
        await page
            .getByRole("button", { name: "Muat ulang Nginx", exact: true })
            .click();
        await page
            .getByText("Layanan berhasil dimuat ulang.", { exact: true })
            .waitFor();
        check(
            actions.at(-1) === "reload",
            "reload submits the typed action and refreshes status",
        );
        actionFailure = true;
        await page
            .getByRole("button", { name: "Mulai ulang", exact: true })
            .click();
        await page.getByRole("alert").waitFor();
        check(
            await page
                .getByRole("button", { name: "Mulai ulang", exact: true })
                .isEnabled(),
            "failed action displays an error and restores controls",
        );
        actionFailure = false;
        await page
            .getByRole("link", { name: "Domain", exact: false })
            .filter({ hasText: "Domain" })
            .first()
            .click();
        check(
            new URL(page.url()).pathname === `/sites/${domain}/domains`,
            "nested tabs retain the domain",
        );
        await page.goBack();
        await ready();
        await page.reload({ waitUntil: "domcontentloaded" });
        await ready();
        check(
            new URL(page.url()).pathname === `/sites/${domain}/overview`,
            "back navigation and direct refresh restore the domain route",
        );
        await open("/sites/42/ssl?source=bookmark#certificate");
        await page.waitForURL((url) => url.pathname === `/sites/${domain}/ssl`);
        check(
            new URL(page.url()).search === "?source=bookmark" &&
                new URL(page.url()).hash === "#certificate",
            "numeric bookmark redirects while retaining tab, query and fragment",
        );
        await page
            .getByRole("link", { name: "Ringkasan", exact: true })
            .click();
        await ready();
        await page
            .getByRole("button", { name: "Ganti tema", exact: true })
            .click();
        await page.waitForFunction(() =>
            document.documentElement.classList.contains("dark"),
        );
        check(
            await page
                .locator(".project-panel")
                .first()
                .evaluate(
                    (el) =>
                        getComputedStyle(el).backgroundColor ===
                        "rgb(23, 33, 39)",
                ),
            "dark panels use the dark theme surface",
        );
        await page.screenshot({
            path: path.join(output, "desktop-dark.png"),
            fullPage: true,
        });
        rich = true;
        await page.getByRole("button", { name: "Segarkan ringkasan" }).click();
        await page.getByText("release-2026-10-16", { exact: true }).waitFor();
        await page.screenshot({
            path: path.join(output, "desktop-dark-history.png"),
            fullPage: true,
        });
        check(
            await page
                .getByText("Sertifikat segera kedaluwarsa", { exact: true })
                .isVisible(),
            "contextual warnings remain visible with populated history",
        );
        await page.locator('main').evaluate(el => { el.scrollTop = el.scrollHeight; });
        await page.screenshot({ path: path.join(output, 'desktop-dark-bottom.png') });
        await page.locator('main').evaluate(el => { el.scrollTop = 0; });
        for (const width of [390, 320]) {
            await page.setViewportSize({ width, height: 844 });
            check(
                await page.evaluate(
                    () => document.documentElement.scrollWidth <= innerWidth,
                ),
                `no page overflow at ${width}px`,
            );
            check(
                await page
                    .locator(".project-overview")
                    .evaluate((el) => el.scrollWidth <= el.clientWidth),
                `overview fits ${width}px`,
            );
            await page.screenshot({
                path: path.join(output, `mobile-${width}-dark.png`),
                fullPage: true,
            });
        }
        await page.evaluate(() =>
            document.documentElement.classList.remove("dark"),
        );
        await page.screenshot({
            path: path.join(output, "mobile-320-light.png"),
            fullPage: true,
        });
        zero = true;
        await page.getByRole("button", { name: "Segarkan ringkasan" }).click();
        await page.waitForFunction(
            () =>
                document
                    .querySelector(
                        '.project-meter[aria-label="Penggunaan memori"]',
                    )
                    ?.getAttribute("aria-valuenow") === "0",
        );
        check(
            await page
                .locator(".project-storage-bar > span")
                .evaluateAll((elements) =>
                    elements.every((el) => el.style.width === "0%"),
                ),
            "zero storage usage has no artificial segments",
        );
        overviewFailure = true;
        await open(`/sites/${domain}/overview`);
        await page
            .getByRole("heading", { name: "Ringkasan belum tersedia" })
            .waitFor();
        check(
            await page
                .getByRole("button", { name: "Coba lagi", exact: true })
                .isVisible(),
            "failed overview offers retry instead of fabricated metrics",
        );
        overviewFailure = false;
        await page
            .getByRole("button", { name: "Coba lagi", exact: true })
            .click();
        await ready();
        check(true, "retry recovers the overview");
        await open("/sites/unavailable.example/overview");
        await page
            .getByRole("heading", { name: "Detail situs tidak tersedia" })
            .waitFor();
        check(
            !calls.includes("/api/v1/sites/unavailable.example"),
            "unknown domain does not become a raw API identifier",
        );
        listFailure = true;
        await open(`/sites/${domain}/overview`);
        await page
            .getByRole("button", { name: "Coba lagi", exact: true })
            .waitFor();
        listFailure = false;
        await page
            .getByRole("button", { name: "Coba lagi", exact: true })
            .click();
        await ready();
        check(true, "domain resolution can recover from a list API failure");
        check(errors.length === 0, `no runtime errors: ${errors.join("; ")}`);
        console.log(`PASS: ${checks} browser checks`);
    } finally {
        await browser.close();
    }
})().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
