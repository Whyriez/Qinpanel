/* Local UI QA only. All API requests are intercepted; no panel services are run. */
const { chromium } = require("playwright");
const { mkdirSync, writeFileSync } = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");

const baseURL = process.env.NUSA_UI_URL || "http://127.0.0.1:5178";
const output = path.resolve(__dirname, "../artifacts/design");
mkdirSync(output, { recursive: true });
const user = {
    id: 1,
    username: "admin",
    role: "admin",
    email: "admin@example.test",
    totp_enabled: true,
};
const sites = ["studio.example", "api.example", "docs.example"].map(
    (domain, index) => ({
        id: index + 1,
        user_id: 1,
        primary_domain: domain,
        kind: ["php", "nodejs", "static"][index],
        version: ["8.3", "22", null][index],
        docroot: "/home/test/sites",
        status: index === 2 ? "disabled" : "active",
        spa_fallback: false,
        force_https: true,
        domains: [
            {
                id: index + 1,
                site_id: index + 1,
                name: domain,
                is_primary: true,
            },
        ],
        created_at: 1,
        updated_at: 1,
    }),
);
const now = Math.floor(Date.now() / 1000);
const metrics = {
    from_ts: now - 3600,
    to_ts: now,
    interval: "1m",
    points: Array.from({ length: 30 }, (_, index) => ({
        timestamp: now - (29 - index) * 60,
        cpu_avg: 12 + Math.sin(index * 0.6) * 4 + index / 3,
        cpu_max: 35,
        memory_used_avg_bytes: 1.4 * 1024 ** 3,
        memory_used_max_bytes: 2 * 1024 ** 3,
        swap_used_avg_bytes: 0,
        load1_avg: 0.5,
        load1_max: 1,
        disk_used_avg_bytes: 24.6 * 1024 ** 3,
        net_rx_avg_bytes_sec: 220000 + index * 3000,
        net_tx_avg_bytes_sec: 180000,
        io_read_avg_bytes_sec: 0,
        io_write_avg_bytes_sec: 0,
    })),
};
let assertions = 0;
const errors = [];
function check(condition, message) {
    assert.ok(condition, message);
    assertions++;
}
async function fixture(page, mode = "normal", role = "admin") {
    await page.route("**/api/v1/**", async (route) => {
        const url = new URL(route.request().url());
        let body = {};
        let status = 200;
        if (url.pathname.endsWith("/auth/csrf"))
            body = { csrf_token: "local-ui-fixture" };
        else if (url.pathname.endsWith("/auth/me")) {
            body = { user: { ...user, role } };
            if (mode === "login") status = 401;
        } else if (url.pathname.endsWith("/system/info")) {
            body = {
                api_version: "0.1.1",
                agent_version: "0.1.1",
                proto_version: 1,
                os: "linux",
                arch: "x86_64",
                agent_connected: true,
                environment: "production",
            };
            if (mode === "error") status = 503;
        } else if (url.pathname === "/api/v1/sites") {
            body = mode === "empty" ? [] : sites;
            if (mode === "error") status = 503;
        } else if (url.pathname.endsWith("/system/analytics/timeseries")) {
            body = mode === "empty" ? { ...metrics, points: [] } : metrics;
            if (mode === "error") status = 503;
        } else if (url.pathname.includes("/analytics/timeseries"))
            body = { points: [] };
        else if (url.pathname.includes("unread-count"))
            body = { count: 0, unread_count: 0 };
        else if (url.pathname.includes("/jobs")) body = { items: [], total: 0 };
        else if (url.pathname.includes("/tenants")) body = { tenants: [] };
        else if (url.pathname.includes("/notifications"))
            body = { notifications: [], total: 0 };
        await route.fulfill({
            status,
            contentType: "application/json",
            body: JSON.stringify(
                status === 503 ? { message: "Local QA error fixture" } : body,
            ),
        });
    });
}
async function open(
    browser,
    { theme = "light", width = 1440, mode = "normal", role = "admin" } = {},
) {
    const page = await browser.newPage({
        viewport: { width, height: 940 },
        reducedMotion: "reduce",
    });
    page.on("pageerror", (error) => {
        errors.push(error.message);
        console.error("Browser error:", error.message);
    });
    await page.addInitScript(
        (value) => localStorage.setItem("nusa_theme", value),
        theme,
    );
    await fixture(page, mode, role);
    await page.goto(baseURL, { waitUntil: "domcontentloaded" });
    try {
        await page
            .locator(mode === "login" ? "#login-username" : ".hosting-summary")
            .waitFor({ timeout: 15000 });
    } catch (error) {
        console.error((await page.locator("body").innerText()).slice(0, 1800));
        await page.screenshot({ path: path.join(output, "debug-failure.png") });
        throw error;
    }
    await page.evaluate(() => document.fonts.ready);
    return page;
}
async function noOverflow(page) {
    const overflowing = await page.evaluate(() =>
        Array.from(document.querySelectorAll("body *"))
            .map((element) => ({
                tag: element.tagName,
                class: element.className,
                right: element.getBoundingClientRect().right,
            }))
            .filter((element) => element.right > innerWidth + 1)
            .slice(0, 12),
    );
    if (
        await page.evaluate(
            () => document.documentElement.scrollWidth > innerWidth + 1,
        )
    )
        console.log("Overflow candidates:", JSON.stringify(overflowing));
    check(
        await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth + 1,
        ),
        "No viewport horizontal overflow",
    );
}

