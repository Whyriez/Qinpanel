import { useState, useEffect, useCallback, useRef, type ReactNode } from "react";
import {
  Activity,
  AlertTriangle,
  Archive,
  ArrowRight,
  CheckCircle2,
  ExternalLink,
  Folder,
  GitBranch,
  Lock,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Server,
  Terminal,
} from "lucide-react";
import type { Site, SiteOverviewResponse } from "../../types/api";
import { fetchSiteOverview, executeSiteQuickAction } from "../../api/client";
import { StatusPill, type StatusVariant } from "../ui/StatusPill";

export interface SiteOverviewTabProps {
  site: Site;
  onNavigateTab: (tab: string) => void;
  onOpenTerminal?: () => void;
  onStatusChange?: (status: string) => void;
}

const formatBytes = (bytes?: number | null): string => {
  if (
    bytes === undefined ||
    bytes === null ||
    !Number.isFinite(bytes) ||
    bytes < 0
  )
    return "Belum tersedia";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(
    units.length - 1,
    Math.max(0, Math.floor(Math.log(bytes) / Math.log(1024))),
  );
  return `${Number((bytes / 1024 ** index).toFixed(1)).toLocaleString("id-ID")} ${units[index]}`;
};
const formatDate = (timestamp?: number | null) =>
  timestamp
    ? new Date(timestamp * 1000).toLocaleString("id-ID", {
        dateStyle: "medium",
        timeStyle: "short",
      })
    : "Belum tersedia";
const formatUptime = (seconds?: number | null) => {
  if (seconds === undefined || seconds === null) return "Belum tersedia";
  if (seconds < 60) return "Kurang dari 1 menit";
  const days = Math.floor(seconds / 86400),
    hours = Math.floor((seconds % 86400) / 3600),
    minutes = Math.floor((seconds % 3600) / 60);
  return days
    ? `${days} hari ${hours} jam`
    : hours
      ? `${hours} jam ${minutes} menit`
      : `${minutes} menit`;
};
const runtimeNames: Record<string, string> = {
  static: "Situs statis",
  php: "PHP",
  nodejs: "Node.js",
  python: "Python",
  proxy: "Reverse proxy",
};
const stateNames: Record<string, string> = {
  running: "Berjalan",
  inactive: "Berhenti",
  failed: "Bermasalah",
  not_applicable: "Tidak diperlukan",
  unknown: "Belum diketahui",
};
const historyNames: Record<string, string> = {
  success: "Berhasil",
  completed: "Selesai",
  running: "Berjalan",
  queued: "Mengantre",
  pending: "Menunggu",
  failed: "Gagal",
  cancelled: "Dibatalkan",
};
const historyVariant = (status: string): StatusVariant =>
  ["success", "completed"].includes(status)
    ? "success"
    : ["queued", "running", "pending"].includes(status)
      ? "info"
      : status === "failed"
        ? "danger"
        : "neutral";

function Panel({
  title,
  description,
  icon,
  children,
  action,
}: {
  title: string;
  description: string;
  icon: ReactNode;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <section className="project-panel">
      <header className="project-panel-heading">
        <div className="project-panel-title">
          {icon}
          <div>
            <h3>{title}</h3>
            <p>{description}</p>
          </div>
        </div>
        {action}
      </header>
      {children}
    </section>
  );
}
function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="project-detail-row">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}
function Meter({
  label,
  value,
  limit,
}: {
  label: string;
  value: number;
  limit?: number;
}) {
  const percentage =
    limit && limit > 0
      ? Math.min(100, Math.max(0, (value / limit) * 100))
      : null;
  return (
    <div
      className="project-meter"
      role={percentage === null ? undefined : "meter"}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percentage ?? undefined}
      aria-valuetext={
        percentage === null
          ? "Batas belum tersedia"
          : `${Math.round(percentage)}%`
      }
    >
      <span
        className={percentage !== null && percentage > 85 ? "is-high" : ""}
        style={{ width: `${percentage ?? 0}%` }}
      />
    </div>
  );
}

