import React, { useState, useEffect } from 'react';
import {
  Network,
  Save,
  Loader2,
  AlertTriangle,
  CheckCircle2,
  Activity,
  Plus,
  Trash2,
  ShieldAlert,
  Radio,
  Layers,
  ArrowRight,
} from 'lucide-react';
import {
  getProxySiteSettings,
  updateProxySiteSettings,
  probeProxyUpstream,
} from '../../api/client';
import {
  ProxySiteSettingsDto,
  UpdateProxySiteSettingsRequest,
  ProxyHeaderConfig,
  ProxyUpstreamProbeResponse,
} from '../../types/api';

export interface ProxySettingsCardProps {
  siteId: number;
  onSaved?: () => void;
}

export const ProxySettingsCard: React.FC<ProxySettingsCardProps> = ({
  siteId,
  onSaved,
}) => {
  const [, setSettings] = useState<ProxySiteSettingsDto | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isSaving, setIsSaving] = useState<boolean>(false);
  const [isProbing, setIsProbing] = useState<boolean>(false);
  const [probeResult, setProbeResult] = useState<ProxyUpstreamProbeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Form states
  const [upstreamUrl, setUpstreamUrl] = useState<string>('http://127.0.0.1:8080');
  const [preserveHost, setPreserveHost] = useState<boolean>(true);
  const [websocketSupport, setWebsocketSupport] = useState<boolean>(true);
  const [proxyConnectTimeout, setProxyConnectTimeout] = useState<string>('60s');
  const [proxySendTimeout, setProxySendTimeout] = useState<string>('60s');
  const [proxyReadTimeout, setProxyReadTimeout] = useState<string>('60s');
  const [proxyBuffering, setProxyBuffering] = useState<boolean>(true);
  const [requestHeaders, setRequestHeaders] = useState<ProxyHeaderConfig[]>([]);
  const [responseHeaders, setResponseHeaders] = useState<ProxyHeaderConfig[]>([]);
  const [allowExternalUpstream, setAllowExternalUpstream] = useState<boolean>(false);

  const loadData = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getProxySiteSettings(siteId);
      setSettings(data);
      setUpstreamUrl(data.upstream_url);
      setPreserveHost(data.preserve_host);
      setWebsocketSupport(data.websocket_support);
      setProxyConnectTimeout(data.proxy_connect_timeout || '60s');
      setProxySendTimeout(data.proxy_send_timeout || '60s');
      setProxyReadTimeout(data.proxy_read_timeout || '60s');
      setProxyBuffering(data.proxy_buffering ?? true);
      setRequestHeaders(data.custom_request_headers || []);
      setResponseHeaders(data.custom_response_headers || []);
      setAllowExternalUpstream(data.allow_external_upstream);
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memuat pengaturan reverse proxy';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, [siteId]);

  const handleProbe = async () => {
    setIsProbing(true);
    setProbeResult(null);
    setError(null);
    try {
      const res = await probeProxyUpstream(siteId, upstreamUrl.trim());
      setProbeResult(res);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal menguji upstream';
      setError(msg);
    } finally {
      setIsProbing(false);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSaving(true);
    setError(null);
    setSuccessMessage(null);

    const payload: UpdateProxySiteSettingsRequest = {
      upstream_url: upstreamUrl.trim(),
      preserve_host: preserveHost,
      websocket_support: websocketSupport,
      proxy_connect_timeout: proxyConnectTimeout.trim() || undefined,
      proxy_send_timeout: proxySendTimeout.trim() || undefined,
      proxy_read_timeout: proxyReadTimeout.trim() || undefined,
      proxy_buffering: proxyBuffering,
      custom_request_headers: requestHeaders.filter(
        (h) => h.name.trim() !== '' && h.value.trim() !== ''
      ),
      custom_response_headers: responseHeaders.filter(
        (h) => h.name.trim() !== '' && h.value.trim() !== ''
      ),
    };

    try {
      const updated = await updateProxySiteSettings(siteId, payload);
      setSettings(updated);
      setSuccessMessage('Pengaturan reverse proxy berhasil disimpan dan diterapkan');
      if (onSaved) {
        onSaved();
      }
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal menyimpan pengaturan reverse proxy';
      setError(msg);
    } finally {
      setIsSaving(false);
    }
  };

  // Header helpers
  const addRequestHeader = () => {
    setRequestHeaders([...requestHeaders, { name: '', value: '' }]);
  };

  const updateRequestHeader = (
    index: number,
    field: 'name' | 'value',
    val: string
  ) => {
    const next = [...requestHeaders];
    next[index][field] = val;
    setRequestHeaders(next);
  };

  const removeRequestHeader = (index: number) => {
    setRequestHeaders(requestHeaders.filter((_, idx) => idx !== index));
  };

  const addResponseHeader = () => {
    setResponseHeaders([...responseHeaders, { name: '', value: '' }]);
  };

  const updateResponseHeader = (
    index: number,
    field: 'name' | 'value',
    val: string
  ) => {
    const next = [...responseHeaders];
    next[index][field] = val;
    setResponseHeaders(next);
  };

  const removeResponseHeader = (index: number) => {
    setResponseHeaders(responseHeaders.filter((_, idx) => idx !== index));
  };

  if (isLoading) {
    return (
      <div className="bg-slate-900 border border-slate-800 rounded-xl p-8 flex items-center justify-center">
        <Loader2 className="w-6 h-6 animate-spin text-teal-400 mr-3" />
        <span className="text-slate-400">Memuat konfigurasi reverse proxy...</span>
      </div>
    );
  }

  return (
    <div className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden shadow-xl">
      {/* Header */}
      <div className="p-6 border-b border-slate-800 bg-slate-900/50 flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center space-x-3">
          <div className="p-2.5 bg-teal-500/10 border border-teal-500/20 rounded-lg text-teal-400">
            <Network className="w-5 h-5" />
          </div>
          <div>
            <h3 className="text-lg font-semibold text-slate-100 flex items-center gap-2">
              Konfigurasi Reverse Proxy
              <span className="text-xs px-2.5 py-0.5 rounded-full font-mono bg-teal-500/10 text-teal-400 border border-teal-500/20">
                Nginx Standalone
              </span>
            </h3>
            <p className="text-sm text-slate-400">
              Kelola upstream backend, header kustom, timeout koneksi, dan guard SSRF
            </p>
          </div>
        </div>
      </div>

      <form onSubmit={handleSave} className="p-6 space-y-6">
        {/* Error and Success Alerts */}
        {error && (
          <div className="p-4 bg-rose-500/10 border border-rose-500/20 rounded-lg flex items-start text-rose-300 text-sm">
            <AlertTriangle className="w-5 h-5 mr-3 shrink-0 mt-0.5" />
            <div className="flex-1">{error}</div>
          </div>
        )}

        {successMessage && (
          <div className="p-4 bg-emerald-500/10 border border-emerald-500/20 rounded-lg flex items-center text-emerald-300 text-sm">
            <CheckCircle2 className="w-5 h-5 mr-3 shrink-0" />
            <span>{successMessage}</span>
          </div>
        )}

        {/* SSRF Guard Notice */}
        {!allowExternalUpstream ? (
          <div className="p-4 bg-amber-500/10 border border-amber-500/20 rounded-lg flex items-start gap-3">
            <ShieldAlert className="w-5 h-5 text-amber-400 shrink-0 mt-0.5" />
            <div className="text-sm text-amber-200/90 leading-relaxed">
              <span className="font-semibold text-amber-300">
                SSRF Guard Aktif (Hanya Loopback):{' '}
              </span>
              Paket langganan tenant membatasi upstream hanya ke localhost loopback (
              <code className="text-amber-300 font-mono text-xs">127.0.0.1</code>,{' '}
              <code className="text-amber-300 font-mono text-xs">localhost</code>, atau Unix socket{' '}
              <code className="text-amber-300 font-mono text-xs">unix:/...</code>). Upstream jaringan eksternal
              membutuhkan izin admin pada paket langganan.
            </div>
          </div>
        ) : (
          <div className="p-4 bg-teal-500/10 border border-teal-500/20 rounded-lg flex items-start gap-3">
            <Network className="w-5 h-5 text-teal-400 shrink-0 mt-0.5" />
            <div className="text-sm text-teal-200/90 leading-relaxed">
              <span className="font-semibold text-teal-300">
                Izin Upstream Eksternal Aktif:{' '}
              </span>
              Tenant diizinkan meneruskan trafik ke host publik (HTTP/HTTPS). IP privat (LAN) dan endpoint metadata cloud tetap diblokir demi keamanan.
            </div>
          </div>
        )}

        {/* Section 1: Upstream Target & Probe */}
        <div className="space-y-4">
          <label className="block text-sm font-medium text-slate-300">
            Upstream Target URL <span className="text-rose-400">*</span>
          </label>
          <div className="flex flex-col sm:flex-row gap-3">
            <div className="relative flex-1">
              <input
                type="text"
                value={upstreamUrl}
                onChange={(e) => setUpstreamUrl(e.target.value)}
                placeholder="http://127.0.0.1:3000 atau unix:/run/app.sock"
                className="w-full bg-slate-950 border border-slate-700/80 rounded-lg px-4 py-2.5 text-sm text-slate-100 placeholder-slate-500 focus:outline-none focus:border-teal-500 font-mono"
                required
              />
            </div>
            <button
              type="button"
              onClick={handleProbe}
              disabled={isProbing || !upstreamUrl.trim()}
              className="inline-flex items-center justify-center px-4 py-2.5 rounded-lg text-sm font-medium bg-slate-800 hover:bg-slate-700 text-teal-300 border border-slate-700 hover:border-teal-500/40 transition-colors disabled:opacity-50 shrink-0"
            >
              {isProbing ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin mr-2" />
                  Menguji...
                </>
              ) : (
                <>
                  <Activity className="w-4 h-4 mr-2 text-teal-400" />
                  Uji Upstream
                </>
              )}
            </button>
          </div>
          <p className="text-xs text-slate-500">
            Contoh: <code className="text-slate-400">http://127.0.0.1:8080</code>,{' '}
            <code className="text-slate-400">https://api.domain.com</code>, atau{' '}
            <code className="text-slate-400">unix:/run/app.sock</code>
          </p>

          {/* Live Probe Feedback Banner */}
          {probeResult && (
            <div
              className={`p-3.5 rounded-lg border text-sm flex items-center justify-between gap-4 transition-all ${
                probeResult.healthy
                  ? 'bg-emerald-950/40 border-emerald-500/30 text-emerald-300'
                  : 'bg-rose-950/40 border-rose-500/30 text-rose-300'
              }`}
            >
              <div className="flex items-center gap-2.5">
                {probeResult.healthy ? (
                  <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
                ) : (
                  <AlertTriangle className="w-4 h-4 text-rose-400 shrink-0" />
                )}
                <span>{probeResult.message}</span>
              </div>
              <div className="flex items-center gap-3 text-xs font-mono shrink-0">
                {probeResult.status_code && (
                  <span className="px-2 py-0.5 bg-slate-900/60 rounded border border-slate-700/60">
                    HTTP {probeResult.status_code}
                  </span>
                )}
                <span className="text-slate-400">
                  {probeResult.latency_ms} ms
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Section 2: Routing & Proxy Flags */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-2 border-t border-slate-800/80">
          <label className="flex items-start gap-3 p-3.5 bg-slate-950/50 border border-slate-800 rounded-lg cursor-pointer hover:border-slate-700 transition-colors">
            <input
              type="checkbox"
              checked={preserveHost}
              onChange={(e) => setPreserveHost(e.target.checked)}
              className="mt-1 rounded bg-slate-800 border-slate-700 text-teal-500 focus:ring-teal-500 focus:ring-offset-slate-900"
            />
            <div className="space-y-0.5">
              <span className="text-sm font-medium text-slate-200">
                Preserve Host Header ($host)
              </span>
              <p className="text-xs text-slate-400">
                Meneruskan nama domain pengunjung ke upstream, bukan nama host upstream
              </p>
            </div>
          </label>

          <label className="flex items-start gap-3 p-3.5 bg-slate-950/50 border border-slate-800 rounded-lg cursor-pointer hover:border-slate-700 transition-colors">
            <input
              type="checkbox"
              checked={websocketSupport}
              onChange={(e) => setWebsocketSupport(e.target.checked)}
              className="mt-1 rounded bg-slate-800 border-slate-700 text-teal-500 focus:ring-teal-500 focus:ring-offset-slate-900"
            />
            <div className="space-y-0.5">
              <span className="text-sm font-medium text-slate-200">
                WebSocket Upgrade Support
              </span>
              <p className="text-xs text-slate-400">
                Mengizinkan koneksi WebSocket langsung via Upgrade & Connection headers
              </p>
            </div>
          </label>
        </div>

        {/* Section 3: Timeouts and Buffering */}
        <div className="pt-2 border-t border-slate-800/80">
          <h4 className="text-sm font-medium text-slate-200 mb-3 flex items-center gap-2">
            <Layers className="w-4 h-4 text-slate-400" />
            Timeout dan Buffering
          </h4>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">
                Connect Timeout
              </label>
              <input
                type="text"
                value={proxyConnectTimeout}
                onChange={(e) => setProxyConnectTimeout(e.target.value)}
                placeholder="60s"
                className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-teal-500"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">
                Send Timeout
              </label>
              <input
                type="text"
                value={proxySendTimeout}
                onChange={(e) => setProxySendTimeout(e.target.value)}
                placeholder="60s"
                className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-teal-500"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">
                Read Timeout
              </label>
              <input
                type="text"
                value={proxyReadTimeout}
                onChange={(e) => setProxyReadTimeout(e.target.value)}
                placeholder="60s"
                className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-teal-500"
              />
            </div>
          </div>

          <div className="mt-3.5">
            <label className="inline-flex items-center gap-2.5 text-sm text-slate-300 cursor-pointer">
              <input
                type="checkbox"
                checked={proxyBuffering}
                onChange={(e) => setProxyBuffering(e.target.checked)}
                className="rounded bg-slate-800 border-slate-700 text-teal-500 focus:ring-teal-500 focus:ring-offset-slate-900"
              />
              <span>Aktifkan Proxy Buffering (Nginx membaca dan menyimpan respons)</span>
            </label>
          </div>
        </div>

        {/* Section 4: Custom Request Headers */}
        <div className="pt-2 border-t border-slate-800/80">
          <div className="flex items-center justify-between mb-3">
            <div>
              <h4 className="text-sm font-medium text-slate-200 flex items-center gap-2">
                <Radio className="w-4 h-4 text-teal-400" />
                Custom Request Headers (proxy_set_header)
              </h4>
              <p className="text-xs text-slate-500">
                Header tambahan yang dikirim dari Nginx ke server upstream backend
              </p>
            </div>
            <button
              type="button"
              onClick={addRequestHeader}
              className="inline-flex items-center text-xs px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-teal-300 rounded border border-slate-700 transition-colors"
            >
              <Plus className="w-3.5 h-3.5 mr-1" />
              Tambah Header
            </button>
          </div>

          {requestHeaders.length === 0 ? (
            <p className="text-xs text-slate-500 italic py-2">
              Belum ada header request tambahan. Standard headers (X-Real-IP, X-Forwarded-*) otomatis disertakan.
            </p>
          ) : (
            <div className="space-y-2">
              {requestHeaders.map((hdr, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <input
                    type="text"
                    value={hdr.name}
                    onChange={(e) => updateRequestHeader(idx, 'name', e.target.value)}
                    placeholder="Nama Header (e.g. X-Custom-Key)"
                    className="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-teal-500"
                  />
                  <ArrowRight className="w-3.5 h-3.5 text-slate-600 shrink-0" />
                  <input
                    type="text"
                    value={hdr.value}
                    onChange={(e) => updateRequestHeader(idx, 'value', e.target.value)}
                    placeholder="Nilai (e.g. secret_token)"
                    className="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-teal-500"
                  />
                  <button
                    type="button"
                    onClick={() => removeRequestHeader(idx)}
                    className="p-1.5 text-slate-500 hover:text-rose-400 transition-colors"
                    title="Hapus header"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Section 5: Custom Response Headers */}
        <div className="pt-2 border-t border-slate-800/80">
          <div className="flex items-center justify-between mb-3">
            <div>
              <h4 className="text-sm font-medium text-slate-200 flex items-center gap-2">
                <Radio className="w-4 h-4 text-emerald-400" />
                Custom Response Headers (add_header)
              </h4>
              <p className="text-xs text-slate-500">
                Header tambahan yang dikirim dari Nginx ke peramban pengguna
              </p>
            </div>
            <button
              type="button"
              onClick={addResponseHeader}
              className="inline-flex items-center text-xs px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-teal-300 rounded border border-slate-700 transition-colors"
            >
              <Plus className="w-3.5 h-3.5 mr-1" />
              Tambah Header
            </button>
          </div>

          {responseHeaders.length === 0 ? (
            <p className="text-xs text-slate-500 italic py-2">
              Belum ada header respons tambahan.
            </p>
          ) : (
            <div className="space-y-2">
              {responseHeaders.map((hdr, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <input
                    type="text"
                    value={hdr.name}
                    onChange={(e) => updateResponseHeader(idx, 'name', e.target.value)}
                    placeholder="Nama Header (e.g. X-Served-By)"
                    className="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-teal-500"
                  />
                  <ArrowRight className="w-3.5 h-3.5 text-slate-600 shrink-0" />
                  <input
                    type="text"
                    value={hdr.value}
                    onChange={(e) => updateResponseHeader(idx, 'value', e.target.value)}
                    placeholder="Nilai (e.g. Qinpanel-Proxy)"
                    className="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-teal-500"
                  />
                  <button
                    type="button"
                    onClick={() => removeResponseHeader(idx)}
                    className="p-1.5 text-slate-500 hover:text-rose-400 transition-colors"
                    title="Hapus header"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Footer Submit */}
        <div className="pt-4 border-t border-slate-800 flex justify-end">
          <button
            type="submit"
            disabled={isSaving}
            className="inline-flex items-center px-5 py-2.5 rounded-lg text-sm font-medium text-white bg-teal-600 hover:bg-teal-500 shadow-lg shadow-teal-900/30 transition-all disabled:opacity-50"
          >
            {isSaving ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin mr-2" />
                Menerapkan...
              </>
            ) : (
              <>
                <Save className="w-4 h-4 mr-2" />
                Simpan & Terapkan Proxy
              </>
            )}
          </button>
        </div>
      </form>
    </div>
  );
};
