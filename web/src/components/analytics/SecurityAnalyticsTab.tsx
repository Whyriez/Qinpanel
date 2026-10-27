import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import {
  ShieldAlert,
  ShieldCheck,
  Ban,
  UserX,
  Lock,
  Clock,
  AlertTriangle,
  RefreshCw,
  Search,
  Globe,
  Trash2,
  CheckCircle2,
  X,
} from 'lucide-react';
import {
  fetchSiteSecurityOverview,
  fetchSiteBannedIps,
  banSiteIp,
  unbanSiteIp,
} from '../../api/client';
import {
  SecurityOverviewResponse,
  BannedIpDto,
} from '../../types/api';

export interface SecurityAnalyticsTabProps {
  siteId: number;
  siteDomain: string;
  timeRange?: { from: number; to: number };
}

type TtlPreset = '1h' | '6h' | '24h' | '7d' | 'permanent';

const TTL_OPTIONS: { label: string; value: TtlPreset; secs: number | null }[] = [
  { label: '1 Jam', value: '1h', secs: 3600 },
  { label: '6 Jam', value: '6h', secs: 21600 },
  { label: '24 Jam', value: '24h', secs: 86400 },
  { label: '7 Hari', value: '7d', secs: 604800 },
  { label: 'Permanen', value: 'permanent', secs: null },
];

function formatTtlCountdown(remainingSecs?: number | null): string {
  if (remainingSecs === null || remainingSecs === undefined) {
    return 'Permanen';
  }
  if (remainingSecs <= 0) {
    return 'Kedaluwarsa';
  }

  const hours = Math.floor(remainingSecs / 3600);
  const minutes = Math.floor((remainingSecs % 3600) / 60);

  if (hours > 24) {
    const days = Math.floor(hours / 24);
    return `${days} hari ${hours % 24} jam`;
  }
  if (hours > 0) {
    return `${hours} jam ${minutes} mnt`;
  }
  return `${minutes} menit`;
}

