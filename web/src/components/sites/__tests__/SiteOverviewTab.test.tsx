import { describe, it, expect, vi } from "vitest";
import { renderToString } from "react-dom/server";
import { SiteOverviewTab, SiteOverviewContent } from "../SiteOverviewTab";
import { Site, SiteOverviewResponse } from "../../../types/api";
import "../../../i18n";

describe("SiteOverviewTab Component", () => {
  const mockSite: Site = {
    id: 42,
    user_id: 1,
    primary_domain: "mysite.test",
    kind: "php",
    version: "8.3",
    preset: "standard",
    docroot: "/var/www/vhosts/p_user/mysite.test",
    status: "active",
    spa_fallback: false,
    force_https: true,
    created_at: 1770000000,
    updated_at: 1770000000,
    domains: [],
  };

  it("renders initial loading state cleanly", () => {
    const html = renderToString(
      <SiteOverviewTab site={mockSite} onNavigateTab={vi.fn()} />,
    );

    expect(html).toContain("Memuat ringkasan situs...");
  });

  const overview: SiteOverviewResponse = {
    site_id: 42,
    domain: "mysite.test",
    status: {
      webserver_status: "active",
      runtime_status: "running",
      healthy: true,
      uptime_secs: 3600,
      pid: 123,
    },
    resources: {
      cpu_usage_usec: 0,
      cpu_percent: 3.2,
      memory_bytes: 0,
      memory_limit_bytes: 1024,
      pids_current: 0,
      pids_limit: 100,
    },
    ssl: {
      has_certificate: true,
      issuer: "Let's Encrypt",
      days_remaining: 80,
      valid_until: 1800000000,
      auto_renew: true,
      status: "valid",
      force_https: true,
    },
    runtime: {
      kind: "php",
      version: "8.3",
      entrypoint_or_target: "/home/p_user/sites/mysite.test/public",
    },
    storage: {
      docroot_bytes: 0,
      databases_bytes: 0,
      backups_bytes: 0,
      total_site_bytes: 0,
      plan_disk_limit_bytes: 1024,
      usage_percentage: 0,
    },
    quick_actions_available: ["restart", "reload"],
    warnings: [],
    latest_deployment: null,
    latest_backup: null,
  };
  const render = (data = overview) =>
    renderToString(
      <SiteOverviewContent
        site={mockSite}
        overview={data}
        onNavigateTab={vi.fn()}
        onQuickAction={vi.fn()}
        onRefresh={vi.fn()}
      />,
    );

  it("shows actual status, available actions and useful empty states", () => {
    const html = render();
    expect(html).toContain("Situs beroperasi normal");
    expect(html).toContain("Berjalan");
    expect(html).toContain("Muat ulang Nginx");
    expect(html).not.toContain(">Mulai ulang aplikasi</button>");
    expect(html).not.toContain("Hentikan");
    expect(html).toContain("Belum ada deployment");
    expect(html).toContain("Belum ada cadangan");
    expect(html).toContain('href="https://mysite.test"');
    expect(html).not.toMatch(/bg-slate|text-white|backdrop-blur|shadow-lg/);
  });
  it("shows configured access and runtime independently and prevents starting an already running app", () => {
    const html = render({ ...overview, runtime: { ...overview.runtime, kind: "nodejs" }, quick_actions_available: ["start", "stop", "restart", "reload"] });
    expect(html).toContain("Akses situs:");
    expect(html).toContain("Proses aplikasi");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>.*?Mulai<\/button>/);
    expect(html).toContain("Mulai ulang aplikasi");
    expect(html).toContain("cache aplikasi tidak dihapus");
  });
  it("shows a stopped app and disables stop/restart while allowing start", () => {
    const html = render({ ...overview, status: { ...overview.status, webserver_status: "suspended", runtime_status: "inactive", healthy: false }, runtime: { ...overview.runtime, kind: "python" }, quick_actions_available: ["start", "stop", "restart", "reload"] });
    expect(html).toContain("Situs dinonaktifkan");
    expect(html).toContain("Berhenti");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>.*?Hentikan<\/button>/);
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>.*?Mulai ulang aplikasi<\/button>/);
    expect(html).toMatch(/<button class="project-button">.*?Mulai<\/button>/);
  });
  it("does not call an unavailable agent stopped or invent a process for static sites", () => {
    const unknown = render({ ...overview, status: { ...overview.status, runtime_status: "unknown", healthy: false } });
    expect(unknown).toContain("Belum diketahui");
    expect(unknown).not.toContain(">Berhenti<");
    const staticSite = render({ ...overview, runtime: { ...overview.runtime, kind: "static" }, status: { ...overview.status, runtime_status: "not_applicable" } });
    expect(staticSite).toContain("Situs diaktifkan");
    expect(staticSite).not.toContain("Proses aplikasi:");
    expect(staticSite).not.toContain("Situs beroperasi normal");
  });
  it("keeps the PHP master separate from site access and hides unsupported cache/restart controls", () => {
    const html = render({ ...overview, quick_actions_available: ["start", "stop", "restart", "reload", "clear_cache"] });
    expect(html).toContain("PHP-FPM");
    expect(html).toContain("dipakai situs lain");
    expect(html).toContain("Nonaktifkan situs");
    expect(html).not.toContain("Bersihkan cache");
    expect(html).not.toContain(">Mulai ulang aplikasi</button>");
  });
  it("announces the exact pending action and blocks stale controls", () => {
    const html = renderToString(<SiteOverviewContent site={mockSite} overview={overview} onNavigateTab={vi.fn()} onQuickAction={vi.fn()} onRefresh={vi.fn()} actionInProgress="stop" statusStale />);
    expect(html).toContain("Menonaktifkan situs...");
    expect(html).toContain("Menunggu hasil dan pemeriksaan status");
    expect(html).toContain("Status terakhir belum dapat diperbarui");
    expect(html).toContain('aria-live="polite"');
  });
  it("keeps zero usage empty and does not invent tenant limits", () => {
    expect(render()).toContain('aria-valuenow="0"');
    const html = render({
      ...overview,
      resources: {
        ...overview.resources,
        memory_limit_bytes: 0,
        pids_limit: 0,
      },
      storage: { ...overview.storage, plan_disk_limit_bytes: 0 },
    });
    expect(html).toContain("Tanpa batas terkonfigurasi");
    expect(html).toContain("Kuota penyimpanan belum ditetapkan");
    expect(html).not.toContain("aria-valuenow");
    expect(html).not.toContain("1 GB");
    expect(html).not.toContain("10 GB");
  });
  it("does not force HTTPS without a certificate and shows warnings and failed history", () => {
    const html = render({
      ...overview,
      ssl: {
        ...overview.ssl,
        has_certificate: false,
        status: "none",
        issuer: null,
      },
      warnings: [
        {
          id: "ssl",
          level: "warning",
          title: "Sertifikat belum tersedia",
          message: "Terbitkan sertifikat untuk HTTPS.",
          action_tab: "ssl",
        },
      ],
      latest_backup: {
        backup_id: 1,
        filename: "backup.tar.gz",
        size_bytes: 1024,
        status: "failed",
        includes_files: true,
        includes_databases: false,
        created_at: 1770000000,
      },
    });
    expect(html).toContain('href="http://mysite.test"');
    expect(html).toContain("Sertifikat belum tersedia");
    expect(html).toContain("backup.tar.gz");
    expect(html).toContain("Gagal");
  });
});