(async () => {
    const browser = await chromium.launch({
        executablePath: process.env.NUSA_BROWSER_PATH,
        headless: true,
    });
    try {
        for (const theme of ["light", "dark"]) {
            const page = await open(browser, { theme });
            await page.getByText("studio.example", { exact: true }).waitFor();
            await noOverflow(page);
            const text = await page.locator("body").innerText();
            check(
                !/Rust|axum|Phase 0|vps-01|WCAG/.test(text),
                "No development or implementation copy",
            );
            check(
                text.includes("3 situs terdaftar, 2 aktif"),
                "Site counts use API data",
            );
            check(
                await page.evaluate(() =>
                    document.fonts.check('500 14px "Plus Jakarta Sans"'),
                ),
                "Self-hosted font available",
            );
            await page.screenshot({
                path: path.join(output, `dashboard-${theme}.png`),
                fullPage: true,
            });
            if (theme === "light") {
                const themeButton = page.getByRole("button", {
                    name: "Ganti tema",
                    exact: true,
                });
                await themeButton.click();
                check(
                    await page.evaluate(
                        () =>
                            document.documentElement.classList.contains(
                                "dark",
                            ) && localStorage.getItem("nusa_theme") === "dark",
                    ),
                    "Theme control applies and persists dark theme",
                );
                await themeButton.click();
                await themeButton.click();
                check(
                    await page.evaluate(
                        () => localStorage.getItem("nusa_theme") === "light",
                    ),
                    "Theme cycle returns to light",
                );
                await page
                    .getByRole("button", { name: "Ganti bahasa", exact: true })
                    .click();
                await page
                    .getByText("Hosting overview", { exact: true })
                    .waitFor();
                check(
                    await page.evaluate(
                        () => document.documentElement.lang === "en",
                    ),
                    "English copy and document language agree",
                );
                await page
                    .getByRole("button", {
                        name: "Change language",
                        exact: true,
                    })
                    .click();
                await page
                    .getByText("Ringkasan hosting", { exact: true })
                    .waitFor();
            }
            await page.keyboard.press("Control+k");
            await page.getByRole("combobox").waitFor();
            check(
                await page.getByRole("combobox").isVisible(),
                "Command palette keyboard shortcut",
            );
            await page.keyboard.press("Escape");
            await page
                .getByRole("button", { name: "Situs", exact: true })
                .first()
                .click();
            await page.getByPlaceholder("Cari nama domain...").waitFor();
            await page
                .getByPlaceholder("Cari nama domain...")
                .fill("api.example");
            check(
                (await page.locator("tbody tr").count()) === 1,
                "Website filtering works",
            );
            await page.screenshot({
                path: path.join(output, `sites-${theme}.png`),
                fullPage: true,
            });
            await page.close();
        }
        for (const width of [390, 320, 1024]) {
            const page = await open(browser, { width });
            await page.getByText("studio.example", { exact: true }).waitFor();
            await noOverflow(page);
            if (width < 768) {
                await page
                    .getByRole("button", { name: "Buka navigasi", exact: true })
                    .first()
                    .click();
                check(
                    await page.locator(".panel-sidebar").isVisible(),
                    "Mobile navigation opens",
                );
                await page.waitForFunction(() => Boolean(document.activeElement.closest('.panel-sidebar')));
                await page.keyboard.press("Shift+Tab");
                check(
                    await page.evaluate(() =>
                        Boolean(
                            document.activeElement.closest(".panel-sidebar"),
                        ),
                    ),
                    "Mobile drawer keeps keyboard focus inside navigation",
                );
                await page.locator(".mobile-close").click();
                check(
                    !(await page.locator(".panel-sidebar").isVisible()),
                    "Mobile navigation closes",
                );
            }
            await page.screenshot({
                path: path.join(output, `dashboard-${width}.png`),
                fullPage: true,
            });
            await page.close();
        }
        for (const mode of ["empty", "error"]) {
            const page = await open(browser, { mode });
            await page
                .getByText(
                    mode === "empty"
                        ? "Mulai dengan situs pertama Anda"
                        : "Daftar situs belum dapat dimuat",
                )
                .first()
                .waitFor();
            check(
                !(await page
                    .getByText("studio.example", { exact: true })
                    .count()),
                "Unavailable sites do not become example sites",
            );
            check(
                !(await page
                    .locator(".resource-value")
                    .filter({ hasText: /\d/ })
                    .count()),
                "Unavailable measurements do not become fabricated values",
            );
            await page.screenshot({
                path: path.join(output, `dashboard-${mode}.png`),
                fullPage: true,
            });
            await page.close();
        }
        for (const width of [1440, 390]) {
            const page = await open(browser, { mode: "login", width });
            await page.getByLabel("Nama pengguna", { exact: true }).waitFor();
            await noOverflow(page);
            check(
                !/Rust|WCAG|alpha/.test(await page.locator("body").innerText()),
                "Login has no implementation claims",
            );
            await page.screenshot({
                path: path.join(output, `login-${width}.png`),
                fullPage: true,
            });
            await page.close();
        }
        const tenantPage = await open(browser, { role: "user" });
        check(
            !(await tenantPage
                .getByRole("button", { name: "Pengaturan", exact: true })
                .count()),
            "Tenant navigation excludes admin settings",
        );
        check(
            !(await tenantPage.locator(".resource-grid").count()),
            "Tenant overview excludes admin host measurements",
        );
        await tenantPage.keyboard.press("Control+k");
        await tenantPage.getByRole("combobox").waitFor();
        check(
            !(await tenantPage
                .getByRole("option", { name: "Pengaturan", exact: true })
                .count()),
            "Tenant command palette excludes admin settings",
        );
        await tenantPage.close();
        check(
            errors.length === 0,
            `No browser script errors: ${errors.join(", ")}`,
        );
        writeFileSync(
            path.join(output, "results.json"),
            JSON.stringify(
                {
                    assertions,
                    screenshotData:
                        "Mocked API fixtures only; no live server/agent validation",
                    errors,
                },
                null,
                2,
            ),
        );
        console.log(
            `${assertions} browser checks passed; screenshots: ${output}`,
        );
    } finally {
        await browser.close();
    }
})().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