function formatRelativeTime(ts: number): string {
  const diffSecs = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (diffSecs < 60) return `${diffSecs} dtk lalu`;
  const mins = Math.floor(diffSecs / 60);
  if (mins < 60) return `${mins} mnt lalu`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} jam lalu`;
  const days = Math.floor(hours / 24);
  return `${days} hari lalu`;
}

function getCategoryBadge(category: string): { label: string; colorClass: string } {
  switch (category) {
    case 'env_credentials':
      return { label: 'Kredensial .env', colorClass: 'bg-rose-500/15 text-rose-500 border-rose-500/20' };
    case 'vcs_leak':
      return { label: 'Kebocoran Git / VCS', colorClass: 'bg-amber-500/15 text-amber-500 border-amber-500/20' };
    case 'wordpress_probing':
      return { label: 'Eksploitasi WordPress', colorClass: 'bg-indigo-500/15 text-indigo-500 border-indigo-500/20' };
    case 'db_admin':
      return { label: 'Manajemen DB', colorClass: 'bg-purple-500/15 text-purple-500 border-purple-500/20' };
    case 'debug_dump':
      return { label: 'Dump Berkas / SQL', colorClass: 'bg-orange-500/15 text-orange-500 border-orange-500/20' };
    case 'admin_probing':
      return { label: 'Panel Administratif', colorClass: 'bg-blue-500/15 text-blue-500 border-blue-500/20' };
    default:
      return { label: 'Pemindaian Jalur', colorClass: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/20' };
  }
}

function getRiskBadge(level: string): { label: string; badgeClass: string } {
  switch (level.toLowerCase()) {
    case 'critical':
      return { label: 'Kritis', badgeClass: 'bg-red-500/20 text-red-500 border-red-500/30' };
    case 'high_risk':
      return { label: 'Risiko Tinggi', badgeClass: 'bg-orange-500/20 text-orange-500 border-orange-500/30' };
    case 'suspicious':
      return { label: 'Mencurigakan', badgeClass: 'bg-amber-500/20 text-amber-500 border-amber-500/30' };
    case 'low':
    default:
      return { label: 'Rendah', badgeClass: 'bg-emerald-500/20 text-emerald-500 border-emerald-500/30' };
  }
}

export const SecurityAnalyticsTab: React.FC<SecurityAnalyticsTabProps> = ({
  siteId,
  siteDomain: _siteDomain,
  timeRange,
}) => {
  const [overview, setOverview] = useState<SecurityOverviewResponse | null>(null);
  const [bannedIps, setBannedIps] = useState<BannedIpDto[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [refreshing, setRefreshing] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Ban Modal Dialog State
  const [isBanModalOpen, setIsBanModalOpen] = useState<boolean>(false);
  const [banInputIp, setBanInputIp] = useState<string>('');
  const [banSelectedTtl, setBanSelectedTtl] = useState<TtlPreset>('24h');
  const [banInputReason, setBanInputReason] = useState<string>('');
  const [submittingBan, setSubmittingBan] = useState<boolean>(false);
  const [banFormError, setBanFormError] = useState<string | null>(null);

  // Unban Loading State
  const [unbanningIp, setUnbanningIp] = useState<string | null>(null);

  const loadData = useCallback(async (isSilent = false) => {
    if (!isSilent) setLoading(true);
    else setRefreshing(true);
    setError(null);

    try {
      const [overviewData, bannedData] = await Promise.all([
        fetchSiteSecurityOverview(siteId, {
          from: timeRange?.from,
          to: timeRange?.to,
          limit: 20,
        }),
        fetchSiteBannedIps(siteId),
      ]);

      setOverview(overviewData);
      setBannedIps(bannedData);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat analitik keamanan';
      setError(msg);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [siteId, timeRange]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Open modal prefilled
  const handleOpenBanModal = (prefillIp = '', prefillReason = '') => {
    setBanInputIp(prefillIp);
    setBanInputReason(prefillReason || 'Aktivitas mencurigakan terdeteksi');
    setBanSelectedTtl('24h');
    setBanFormError(null);
    setIsBanModalOpen(true);
  };

  // Submit Ban IP
  const handleBanSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanIp = banInputIp.trim();
    if (!cleanIp) {
      setBanFormError('Alamat IP atau CIDR harus diisi');
      return;
    }

    setSubmittingBan(true);
    setBanFormError(null);

    try {
      const opt = TTL_OPTIONS.find((o) => o.value === banSelectedTtl);
      const ttlSecs = opt ? opt.secs : null;

      await banSiteIp(siteId, {
        ip: cleanIp,
        ttl_secs: ttlSecs,
        reason: banInputReason.trim() || undefined,
      });

      setIsBanModalOpen(false);
      setSuccessMessage(`Alamat IP ${cleanIp} berhasil diblokir di level Nginx`);
      setTimeout(() => setSuccessMessage(null), 4000);
      loadData(true);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memblokir alamat IP';
      setBanFormError(msg);
    } finally {
      setSubmittingBan(false);
    }
  };

  // Unban IP
  const handleUnban = async (ip: string) => {
    if (!window.confirm(`Yakin ingin membuka blokir alamat IP ${ip}?`)) {
      return;
    }

    setUnbanningIp(ip);
    try {
      await unbanSiteIp(siteId, ip);
      setSuccessMessage(`Alamat IP ${ip} berhasil dibuka dari daftar blokir`);
      setTimeout(() => setSuccessMessage(null), 4000);
      loadData(true);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal membuka blokir alamat IP';
      setError(msg);
    } finally {
      setUnbanningIp(null);
    }
  };

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center p-12 text-center text-[var(--ink-muted)]">
        <RefreshCw className="w-8 h-8 animate-spin mb-3 text-[var(--primary)]" />
        <p className="text-sm">Memuat data analitik keamanan & status pemblokiran...</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header & Quick Action */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl">
        <div>
          <h3 className="text-base font-semibold text-[var(--ink)] flex items-center gap-2">
            <ShieldAlert className="w-5 h-5 text-rose-500" />
            Analitik Keamanan & Proteksi Akses IP
          </h3>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Mendeteksi pemindaian berkas sensitif, serangan brute force login, dan menerapkan pemblokiran IP atomik via Nginx dengan TTL.
          </p>
        </div>

        <div className="flex items-center gap-2 self-start sm:self-auto">
          <button
            type="button"
            onClick={() => loadData(true)}
            disabled={refreshing}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium border border-[var(--line)] rounded-lg text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
            Perbarui
          </button>
          <button
            type="button"
            onClick={() => handleOpenBanModal()}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-lg bg-rose-600 hover:bg-rose-700 text-white shadow-xs transition-colors"
          >
            <Ban className="w-3.5 h-3.5" />
            Blokir Alamat IP
          </button>
        </div>
      </div>

      {/* Notifications / Alerts */}
      {error && (
        <div className="p-3.5 bg-rose-500/10 border border-rose-500/20 text-rose-500 rounded-lg text-xs flex items-center justify-between">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-4 h-4 shrink-0" />
            <span>{error}</span>
          </div>
          <button
            type="button"
            onClick={() => setError(null)}
            className="p-1 hover:bg-rose-500/20 rounded"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {successMessage && (
        <div className="p-3.5 bg-emerald-500/10 border border-emerald-500/20 text-emerald-500 rounded-lg text-xs flex items-center justify-between">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-4 h-4 shrink-0" />
            <span>{successMessage}</span>
          </div>
          <button
            type="button"
            onClick={() => setSuccessMessage(null)}
            className="p-1 hover:bg-emerald-500/20 rounded"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* Summary KPI Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Probing Requests */}
        <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span>Pemindaian Jalur (Probing)</span>
            <ShieldAlert className="w-4 h-4 text-orange-500" />
          </div>
          <div className="text-2xl font-bold font-mono text-[var(--ink)]">
            {(overview?.total_probing_requests ?? 0).toLocaleString()}
          </div>
          <p className="text-[11px] text-[var(--ink-muted)] mt-1">
            Akses ke .env, .git, backup.sql, dll.
          </p>
        </div>

        {/* Brute Force Attempts */}
        <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span>Percobaan Brute Force</span>
            <Lock className="w-4 h-4 text-rose-500" />
          </div>
          <div className="text-2xl font-bold font-mono text-[var(--ink)]">
            {(overview?.total_brute_force_attempts ?? 0).toLocaleString()}
          </div>
          <p className="text-[11px] text-[var(--ink-muted)] mt-1">
            Kegagalan login berulang (401/403/429)
          </p>
        </div>

        {/* Unique Suspicious IPs */}
        <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span>IP Mencurigakan Unik</span>
            <UserX className="w-4 h-4 text-purple-500" />
          </div>
          <div className="text-2xl font-bold font-mono text-[var(--ink)]">
            {(overview?.unique_suspicious_ips ?? 0).toLocaleString()}
          </div>
          <p className="text-[11px] text-[var(--ink-muted)] mt-1">
            Klien dengan skor ancaman terdeteksi
          </p>
        </div>

        {/* Active Banned IPs */}
        <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span>IP Terblokir Aktif</span>
            <Ban className="w-4 h-4 text-emerald-500" />
          </div>
          <div className="text-2xl font-bold font-mono text-[var(--ink)]">
            {bannedIps.length.toLocaleString()}
          </div>
          <p className="text-[11px] text-[var(--ink-muted)] mt-1">
            Aturan deny aktif pada Nginx vhost
          </p>
        </div>
      </div>

      {/* Active Banned IPs Management Section */}
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-xl overflow-hidden shadow-xs">
        <div className="p-4 border-b border-[var(--line)] flex items-center justify-between">
          <div>
            <h4 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
              <ShieldCheck className="w-4 h-4 text-emerald-500" />
              Daftar IP Terblokir Aktif (Nginx Deny Rules)
            </h4>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              Alamat IP berikut ditolak secara instan oleh web server Nginx. Pemblokiran sementara akan kedaluwarsa otomatis sesuai TTL.
            </p>
          </div>
          <span className="text-xs font-mono font-medium px-2 py-0.5 rounded-full bg-[var(--surface-hover)] text-[var(--ink-muted)]">
            {bannedIps.length} aturan aktif
          </span>
        </div>

        {bannedIps.length === 0 ? (
          <div className="p-8 text-center text-[var(--ink-muted)]">
            <CheckCircle2 className="w-8 h-8 mx-auto mb-2 text-emerald-500/70" />
            <p className="text-xs">Tidak ada alamat IP yang sedang diblokir pada situs ini.</p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead className="bg-[var(--surface-hover)] border-b border-[var(--line)] text-[var(--ink-muted)]">
                <tr>
                  <th className="py-2.5 px-4 font-medium">Alamat IP / CIDR</th>
                  <th className="py-2.5 px-4 font-medium">Alasan Pemblokiran</th>
                  <th className="py-2.5 px-4 font-medium">Waktu Blokir</th>
                  <th className="py-2.5 px-4 font-medium">Sisa TTL</th>
                  <th className="py-2.5 px-4 font-medium">Diblokir Oleh</th>
                  <th className="py-2.5 px-4 font-medium text-right">Aksi</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                {bannedIps.map((b) => (
                  <tr key={b.id} className="hover:bg-[var(--surface-hover)]/50 transition-colors">
                    <td className="py-2.5 px-4 font-mono font-semibold text-rose-500">
                      {b.ip_address}
                    </td>
                    <td className="py-2.5 px-4 text-[var(--ink-muted)] max-w-xs truncate">
                      {b.reason}
                    </td>
                    <td className="py-2.5 px-4 text-[var(--ink-muted)]">
                      {formatRelativeTime(b.banned_at)}
                    </td>
                    <td className="py-2.5 px-4">
                      <span className="inline-flex items-center gap-1 font-mono text-xs px-2 py-0.5 rounded-md bg-[var(--surface-hover)] text-[var(--ink)]">
                        <Clock className="w-3 h-3 text-[var(--ink-muted)]" />
                        {formatTtlCountdown(b.remaining_ttl_secs)}
                      </span>
                    </td>
                    <td className="py-2.5 px-4 text-[var(--ink-muted)]">
                      {b.banned_by}
                    </td>
                    <td className="py-2.5 px-4 text-right">
                      <button
                        type="button"
                        onClick={() => handleUnban(b.ip_address)}
                        disabled={unbanningIp === b.ip_address}
                        className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded-md border border-[var(--line)] hover:bg-rose-500/10 hover:border-rose-500/30 text-rose-500 transition-colors disabled:opacity-50"
                        title="Buka blokir IP ini"
                      >
                        <Trash2 className="w-3 h-3" />
                        {unbanningIp === b.ip_address ? 'Membuka...' : 'Buka Blokir'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Top Client IPs with Threat Scoring */}
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-xl overflow-hidden shadow-xs">
        <div className="p-4 border-b border-[var(--line)] flex items-center justify-between">
          <div>
            <h4 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
              <UserX className="w-4 h-4 text-orange-500" />
              IP Pengakses Teratas & Tingkat Risiko
            </h4>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              Evaluasi tingkat risiko klien berdasarkan frekuensi permintaan, rasio error 4xx/5xx, serta aktivitas pemindaian.
            </p>
          </div>
        </div>

        {(!overview || overview.top_ips.length === 0) ? (
          <div className="p-8 text-center text-[var(--ink-muted)]">
            <CheckCircle2 className="w-8 h-8 mx-auto mb-2 text-emerald-500/70" />
            <p className="text-xs">Belum ada aktivitas mencurigakan yang tercatat dalam rentang waktu ini.</p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead className="bg-[var(--surface-hover)] border-b border-[var(--line)] text-[var(--ink-muted)]">
                <tr>
                  <th className="py-2.5 px-4 font-medium">Alamat IP</th>
                  <th className="py-2.5 px-4 font-medium">Asal Negara</th>
                  <th className="py-2.5 px-4 font-medium text-right">Permintaan</th>
                  <th className="py-2.5 px-4 font-medium text-right">Error 4xx/5xx</th>
                  <th className="py-2.5 px-4 font-medium text-right">Probing</th>
                  <th className="py-2.5 px-4 font-medium text-right">Auth Fail</th>
                  <th className="py-2.5 px-4 font-medium">Tingkat Risiko</th>
                  <th className="py-2.5 px-4 font-medium">Terakhir Dilihat</th>
                  <th className="py-2.5 px-4 font-medium text-right">Aksi</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                {overview.top_ips.map((ip) => {
                  const risk = getRiskBadge(ip.risk_level);
                  const isAlreadyBanned = bannedIps.some((b) => b.ip_address === ip.ip);

                  return (
                    <tr key={ip.ip} className="hover:bg-[var(--surface-hover)]/50 transition-colors">
                      <td className="py-2.5 px-4 font-mono font-semibold">
                        {ip.ip}
                      </td>
                      <td className="py-2.5 px-4 text-[var(--ink-muted)]">
                        <span className="inline-flex items-center gap-1.5">
                          <Globe className="w-3 h-3 text-[var(--ink-muted)]" />
                          {ip.country || 'Global/LAN'}
                        </span>
                      </td>
                      <td className="py-2.5 px-4 text-right font-mono">
                        {ip.total_requests.toLocaleString()}
                      </td>
                      <td className="py-2.5 px-4 text-right font-mono">
                        <span className={ip.error_4xx + ip.error_5xx > 0 ? 'text-amber-500 font-semibold' : ''}>
                          {ip.error_4xx + ip.error_5xx}
                        </span>
                      </td>
                      <td className="py-2.5 px-4 text-right font-mono">
                        <span className={ip.probing_count > 0 ? 'text-rose-500 font-semibold' : ''}>
                          {ip.probing_count}
                        </span>
                      </td>
                      <td className="py-2.5 px-4 text-right font-mono">
                        <span className={ip.auth_failures > 0 ? 'text-purple-500 font-semibold' : ''}>
                          {ip.auth_failures}
                        </span>
                      </td>
                      <td className="py-2.5 px-4">
                        <span className={`px-2 py-0.5 rounded-full text-[10px] font-semibold border ${risk.badgeClass}`}>
                          {risk.label}
                        </span>
                      </td>
                      <td className="py-2.5 px-4 text-[var(--ink-muted)]">
                        {formatRelativeTime(ip.last_seen)}
                      </td>
                      <td className="py-2.5 px-4 text-right">
                        {isAlreadyBanned ? (
                          <span className="text-[11px] font-medium text-emerald-500 inline-flex items-center gap-1">
                            <CheckCircle2 className="w-3 h-3" />
                            Terblokir
                          </span>
                        ) : (
                          <button
                            type="button"
                            onClick={() => handleOpenBanModal(ip.ip, `Terdeteksi aktivitas berisiko tinggi (${ip.probing_count} probe, ${ip.auth_failures} auth fail)`)}
                            className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-semibold rounded-md bg-rose-500/15 hover:bg-rose-500 text-rose-500 hover:text-white transition-colors"
                          >
                            <Ban className="w-3 h-3" />
                            Blokir
                          </button>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Probing Paths & Brute Force Incidents Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Probing Paths Table */}
        <div className="bg-[var(--surface)] border border-[var(--line)] rounded-xl overflow-hidden shadow-xs flex flex-col">
          <div className="p-4 border-b border-[var(--line)]">
            <h4 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
              <Search className="w-4 h-4 text-indigo-500" />
              Jalur Sensitif yang Dipindai (Probing Paths)
            </h4>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              Pemindaian jalur file konfigurasi, kredensial .env, dan exploit scanner.
            </p>
          </div>

          {(!overview || overview.probing_paths.length === 0) ? (
            <div className="p-8 text-center text-[var(--ink-muted)] my-auto">
              <CheckCircle2 className="w-7 h-7 mx-auto mb-2 text-emerald-500/70" />
              <p className="text-xs">Tidak ada pemindaian jalur sensitif terdeteksi.</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs">
                <thead className="bg-[var(--surface-hover)] border-b border-[var(--line)] text-[var(--ink-muted)]">
                  <tr>
                    <th className="py-2.5 px-4 font-medium">Jalur (URI)</th>
                    <th className="py-2.5 px-4 font-medium">Kategori</th>
                    <th className="py-2.5 px-4 font-medium text-right">Hits</th>
                    <th className="py-2.5 px-4 font-medium text-right">IP Unik</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                  {overview.probing_paths.map((p) => {
                    const cat = getCategoryBadge(p.category);
                    return (
                      <tr key={p.path} className="hover:bg-[var(--surface-hover)]/50 transition-colors">
                        <td className="py-2.5 px-4 font-mono font-medium text-[var(--ink)] max-w-xs truncate">
                          {p.path}
                        </td>
                        <td className="py-2.5 px-4">
                          <span className={`px-2 py-0.5 rounded-md text-[10px] font-medium border ${cat.colorClass}`}>
                            {cat.label}
                          </span>
                        </td>
                        <td className="py-2.5 px-4 text-right font-mono font-semibold text-rose-500">
                          {p.count.toLocaleString()}
                        </td>
                        <td className="py-2.5 px-4 text-right font-mono text-[var(--ink-muted)]">
                          {p.unique_ips}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </div>

        {/* Brute Force Incidents Table */}
        <div className="bg-[var(--surface)] border border-[var(--line)] rounded-xl overflow-hidden shadow-xs flex flex-col">
          <div className="p-4 border-b border-[var(--line)]">
            <h4 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
              <Lock className="w-4 h-4 text-rose-500" />
              Indikasi Percobaan Brute Force
            </h4>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              Percobaan login gagal beruntun pada endpoint autentikasi (/login, /wp-login, dll).
            </p>
          </div>

          {(!overview || overview.brute_force_incidents.length === 0) ? (
            <div className="p-8 text-center text-[var(--ink-muted)] my-auto">
              <CheckCircle2 className="w-7 h-7 mx-auto mb-2 text-emerald-500/70" />
              <p className="text-xs">Tidak ada insiden brute force teridentifikasi.</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs">
                <thead className="bg-[var(--surface-hover)] border-b border-[var(--line)] text-[var(--ink-muted)]">
                  <tr>
                    <th className="py-2.5 px-4 font-medium">IP Penyerang</th>
                    <th className="py-2.5 px-4 font-medium">Target Path</th>
                    <th className="py-2.5 px-4 font-medium text-right">Kegagalan</th>
                    <th className="py-2.5 px-4 font-medium text-right">Aksi</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                  {overview.brute_force_incidents.map((bf) => {
                    const isAlreadyBanned = bannedIps.some((b) => b.ip_address === bf.ip);
                    return (
                      <tr key={`${bf.ip}-${bf.target_path}`} className="hover:bg-[var(--surface-hover)]/50 transition-colors">
                        <td className="py-2.5 px-4 font-mono font-semibold text-[var(--ink)]">
                          {bf.ip}
                          {bf.country && (
                            <span className="ml-1.5 text-[10px] text-[var(--ink-muted)]">
                              ({bf.country})
                            </span>
                          )}
                        </td>
                        <td className="py-2.5 px-4 font-mono text-[var(--ink-muted)]">
                          {bf.target_path}
                        </td>
                        <td className="py-2.5 px-4 text-right font-mono font-semibold text-rose-500">
                          {bf.failed_attempts}x
                        </td>
                        <td className="py-2.5 px-4 text-right">
                          {isAlreadyBanned ? (
                            <span className="text-[10px] font-medium text-emerald-500">Terblokir</span>
                          ) : (
                            <button
                              type="button"
                              onClick={() => handleOpenBanModal(bf.ip, `Serangan brute force login pada ${bf.target_path} (${bf.failed_attempts} kegagalan)`)}
                              className="inline-flex items-center gap-1 px-2 py-0.5 text-xs font-semibold rounded-md bg-rose-500/15 hover:bg-rose-500 text-rose-500 hover:text-white transition-colors"
                            >
                              <Ban className="w-3 h-3" />
                              Blokir
                            </button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>

      {/* Modal Dialog: Blokir Alamat IP dengan TTL */}
      {isBanModalOpen && (
        <ModalOverlay
          role="dialog"
          aria-modal="true"
          aria-labelledby="ban-ip-modal-title"
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-200"
          onDismiss={() => setIsBanModalOpen(false)}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-2xl w-full max-w-md shadow-2xl overflow-hidden dialog-panel">
            <div className="p-5 border-b border-[var(--line)] flex items-center justify-between dialog-header">
              <h3 id="ban-ip-modal-title" className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Ban className="w-4 h-4 text-rose-500" />
                Blokir Alamat IP (Nginx Deny)
              </h3>
              <button
                type="button"
                onClick={() => setIsBanModalOpen(false)}
                className="p-1 rounded-lg text-[var(--ink-muted)] hover:bg-[var(--surface-hover)] transition-colors"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <form onSubmit={handleBanSubmit} className="p-5 dialog-form">
              <div className="dialog-body space-y-4">
                {banFormError && (
                  <div className="p-3 bg-rose-500/10 border border-rose-500/20 text-rose-500 rounded-lg text-xs">
                    {banFormError}
                  </div>
                )}

                {/* IP Input */}
                <div>
                  <label className="block text-xs font-medium text-[var(--ink)] mb-1">Alamat IP atau CIDR *</label>
                  <input
                    type="text"
                    required
                    placeholder="Contoh: 198.51.100.44 atau 10.0.0.0/24"
                    value={banInputIp}
                    onChange={(e) => setBanInputIp(e.target.value)}
                    className="w-full px-3 py-2 text-xs font-mono rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  />
                </div>

                {/* TTL Selection */}
                <div>
                  <label className="block text-xs font-medium text-[var(--ink)] mb-1.5">
                    Durasi Pemblokiran (TTL)
                  </label>
                  <div className="grid grid-cols-3 gap-2">
                    {TTL_OPTIONS.map((opt) => (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={() => setBanSelectedTtl(opt.value)}
                        className={`px-2.5 py-1.5 text-xs font-medium rounded-lg border text-center transition-all ${
                          banSelectedTtl === opt.value
                            ? 'border-[var(--primary)] bg-[var(--primary)]/10 text-[var(--primary)] font-semibold'
                            : 'border-[var(--line)] text-[var(--ink-muted)] hover:border-[var(--ink-muted)]'
                        }`}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                  <p className="text-[11px] text-[var(--ink-muted)] mt-1.5">
                    Setelah waktu TTL habis, IP akan otomatis dibuka blokirnya dari Nginx.
                  </p>
                </div>

                {/* Reason Input */}
                <div>
                  <label className="block text-xs font-medium text-[var(--ink)] mb-1">Alasan Pemblokiran</label>
                  <input
                    type="text"
                    placeholder="Contoh: Pemindaian berkas rahasia .env"
                    value={banInputReason}
                    onChange={(e) => setBanInputReason(e.target.value)}
                    className="w-full px-3 py-2 text-xs rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  />
                </div>

                {/* Modal Actions */}
              </div>
              <div className="pt-3 border-t border-[var(--line)] flex items-center justify-end gap-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsBanModalOpen(false)}
                  disabled={submittingBan}
                  className="px-3.5 py-2 text-xs font-medium rounded-lg border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={submittingBan}
                  className="px-4 py-2 text-xs font-semibold rounded-lg bg-rose-600 hover:bg-rose-700 text-white transition-colors flex items-center gap-1.5 shadow-xs disabled:opacity-50"
                >
                  {submittingBan ? (
                    <>
                      <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                      Menerapkan...
                    </>
                  ) : (
                    <>
                      <Ban className="w-3.5 h-3.5" />
                      Terapkan Pemblokiran
                    </>
                  )}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
