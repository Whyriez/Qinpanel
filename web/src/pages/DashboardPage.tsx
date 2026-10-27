import { useCallback, useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  RefreshCw,
  Plus,
  Cpu,
  HardDrive,
  Network,
  MemoryStick,
  Globe,
  ExternalLink,
  Activity,
  Server,
} from "lucide-react";
import {
  fetchSystemInfo,
  fetchSites,
  fetchSystemAnalyticsTimeseries,
  fetchSystemMetrics,
  HostMetricsResponse,
} from "../api/client";
import {
  Site,
  SystemInfoResponse,
  SystemTimeseriesResponse,
} from "../types/api";
import { useAuth } from "../context/AuthContext";
import { StatusPill } from "../components/ui/StatusPill";
import { PulseStrip } from "../components/ui/PulseStrip";
import { Skeleton } from "../components/ui/Skeleton";
import { SiteWizardDialog } from "../components/sites/SiteWizardDialog";

interface DashboardPageProps {
  onNavigate?: (page: string) => void;
  onSelectSite?: (site: Site) => void;
}

export function DashboardPage({
  onNavigate = () => {},
  onSelectSite = () => {},
}: DashboardPageProps) {
  const { t, i18n } = useTranslation();
  const { user } = useAuth();
  const admin = user?.role === "admin";
  const [info, setInfo] = useState<SystemInfoResponse | null>(null);
  const [sites, setSites] = useState<Site[]>([]);
  const [metrics, setMetrics] = useState<SystemTimeseriesResponse | null>(null);
  const [hostMetrics, setHostMetrics] = useState<HostMetricsResponse | null>(null);
  const [metricsError, setMetricsError] = useState<string | null>(null);
  const loaded = useRef(false);
  const [loading, setLoading] = useState(true);
  const [infoError, setInfoError] = useState(false);
  const [sitesError, setSitesError] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [updated, setUpdated] = useState<Date | null>(null);
  const loadData = useCallback(async () => {
    setLoading(!loaded.current);
    const [system, website, host, sample] = await Promise.allSettled([
      fetchSystemInfo(),
      fetchSites(),
      admin
        ? fetchSystemAnalyticsTimeseries({ interval: "1m" })
        : Promise.resolve(null),
      admin ? fetchSystemMetrics() : Promise.resolve(null),
    ]);
    setInfo(system.status === "fulfilled" ? system.value : null);
    setInfoError(system.status === "rejected");
    setSites(website.status === "fulfilled" ? website.value : []);
    setSitesError(website.status === "rejected");
    setMetrics(host.status === "fulfilled" ? host.value : null);
    setHostMetrics(sample.status === 'fulfilled' ? sample.value : null);
    setMetricsError(sample.status === 'rejected' ? sample.reason?.message || 'Gagal memuat metrik server.' : null);
    loaded.current = true;
    setUpdated(new Date());
    setLoading(false);
  }, [admin]);
  useEffect(() => {
    void loadData();
    const refresh = window.setInterval(() => { void loadData(); }, 10_000);
    return () => window.clearInterval(refresh);
  }, [loadData]);

  const points = metrics?.points || [];
  const latest = hostMetrics?.sample;
  const number = (value: number, digits = 1) =>
    new Intl.NumberFormat(i18n.language, {
      maximumFractionDigits: digits,
    }).format(value);
  const bytes = (value: number) =>
    value >= 1024 ** 3
      ? `${number(value / 1024 ** 3)} GB`
      : `${number(value / 1024 ** 2)} MB`;
  const active = sites.filter((site) => site.status === "active").length;
  const stats = [
    {
      label: t("dashboard.quickStats.cpu"),
      icon: Cpu,
      value: latest ? `${number(latest.cpu_percent)}%` : "—",
      caption: latest ? `${latest.cpu_cores} core` : t("design.averageUsage"),
      data: points.map((point) => point.cpu_avg),
    },
    {
      label: t("dashboard.quickStats.memory"),
      icon: MemoryStick,
      value: latest ? `${bytes(latest.memory_used_bytes)} / ${bytes(latest.memory_total_bytes)}` : "—",
      caption: t("design.memoryUsed"),
      data: points.map((point) => point.memory_used_avg_bytes),
    },
    {
      label: t("dashboard.quickStats.disk"),
      icon: HardDrive,
      value: latest ? `${bytes(latest.disk_used_bytes)} / ${bytes(latest.disk_total_bytes)}` : "—",
      caption: t("design.storageUsed"),
      data: points.map((point) => point.disk_used_avg_bytes),
    },
    {
      label: t("dashboard.quickStats.network"),
      icon: Network,
      value: latest
        ? `${number(latest.net_rx_bytes_sec / 1024)} KB/s ↓ / ${number(latest.net_tx_bytes_sec / 1024)} KB/s ↑`
        : "—",
      caption: t("design.inboundOutbound"),
      data: points.map(
        (point) => point.net_rx_avg_bytes_sec + point.net_tx_avg_bytes_sec,
      ),
    },
  ];

  return (
    <div className="hosting-dashboard">
      {admin && !loading && (metricsError || hostMetrics?.state === 'failed' || hostMetrics?.state === 'stale') && <div role="alert" className="page-notice is-error">{metricsError || hostMetrics?.error || 'Data server sudah kedaluwarsa. Periksa collector metrik.'}</div>}
      <div className="page-heading">
        <div>
          <h1>{t("nav.dashboard")}</h1>
          <p>{t("design.dashboardDescription")}</p>
        </div>
        <div className="page-actions">
          <button
            className="button-secondary"
            onClick={loadData}
            disabled={loading}
          >
            <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
            {t("common.refresh")}
          </button>
          <button
            className="button-primary"
            onClick={() => setWizardOpen(true)}
          >
            <Plus size={17} />
            {t("sites.newSite")}
          </button>
        </div>
      </div>
      <div className="hosting-summary">
        <div className="hosting-summary-title">
          <span className="summary-icon">
            <Globe size={20} />
          </span>
          <div>
            <h2>{t("design.yourHosting")}</h2>
            <p>
              {loading
                ? t("design.loadingHosting")
                : sitesError
                  ? t("design.sitesUnavailable")
                  : t("design.siteSummary", { count: sites.length, active })}
            </p>
          </div>
        </div>
        <StatusPill
          variant={
            loading || infoError
              ? "neutral"
              : info?.agent_connected
                ? "success"
                : "warning"
          }
          label={
            loading
              ? t("design.checkingStatus")
              : infoError
                ? t("design.statusUnavailable")
                : info?.agent_connected
                  ? t("design.managementConnected")
                  : t("design.managementDisconnected")
          }
        />
      </div>
      {admin && (
        <section
          className="resource-grid"
          aria-label={t("design.serverResources")}
        >
          {stats.map(({ label, icon: Icon, value, caption, data }) => (
            <article className="resource-stat" key={label}>
              <div className="resource-label">
                <span>{label}</span>
                <Icon size={16} />
              </div>
              {loading ? (
                <Skeleton className="h-9 w-24 my-2" />
              ) : (
                <strong className="resource-value">{value}</strong>
              )}
              <div className="resource-bottom">
                <span>{latest ? caption : t("design.waitingMetrics")}</span>
                {!loading && data.length > 1 && (
                  <PulseStrip data={data} width={72} height={22} />
                )}
              </div>
            </article>
          ))}
        </section>
      )}
      <div className={`dashboard-detail-grid ${!admin ? "is-tenant" : ""}`}>
        <section
          className="hosting-panel sites-overview"
          aria-labelledby="dashboard-sites-title"
        >
          <header className="section-heading">
            <div>
              <h2 id="dashboard-sites-title">{t("nav.sites")}</h2>
              <p>{t("design.manageWebsites")}</p>
            </div>
            <button className="text-button" onClick={() => onNavigate("sites")}>
              {t("design.viewAll")}
            </button>
          </header>
          {loading ? (
            <div className="dashboard-loading" role="status">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
              <span className="sr-only">{t("design.loadingHosting")}</span>
            </div>
          ) : sitesError ? (
            <div className="dashboard-empty" role="alert">
              <Globe size={27} />
              <h3>{t("design.sitesUnavailable")}</h3>
              <p>{t("design.retryDescription")}</p>
              <button className="button-secondary" onClick={loadData}>
                {t("common.retry")}
              </button>
            </div>
          ) : sites.length === 0 ? (
            <div className="dashboard-empty">
              <Globe size={30} strokeWidth={1.3} />
              <h3>{t("design.noSites")}</h3>
              <p>{t("design.noSitesDescription")}</p>
              <button
                className="button-primary"
                onClick={() => setWizardOpen(true)}
              >
                <Plus size={16} />
                {t("sites.newSite")}
              </button>
            </div>
          ) : (
            <div className="hosting-table-scroll">
              <table className="hosting-table">
                <thead>
                  <tr>
                    <th>{t("dashboard.sitesTable.domain")}</th>
                    <th>{t("sites.table.type")}</th>
                    <th>{t("sites.table.status")}</th>
                    <th>
                      <span className="sr-only">{t("common.actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {sites.slice(0, 7).map((site) => (
                    <tr key={site.id}>
                      <td>
                        <button
                          className="domain-link"
                          onClick={() => onSelectSite(site)}
                        >
                          {site.primary_domain}
                        </button>
                        <span className="domain-detail">
                          {site.force_https ? "HTTPS" : "HTTP"}
                        </span>
                      </td>
                      <td>
                        <span className="runtime-label">
                          {{
                            static: "Static",
                            php: "PHP",
                            nodejs: "Node.js",
                            python: "Python",
                            proxy: "Reverse proxy",
                          }[site.kind] || site.kind}
                          {site.version ? ` ${site.version}` : ""}
                        </span>
                      </td>
                      <td>
                        <StatusPill
                          size="sm"
                          variant={
                            site.status === "active"
                              ? "success"
                              : site.status === "suspended"
                                ? "warning"
                                : "neutral"
                          }
                          label={
                            site.status === "active"
                              ? t("design.active")
                              : site.status === "suspended"
                                ? t("design.suspended")
                                : t("design.inactive")
                          }
                        />
                      </td>
                      <td>
                        <a
                          className="icon-button"
                          href={`${site.force_https ? "https" : "http"}://${site.primary_domain}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          aria-label={t("design.visitSite", {
                            domain: site.primary_domain,
                          })}
                        >
                          <ExternalLink size={15} />
                        </a>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {!loading && !sitesError && sites.length > 0 && (
            <footer className="table-footer">
              {t("design.showingSites", {
                shown: Math.min(sites.length, 7),
                total: sites.length,
              })}
            </footer>
          )}
        </section>
        {admin && (
          <section className="hosting-panel server-overview">
            <header className="section-heading">
              <div>
                <h2>{t("design.serverActivity")}</h2>
                <p>{t("design.recentCpu")}</p>
              </div>
              <Activity size={17} className="muted-icon" />
            </header>
            <div className="cpu-chart">
              {loading ? (
                <Skeleton className="h-32" />
              ) : points.length > 1 ? (
                <>
                  <div className="chart-current">
                    <strong>
                      {number(latest!.cpu_percent)}
                      <span>%</span>
                    </strong>
                    <span>{t("design.cpuAverage")}</span>
                  </div>
                  <PulseStrip
                  responsive
                  label={t('design.recentCpu')}
                    data={points.map((point) => point.cpu_avg)}
                    width={400}
                    height={112}
                  />
                  <div className="chart-time">
                    <span>
                      {new Date(points[0].timestamp * 1000).toLocaleTimeString(
                        i18n.language,
                        { hour: "2-digit", minute: "2-digit" },
                      )}
                    </span>
                    <span>
                      {new Date(
                        points[points.length - 1].timestamp * 1000,
                      ).toLocaleTimeString(i18n.language, {
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </span>
                  </div>
                </>
              ) : (
                <div className="metrics-empty">
                  <Activity size={28} strokeWidth={1.4} />
                  <p>{t("design.noMetrics")}</p>
                </div>
              )}
            </div>
            <div className="server-information">
              <Server size={16} />
              <span>{t("design.serverEnvironment")}</span>
              <strong>{info ? `${info.os} · ${info.arch}` : "—"}</strong>
            </div>
            <button
              className="panel-footer-action"
              onClick={() => onNavigate("analytics")}
            >
              {t("design.openAnalytics")}
            </button>
          </section>
        )}
      </div>
      <footer className="dashboard-footer">
        <span>{t("design.workspaceOverview")}</span>
        <span>
          {updated
            ? t("design.updatedAt", {
                time: updated.toLocaleTimeString(i18n.language, {
                  hour: "2-digit",
                  minute: "2-digit",
                }),
              })
            : t("design.loadingHosting")}
        </span>
      </footer>
      <SiteWizardDialog
        isOpen={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onSuccess={() => {
          setWizardOpen(false);
          void loadData();
        }}
      />
    </div>
  );
}
