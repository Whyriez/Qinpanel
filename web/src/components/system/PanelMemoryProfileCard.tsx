import React, { useEffect, useState } from "react";
import { fetchMemoryProfile } from "../../api/client";
import type { PanelMemoryProfileReport } from "../../types/api";
import {
  Cpu,
  Database,
  CheckCircle2,
  AlertTriangle,
  RefreshCw,
  Download,
  Layers,
  Zap,
  Activity,
  HardDrive,
} from "lucide-react";

interface PanelMemoryProfileCardProps {
  initialReport?: PanelMemoryProfileReport;
  onRefresh?: () => void;
}

export const PanelMemoryProfileCard: React.FC<PanelMemoryProfileCardProps> = ({
  initialReport,
  onRefresh,
}) => {
  const [report, setReport] = useState<PanelMemoryProfileReport | null>(
    initialReport || null,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadData = async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await fetchMemoryProfile();
      setReport(data);
      if (onRefresh) onRefresh();
    } catch (err: any) {
      setError(
        err?.message ||
          "Gagal memuat metrik profil memori panel. Pastikan Anda memiliki hak akses administrator.",
      );
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!initialReport) {
      loadData();
    }
  }, [initialReport]);

  const handleDownloadMarkdown = () => {
    if (!report) return;

    let md = `# Laporan Profil Memori & Validasi RAM PRD Bab 9\n\n`;
    md += `**Waktu**: ${report.generated_at}\n`;
    md += `**Memory Allocator**: ${report.allocator}\n`;
    md += `**Release Profile**: opt-level=${report.release_profile.opt_level}, lto=${report.release_profile.lto}, codegen-units=${report.release_profile.codegen_units}, panic=${report.release_profile.panic_strategy}, strip=${report.release_profile.strip}\n\n`;
    md += `## 1. Hasil Evaluasi Target Non-Fungsional (PRD Bab 9)\n\n`;
    md += `| ID Target | Metrik | Ambang Batas | Terukur | Status | Catatan |\n`;
    md += `|---|---|---|---|---|---|\n`;
    report.targets.forEach((t) => {
      md += `| ${t.target_id} | ${t.name} | ${t.threshold} | ${t.measured} | ${t.passed ? "PASS" : "FAIL"} | ${t.notes} |\n`;
    });
    md += `\n## 2. Kesimpulan\n\n${report.summary}\n`;

    const blob = new Blob([md], { type: "text/markdown;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `memory-profile-report-${Date.now()}.md`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
  };

  return (
    <div
      className="bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 shadow-sm p-6"
      data-testid="panel-memory-profile-card"
    >
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-5 border-b border-slate-200 dark:border-slate-800">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-lg font-semibold text-slate-900 dark:text-white">
              Profil Memori & Efisiensi Sistem
            </h2>
            {report && (
              <span
                className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${
                  report.all_passed
                    ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/60 dark:text-emerald-400"
                    : "bg-amber-100 text-amber-800 dark:bg-amber-950/60 dark:text-amber-400"
                }`}
              >
                {report.all_passed ? "100% Target Lolos" : "Perlu Optimasi"}
              </span>
            )}
          </div>
          <p className="text-sm text-slate-500 dark:text-slate-400 mt-1">
            Validasi kepatuhan batas konsumsi RAM, cold start, dan ukuran bundel
            antarmuka sesuai standar arsitektur Qinpanel.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={loadData}
            disabled={loading}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-slate-700 dark:text-slate-200 bg-slate-100 dark:bg-slate-800 hover:bg-slate-200 dark:hover:bg-slate-700 rounded-lg transition-colors disabled:opacity-60"
            aria-label="Segarkan profil memori"
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`}
            />
            Segarkan
          </button>
          <button
            type="button"
            onClick={handleDownloadMarkdown}
            disabled={!report || loading}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-slate-700 dark:text-slate-200 bg-slate-100 dark:bg-slate-800 hover:bg-slate-200 dark:hover:bg-slate-700 rounded-lg transition-colors disabled:opacity-60"
            aria-label="Unduh laporan markdown"
          >
            <Download className="w-3.5 h-3.5" />
            Ekspor MD
          </button>
        </div>
      </div>

      {error && (
        <div className="mt-4 p-3 bg-red-50 dark:bg-red-950/30 border border-red-200 dark:border-red-900 rounded-lg flex items-center gap-2 text-sm text-red-700 dark:text-red-300">
          <AlertTriangle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Grid KPI Cards */}
      {report && (
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-3 mt-4">
          {/* Card 1: RAM Idle API */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Memori panel</span>
              <Cpu className="w-3.5 h-3.5 text-blue-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {report.api_process
                ? `${report.api_process.rss_mb.toFixed(1)} MB`
                : "-"}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: ≤ 40 MB RSS
            </div>
          </div>

          {/* Card 2: RAM Idle Agent */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Memori layanan</span>
              <HardDrive className="w-3.5 h-3.5 text-cyan-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {report.agent_process
                ? `${report.agent_process.rss_mb.toFixed(1)} MB`
                : "-"}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: ≤ 20 MB RSS
            </div>
          </div>

          {/* Card 3: Total Panel Load */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Total RAM Panel</span>
              <Layers className="w-3.5 h-3.5 text-purple-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {`${report.total_panel_rss_mb.toFixed(1)} MB`}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: ≤ 150 MB RSS
            </div>
          </div>

          {/* Card 4: Cold Start */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Cold Start API</span>
              <Activity className="w-3.5 h-3.5 text-amber-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {report.cold_start_api_ms
                ? `${report.cold_start_api_ms} ms`
                : "< 1s"}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: &lt; 1000 ms
            </div>
          </div>

          {/* Card 5: API Latency p95 */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Latensi Baca p95</span>
              <Zap className="w-3.5 h-3.5 text-emerald-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {report.api_read_latency_p95_ms
                ? `${report.api_read_latency_p95_ms.toFixed(1)} ms`
                : "< 50 ms"}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: &lt; 50 ms
            </div>
          </div>

          {/* Card 6: Bundle Web UI */}
          <div className="p-3.5 bg-white dark:bg-slate-800 rounded-lg border border-slate-200 dark:border-slate-700/80">
            <div className="flex items-center justify-between text-slate-500 dark:text-slate-400 mb-1">
              <span className="text-xs font-medium">Bundle Web (gzip)</span>
              <Database className="w-3.5 h-3.5 text-teal-500" />
            </div>
            <div className="text-lg font-bold text-slate-900 dark:text-white">
              {`${report.bundle_gzip_kb.toFixed(1)} KB`}
            </div>
            <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
              Target: ≤ 250 KB gzip
            </div>
          </div>
        </div>
      )}

      {/* Target Compliance Table */}
      {report && (
        <div className="mt-5 overflow-x-auto rounded-lg border border-slate-200 dark:border-slate-800">
          <table className="w-full text-left text-sm">
            <thead className="bg-slate-50 dark:bg-slate-800/60 text-slate-600 dark:text-slate-300 text-xs uppercase font-medium">
              <tr>
                <th className="px-4 py-3">Pengukuran</th>
                <th className="px-4 py-3">Ambang Batas</th>
                <th className="px-4 py-3">Terukur</th>
                <th className="px-4 py-3">Status</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-200 dark:divide-slate-800 text-xs">
              {report.targets.map((t) => (
                <tr
                  key={t.target_id}
                  className="hover:bg-slate-50/50 dark:hover:bg-slate-800/40 transition-colors"
                >
                  <td className="px-4 py-3 text-slate-800 dark:text-slate-200">
                    {{
                      "BAB9-RAM-API": "Memori panel",
                      "BAB9-RAM-AGENT": "Memori layanan server",
                      "BAB9-RAM-TOTAL": "Total memori panel",
                      "BAB9-COLD-START": "Waktu mulai",
                      "BAB9-API-LATENCY": "Waktu respons",
                      "BAB9-WEB-BUNDLE": "Ukuran antarmuka",
                    }[t.target_id] || t.name}
                  </td>
                  <td className="px-4 py-3 font-mono text-slate-600 dark:text-slate-400">
                    {t.threshold}
                  </td>
                  <td className="px-4 py-3 font-mono font-semibold text-slate-900 dark:text-white">
                    {t.measured}
                  </td>
                  <td className="px-4 py-3">
                    <span
                      className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-medium ${
                        t.passed
                          ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/70 dark:text-emerald-400"
                          : "bg-red-100 text-red-800 dark:bg-red-950/70 dark:text-red-400"
                      }`}
                    >
                      {t.passed ? (
                        <>
                          <CheckCircle2 className="w-3 h-3" />
                          Lolos
                        </>
                      ) : (
                        <>
                          <AlertTriangle className="w-3 h-3" />
                          Gagal
                        </>
                      )}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Summary Note */}
      {report && (
        <div className="mt-4 text-xs text-slate-500 dark:text-slate-400 flex items-center justify-between">
          <p>
            {report.all_passed
              ? "Seluruh pengukuran berada dalam batas yang ditetapkan."
              : "Beberapa pengukuran memerlukan peninjauan."}
          </p>
          <span className="font-mono text-[11px]">
            Waktu: {report.generated_at}
          </span>
        </div>
      )}
    </div>
  );
};