export const SiteOverviewTab = ({
  site,
  onNavigateTab,
  onOpenTerminal,
  onStatusChange,
}: SiteOverviewTabProps) => {
  const [overview, setOverview] = useState<SiteOverviewResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [actionInProgress, setActionInProgress] = useState<string | null>(null);
  const [message, setMessage] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);
  const [statusStale, setStatusStale] = useState(false);
  const sequence = useRef(0);
  const currentSite = useRef(site.id);
  const lifecycle = useRef(0);
  const busy = useRef(false);
  const fetching = useRef(false);
  const loadOverview = useCallback(async (silent = false) => {
    if (silent && fetching.current) return;
    const request = ++sequence.current;
    fetching.current = true;
    if (!silent) setIsLoading(true);
    try {
      const data = await fetchSiteOverview(site.id);
      if (request !== sequence.current || site.id !== currentSite.current) return;
      setOverview(data);
      if (["active", "suspended"].includes(data.status.webserver_status)) onStatusChange?.(data.status.webserver_status);
      setStatusStale(false);
    } catch (error: unknown) {
      if (request !== sequence.current || site.id !== currentSite.current) return;
      setStatusStale(true);
      if (!silent) setMessage({ type: "error", text: error instanceof Error ? error.message : "Gagal memuat ringkasan situs." });
    } finally {
      if (request === sequence.current) {
        fetching.current = false;
        setIsLoading(false);
      }
    }
  }, [site.id, onStatusChange]);
  useEffect(() => {
    ++lifecycle.current;
    currentSite.current = site.id;
    busy.current = false;
    setOverview(null);
    setMessage(null);
    setActionInProgress(null);
    setStatusStale(false);
    void loadOverview();
    return () => { ++sequence.current; ++lifecycle.current; };
  }, [site.id, loadOverview]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (!busy.current && document.visibilityState !== "hidden") void loadOverview(true);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [loadOverview]);
  const handleQuickAction = async (action: string) => {
    if (busy.current) return;
    const requestedSite = site.id;
    const generation = lifecycle.current;
    const isCurrent = () => requestedSite === currentSite.current && generation === lifecycle.current;
    busy.current = true;
    ++sequence.current;
    fetching.current = false;
    setActionInProgress(action);
    setMessage(null);
    try {
      const response = await executeSiteQuickAction(requestedSite, action);
      if (!isCurrent()) return;
      if (!response.success) throw new Error(response.message || "Aksi situs belum berhasil.");
      setMessage({ type: "success", text: response.message });
    } catch (error: unknown) {
      if (!isCurrent()) return;
      setMessage({ type: "error", text: error instanceof Error ? error.message : "Gagal menjalankan aksi situs." });
    } finally {
      if (isCurrent()) {
        // Always re-read, including failures and ambiguous network responses.
        await loadOverview();
        if (isCurrent()) {
          busy.current = false;
          setActionInProgress(null);
        }
      }
    }
  };
  return (
    <div
      className="project-overview"
      data-testid="site-overview-tab"
      aria-busy={isLoading || !!actionInProgress}
    >
      {message && (
        <div
          className={`project-notice ${message.type === "error" ? "is-danger" : "is-success"}`}
          role={message.type === "error" ? "alert" : "status"}
        >
          {message.type === "error" ? (
            <AlertTriangle size={18} />
          ) : (
            <CheckCircle2 size={18} />
          )}
          <p>{message.text}</p>
          <button
            onClick={() => setMessage(null)}
            aria-label="Tutup pemberitahuan"
          >
            Tutup
          </button>
        </div>
      )}
      {!overview ? (
        isLoading ? (
          <div className="project-loading" role="status">
            <RefreshCw size={20} className="animate-spin" />
            Memuat ringkasan situs...
          </div>
        ) : (
          <div className="project-empty">
            <h2>Ringkasan belum tersedia</h2>
            <p>Data situs belum berhasil dimuat.</p>
            <button
              className="project-button"
              onClick={() => void loadOverview()}
            >
              Coba lagi
            </button>
          </div>
        )
      ) : (
        <SiteOverviewContent
          site={site}
          overview={overview}
          onNavigateTab={onNavigateTab}
          onOpenTerminal={onOpenTerminal}
          onRefresh={() => void loadOverview()}
          isRefreshing={isLoading}
          statusStale={statusStale}
          actionInProgress={actionInProgress}
          onQuickAction={(action) => void handleQuickAction(action)}
        />
      )}
    </div>
  );
};

interface ContentProps extends SiteOverviewTabProps {
  overview: SiteOverviewResponse;
  onRefresh: () => void;
  isRefreshing?: boolean;
  statusStale?: boolean;
  actionInProgress?: string | null;
  onQuickAction: (action: string) => void;
}

