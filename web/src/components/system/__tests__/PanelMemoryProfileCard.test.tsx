import { describe, it, expect } from "vitest";
import { renderToString } from "react-dom/server";
import { PanelMemoryProfileCard } from "../PanelMemoryProfileCard";
import type { PanelMemoryProfileReport } from "../../../types/api";

describe("PanelMemoryProfileCard Component", () => {
  const mockReport: PanelMemoryProfileReport = {
    generated_at: "1720000000-epoch-sec",
    allocator: "mimalloc",
    release_profile: {
      opt_level: "3",
      lto: "fat",
      codegen_units: 1,
      panic_strategy: "abort",
      strip: true,
      memory_allocator: "mimalloc",
    },
    api_process: {
      pid: 1001,
      rss_bytes: 32 * 1024 * 1024,
      rss_mb: 32.0,
      vsize_bytes: 90 * 1024 * 1024,
      vsize_mb: 90.0,
      allocator: "mimalloc",
      timestamp_epoch_secs: 1720000000,
    },
    agent_process: {
      pid: 1002,
      rss_bytes: 14 * 1024 * 1024,
      rss_mb: 14.0,
      vsize_bytes: 45 * 1024 * 1024,
      vsize_mb: 45.0,
      allocator: "mimalloc",
      timestamp_epoch_secs: 1720000000,
    },
    total_panel_rss_mb: 46.0,
    bundle_gzip_kb: 145.46,
    cold_start_api_ms: 380,
    api_read_latency_p95_ms: 12.5,
    targets: [
      {
        target_id: "BAB9-RAM-API",
        name: "RAM Idle panel-api",
        threshold: "≤ 40.0 MB RSS",
        measured: "32.0 MB RSS",
        passed: true,
        notes: "Memori proses REST API saat kondisi standby (mimalloc)",
      },
      {
        target_id: "BAB9-RAM-AGENT",
        name: "RAM Idle panel-agent",
        threshold: "≤ 20.0 MB RSS",
        measured: "14.0 MB RSS",
        passed: true,
        notes: "Memori root daemon agent saat standby (mimalloc)",
      },
      {
        target_id: "BAB9-RAM-TOTAL",
        name: "RAM Total Panel (100 situs, 10 sesi)",
        threshold: "≤ 150.0 MB RSS",
        measured: "46.0 MB RSS",
        passed: true,
        notes: "Total footprint panel-api + panel-agent saat beban aktif",
      },
      {
        target_id: "BAB9-COLD-START",
        name: "Start Cold panel-api",
        threshold: "< 1000 ms",
        measured: "380 ms",
        passed: true,
        notes: "Waktu inisialisasi cold start",
      },
      {
        target_id: "BAB9-API-LATENCY",
        name: "Latensi API Baca (p95)",
        threshold: "< 50.0 ms",
        measured: "12.50 ms",
        passed: true,
        notes: "Respons endpoint baca di bawah 50ms",
      },
      {
        target_id: "BAB9-WEB-BUNDLE",
        name: "Web UI Initial Bundle (gzip)",
        threshold: "≤ 250.0 KB",
        measured: "145.46 KB",
        passed: true,
        notes: "Ukuran bundle entry point gzip",
      },
    ],
    all_passed: true,
    summary:
      "Semua 6 target efisiensi memori & performa PRD Bab 9 terpenuhi secara valid.",
  };

  it("renders initial report without crashing", () => {
    const html = renderToString(
      <PanelMemoryProfileCard initialReport={mockReport} />,
    );

    expect(html).toContain("Profil Memori &amp; Efisiensi Sistem");
    expect(html).toContain("100% Target Lolos");
    expect(html).not.toMatch(/mimalloc|Release Profile|PRD Bab 9/);
    expect(html).not.toContain("opt-level=3");
    expect(html).not.toContain("LTO=fat");
    expect(html).not.toContain("panic=abort");
  });

  it("displays operational measurements and statuses without internal requirement IDs", () => {
    const html = renderToString(
      <PanelMemoryProfileCard initialReport={mockReport} />,
    );

    for (const target of mockReport.targets) {
      expect(html).toContain(target.measured);
      expect(html).not.toContain(target.target_id);
    }
    expect(html).toContain("Lolos");
  });

  it("displays accurate measured metric values in cards", () => {
    const html = renderToString(
      <PanelMemoryProfileCard initialReport={mockReport} />,
    );

    expect(html).toContain("32.0 MB");
    expect(html).toContain("14.0 MB");
    expect(html).toContain("46.0 MB");
    expect(html).toContain("380 ms");
    expect(html).toContain("12.5 ms");
    expect(html).toContain("145.5 KB");
  });
});