export function SiteOverviewContent({
  site,
  overview,
  onNavigateTab,
  onOpenTerminal,
  onRefresh,
  isRefreshing,
  statusStale,
  actionInProgress,
  onQuickAction,
}: ContentProps) {
  const {
    status,
    resources,
    ssl,
    runtime,
    storage,
    latest_deployment: deploy,
    latest_backup: backup,
  } = overview;
  const hasLimit = storage.plan_disk_limit_bytes > 0;
  const diskPercentage = hasLimit
    ? Math.max(
        0,
        (storage.total_site_bytes / storage.plan_disk_limit_bytes) * 100,
      )
    : null;
  const runtimeName = runtimeNames[runtime.kind] || runtime.kind;
  const runtimeTab =
    runtime.kind === "nodejs"
      ? "node"
      : ["php", "python", "proxy"].includes(runtime.kind)
        ? runtime.kind
        : "settings";
  const hasAppProcess = ["nodejs", "python"].includes(runtime.kind);
  const hasRuntime = hasAppProcess || runtime.kind === "php";
  const enabled = status.webserver_status === "active";
  const stopped = status.webserver_status === "suspended";
  const running = status.runtime_status === "running";
  const inactive = status.runtime_status === "inactive";
  const pendingLabels: Record<string, string> = {
    start: hasAppProcess ? "Memulai aplikasi..." : "Mengaktifkan situs...",
    stop: hasAppProcess ? "Menghentikan aplikasi..." : "Menonaktifkan situs...",
    restart: "Memulai ulang aplikasi...",
    reload: "Memuat ulang Nginx...",
  };
  const serviceActions = [
    { id: "start", label: hasAppProcess ? "Mulai" : "Aktifkan situs", icon: Play,
      disabled: enabled && (!hasAppProcess || running) },
    { id: "stop", label: hasAppProcess ? "Hentikan" : "Nonaktifkan situs", icon: Pause,
      disabled: stopped && (!hasAppProcess || inactive) },
    { id: "restart", label: "Mulai ulang aplikasi", icon: RotateCcw, disabled: !enabled || !running },
    { id: "reload", label: "Muat ulang Nginx", icon: RefreshCw, disabled: false },
  ].filter((action) => overview.quick_actions_available.includes(action.id) && (action.id !== "restart" || hasAppProcess));
  const linkButton = (tab: string, label: string) => (
    <button className="project-text-button" onClick={() => onNavigateTab(tab)}>
      {label}
      <ArrowRight size={14} />
    </button>
  );
  const sslLabel =
    ssl.status === "valid"
      ? "Valid"
      : ssl.status === "expiring_soon"
        ? "Segera kedaluwarsa"
        : ssl.status === "expired"
          ? "Kedaluwarsa"
          : "Belum terpasang";
  const sslVariant: StatusVariant =
    ssl.status === "valid"
      ? "success"
      : ssl.status === "expiring_soon"
        ? "warning"
        : ssl.status === "expired"
          ? "danger"
          : "neutral";
  const segments = [
    {
      name: "Berkas situs",
      bytes: storage.docroot_bytes,
      className: "is-files",
    },
    {
      name: "Database",
      bytes: storage.databases_bytes,
      className: "is-database",
    },
    { name: "Cadangan", bytes: storage.backups_bytes, className: "is-backup" },
  ];
  const storageBase = Math.max(
    storage.plan_disk_limit_bytes,
    storage.total_site_bytes,
    segments.reduce((sum, item) => sum + item.bytes, 0),
    1,
  );
  return (
    <>
      <section
        className="project-service project-panel"
        aria-label="Status dan aksi situs"
      >
        <div className="project-service-heading">
          <div>
            <div className="project-eyebrow">Status situs</div>
            <div className="project-service-status">
              <h2>
                {stopped ? "Situs dinonaktifkan" : !hasRuntime && enabled ? "Situs diaktifkan" : status.healthy ? "Situs beroperasi normal" : "Situs perlu diperiksa"}
              </h2>
              <StatusPill
                variant={
                  status.webserver_status === "active" ? "success" : "neutral"
                }
                label={
                  status.webserver_status === "active"
                    ? "Akses diaktifkan"
                    : status.webserver_status === "suspended"
                      ? "Akses dinonaktifkan"
                      : "Belum diketahui"
                }
              />
            </div>
            <p>
              {runtimeName}
              {runtime.version ? ` ${runtime.version}` : ""}
              <span aria-hidden="true"> · </span>Uptime:{" "}
              {formatUptime(status.uptime_secs)}
            </p>
          </div>
          <div className="project-service-links">
            <button
              className="project-button project-icon-button"
              aria-label="Segarkan ringkasan"
              onClick={onRefresh}
              disabled={isRefreshing || !!actionInProgress}
            >
              <RefreshCw
                size={16}
                className={isRefreshing ? "animate-spin" : ""}
              />
            </button>
            <a
              className="project-button"
              href={`${ssl.has_certificate && site.force_https ? "https" : "http"}://${site.primary_domain}`}
              target="_blank"
              rel="noreferrer"
            >
              Kunjungi situs
              <ExternalLink size={14} />
            </a>
          </div>
        </div>
        <div className="project-service-actions">
          <div className="project-control-status" role="status" aria-live="polite">
            <span className="project-action-label">Kontrol layanan</span>
            <p>Akses situs: <strong>{enabled ? "Diaktifkan" : stopped ? "Dinonaktifkan" : "Belum diketahui"}</strong></p>
            {hasRuntime && <p>{hasAppProcess ? "Proses aplikasi" : "PHP-FPM"}: <strong>{stateNames[status.runtime_status] || "Belum diketahui"}</strong></p>}
            {actionInProgress && <p>{pendingLabels[actionInProgress] || "Memproses..."} Menunggu hasil dan pemeriksaan status.</p>}
            {statusStale && <p>Status terakhir belum dapat diperbarui. Segarkan untuk memeriksa kembali.</p>}
          </div>
          <div>
            {serviceActions.length ? (
              serviceActions.map((action) => (
                <button
                  key={action.id}
                  className="project-button"
                  disabled={!!actionInProgress || isRefreshing || statusStale || action.disabled}
                  onClick={() => onQuickAction(action.id)}
                >
                  <action.icon
                    size={14}
                    className={
                      actionInProgress === action.id ? "animate-spin" : ""
                    }
                  />
                  {actionInProgress === action.id
                    ? pendingLabels[action.id]
                    : action.label}
                </button>
              ))
            ) : (
              <span className="project-muted">
                Tidak ada aksi layanan yang tersedia.
              </span>
            )}
          </div>
        </div>
        <div className="project-control-help">
          {hasAppProcess ? <p><strong>Mulai ulang aplikasi</strong> menghentikan lalu menjalankan kembali proses {runtimeName}; aplikasi dapat terputus sebentar.</p> : <p>Aktifkan atau nonaktifkan akses situs melalui Nginx. {runtime.kind === "php" ? "PHP-FPM dapat dipakai situs lain dalam tenant yang sama, sehingga tidak dihentikan dari sini." : "Situs ini tidak memiliki proses aplikasi lokal yang dikontrol dari sini."}</p>}
          <p><strong>Muat ulang Nginx</strong> menerapkan konfigurasi webserver sambil menyelesaikan koneksi lama. Proses aplikasi tidak dimulai ulang dan cache aplikasi tidak dihapus.</p>
          <p>Status akses mengikuti konfigurasi tersimpan. Status proses diperiksa melalui agent setiap 5 detik; ini bukan pemeriksaan akses dari internet.</p>
        </div>
        <nav className="project-shortcuts" aria-label="Akses cepat project">
          <button onClick={() => onNavigateTab("files")}>
            <Folder size={16} />
            Berkas situs
            <ArrowRight size={14} />
          </button>
          {onOpenTerminal && (
            <button onClick={onOpenTerminal}>
              <Terminal size={16} />
              Terminal
              <ArrowRight size={14} />
            </button>
          )}
          <button onClick={() => onNavigateTab("backups")}>
            <Archive size={16} />
            Cadangan
            <ArrowRight size={14} />
          </button>
          <button onClick={() => onNavigateTab("deploy")}>
            <GitBranch size={16} />
            Deployment
            <ArrowRight size={14} />
          </button>
        </nav>
      </section>

      {overview.warnings.length > 0 && (
        <div
          className="project-warnings"
          data-testid="contextual-warnings-section"
        >
          {overview.warnings.map((warning) => (
            <div
              className={`project-notice ${warning.level === "danger" ? "is-danger" : warning.level === "info" ? "is-info" : "is-warning"}`}
              key={warning.id}
            >
              <AlertTriangle size={18} />
              <div>
                <h3>{warning.title}</h3>
                <p>{warning.message}</p>
              </div>
              {warning.action_tab && linkButton(warning.action_tab, "Periksa")}
            </div>
          ))}
        </div>
      )}

      <Panel
        title="Penggunaan sumber daya"
        description="Pemakaian bersama seluruh situs dalam akun tenant ini."
        icon={<Activity size={18} />}
      >
        <div className="project-metrics">
          <div className="project-metric">
            <span>CPU</span>
            <strong>
              {Number.isFinite(resources.cpu_percent)
                ? `${resources.cpu_percent.toLocaleString("id-ID", { maximumFractionDigits: 1 })}%`
                : "Belum tersedia"}
            </strong>
            <small>Penggunaan prosesor tenant</small>
          </div>
          <div className="project-metric">
            <span>Memori</span>
            <strong>{formatBytes(resources.memory_bytes)}</strong>
            <small>
              {resources.memory_limit_bytes > 0
                ? `dari ${formatBytes(resources.memory_limit_bytes)}`
                : "Tanpa batas terkonfigurasi"}
            </small>
            <Meter
              label="Penggunaan memori"
              value={resources.memory_bytes}
              limit={resources.memory_limit_bytes}
            />
          </div>
          <div className="project-metric">
            <span>Proses</span>
            <strong>{resources.pids_current.toLocaleString("id-ID")}</strong>
            <small>
              {resources.pids_limit > 0
                ? `dari ${resources.pids_limit} proses`
                : "Tanpa batas terkonfigurasi"}
            </small>
            <Meter
              label="Penggunaan proses"
              value={resources.pids_current}
              limit={resources.pids_limit}
            />
          </div>
        </div>
      </Panel>

      <div className="project-two-columns">
        <Panel
          title="Keamanan SSL"
          description="Sertifikat dan koneksi HTTPS situs."
          icon={<Lock size={18} />}
          action={<StatusPill variant={sslVariant} label={sslLabel} />}
        >
          <dl className="project-details">
            <Detail label="Penerbit">{ssl.issuer || "Belum terpasang"}</Detail>
            <Detail label="Berlaku hingga">
              {formatDate(ssl.valid_until)}
            </Detail>
            <Detail label="Sisa masa berlaku">
              {ssl.days_remaining !== null && ssl.days_remaining !== undefined
                ? ssl.days_remaining < 0
                  ? "Sudah kedaluwarsa"
                  : `${ssl.days_remaining} hari`
                : "Belum tersedia"}
            </Detail>
            <Detail label="Perpanjangan otomatis">
              {ssl.auto_renew ? "Aktif" : "Nonaktif"}
            </Detail>
            <Detail label="Alihkan ke HTTPS">
              {ssl.force_https ? "Aktif" : "Nonaktif"}
            </Detail>
          </dl>
          <footer className="project-panel-footer">
            {linkButton("ssl", "Kelola sertifikat")}
          </footer>
        </Panel>
        <Panel
          title="Runtime aplikasi"
          description="Lingkungan yang menjalankan project ini."
          icon={<Server size={18} />}
          action={
            <StatusPill
              variant={
                status.runtime_status === "running"
                  ? "success"
                  : status.runtime_status === "failed"
                    ? "danger"
                    : "neutral"
              }
              label={stateNames[status.runtime_status] || "Belum diketahui"}
            />
          }
        >
          <dl className="project-details">
            <Detail label="Jenis runtime">{runtimeName}</Detail>
            <Detail label="Versi">{runtime.version || "Tidak berlaku"}</Detail>
            <Detail label="Framework / preset">
              {runtime.framework || runtime.preset || "Tidak digunakan"}
            </Detail>
            <Detail
              label={
                runtime.kind === "proxy"
                  ? "Target upstream"
                  : "Entrypoint / direktori"
              }
            >
              <code>{runtime.entrypoint_or_target || site.docroot}</code>
            </Detail>
            {status.pid ? (
              <Detail label="ID proses">{status.pid}</Detail>
            ) : null}
          </dl>
          <footer className="project-panel-footer">
            {linkButton(runtimeTab, "Pengaturan runtime")}
          </footer>
        </Panel>
      </div>

      <Panel
        title="Penyimpanan"
        description="Rincian data yang digunakan oleh project ini."
        icon={<Folder size={18} />}
        action={linkButton("files", "Kelola berkas")}
      >
        <div className="project-storage-summary">
          <strong>{formatBytes(storage.total_site_bytes)}</strong>
          <span>
            {hasLimit
              ? `dari kuota tenant ${formatBytes(storage.plan_disk_limit_bytes)} (${Math.round(diskPercentage ?? 0)}%)`
              : "Kuota penyimpanan belum ditetapkan"}
          </span>
        </div>
        <div
          className="project-storage-bar"
          role="img"
          aria-label={`Penyimpanan situs ${formatBytes(storage.total_site_bytes)}${hasLimit ? ` dari kuota tenant ${formatBytes(storage.plan_disk_limit_bytes)}` : ""}`}
        >
          {segments.map((segment) => (
            <span
              key={segment.name}
              className={segment.className}
              style={{
                width: `${(Math.max(0, segment.bytes) / storageBase) * 100}%`,
              }}
            />
          ))}
        </div>
        <dl className="project-storage-legend">
          {segments.map((segment) => (
            <div key={segment.name}>
              <dt>
                <i className={segment.className} />
                {segment.name}
              </dt>
              <dd>{formatBytes(segment.bytes)}</dd>
            </div>
          ))}
          <div>
            <dt>Kuota tenant</dt>
            <dd>
              {hasLimit
                ? formatBytes(storage.plan_disk_limit_bytes)
                : "Belum ditetapkan"}
            </dd>
          </div>
        </dl>
      </Panel>

      <div className="project-two-columns">
        <Panel
          title="Deployment terakhir"
          description="Riwayat rilis project dari repositori Git."
          icon={<GitBranch size={18} />}
          action={
            deploy ? (
              <StatusPill
                variant={historyVariant(deploy.status)}
                label={historyNames[deploy.status] || deploy.status}
              />
            ) : undefined
          }
        >
          {deploy ? (
            <dl className="project-details">
              <Detail label="Rilis">
                <code>{deploy.release_id}</code>
              </Detail>
              <Detail label="Commit">
                <code>{deploy.commit_hash.slice(0, 7) || "HEAD"}</code> ·{" "}
                {deploy.branch}
              </Detail>
              <Detail label="Pesan commit">
                {deploy.commit_message || "Tidak ada pesan"}
              </Detail>
              <Detail label="Waktu">{formatDate(deploy.created_at)}</Detail>
              {deploy.duration_ms !== null &&
                deploy.duration_ms !== undefined && (
                  <Detail label="Durasi">
                    {(deploy.duration_ms / 1000).toLocaleString("id-ID")} detik
                  </Detail>
                )}
              {deploy.can_rollback && (
                <Detail label="Pemulihan">Rollback tersedia</Detail>
              )}
            </dl>
          ) : (
            <div className="project-empty">
              <GitBranch size={24} />
              <h4>Belum ada deployment</h4>
              <p>Hubungkan repositori untuk mengelola rilis project.</p>
            </div>
          )}
          <footer className="project-panel-footer">
            {linkButton("deploy", "Kelola deployment")}
          </footer>
        </Panel>
        <Panel
          title="Cadangan terakhir"
          description="Salinan berkas dan database project."
          icon={<Archive size={18} />}
          action={
            backup ? (
              <StatusPill
                variant={historyVariant(backup.status)}
                label={historyNames[backup.status] || backup.status}
              />
            ) : undefined
          }
        >
          {backup ? (
            <dl className="project-details">
              <Detail label="Nama arsip">
                <code>{backup.filename}</code>
              </Detail>
              <Detail label="Ukuran">{formatBytes(backup.size_bytes)}</Detail>
              <Detail label="Cakupan">
                {[
                  backup.includes_files ? "Berkas" : "",
                  backup.includes_databases ? "Database" : "",
                ]
                  .filter(Boolean)
                  .join(" dan ") || "Belum tersedia"}
              </Detail>
              <Detail label="Dibuat">{formatDate(backup.created_at)}</Detail>
            </dl>
          ) : (
            <div className="project-empty">
              <Archive size={24} />
              <h4>Belum ada cadangan</h4>
              <p>Buat cadangan sebelum mengubah konfigurasi project.</p>
            </div>
          )}
          <footer className="project-panel-footer">
            {linkButton("backups", "Kelola cadangan")}
          </footer>
        </Panel>
      </div>
    </>
  );
}
