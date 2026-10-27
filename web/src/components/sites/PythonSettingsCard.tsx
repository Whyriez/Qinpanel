import React, { useState, useEffect } from 'react';
import { useAuth } from '../../context/AuthContext';
import { RouteLink } from '../../lib/router';
import {
  Sliders,
  Save,
  Loader2,
  AlertTriangle,
  CheckCircle2,
  Plus,
  Trash2,
  Terminal,
  Folder,
  FileCode2,
  Layers,
  Cpu,
} from 'lucide-react';
import {
  getPythonSiteSettings,
  updatePythonSiteSettings,
  getPythonVersions,
} from '../../api/client';
import {
  PythonSiteSettingsDto,
  UpdatePythonSiteSettingsRequest,
  PythonVersionSummaryDto,
} from '../../types/api';

export interface PythonSettingsCardProps {
  siteId: number;
  onSaved?: () => void;
}

export const PythonSettingsCard: React.FC<PythonSettingsCardProps> = ({
  siteId,
  onSaved,
}) => {
  const { user } = useAuth();
  const [, setSettings] = useState<PythonSiteSettingsDto | null>(null);
  const [availableVersions, setAvailableVersions] = useState<PythonVersionSummaryDto[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isSaving, setIsSaving] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Form states
  const [pythonVersion, setPythonVersion] = useState<string>('3.12.9');
  const [frameworkPreset, setFrameworkPreset] = useState<string>('django');
  const [serverType, setServerType] = useState<string>('gunicorn_wsgi');
  const [entrypointType, setEntrypointType] = useState<string>('wsgi');
  const [entrypointValue, setEntrypointValue] = useState<string>('myproject.wsgi:application');
  const [workers, setWorkers] = useState<number>(3);
  const [appDir, setAppDir] = useState<string>('');
  const [venvDir, setVenvDir] = useState<string>('');
  const [staticUrl, setStaticUrl] = useState<string>('/static/');
  const [staticDir, setStaticDir] = useState<string>('');
  const [mediaUrl, setMediaUrl] = useState<string>('/media/');
  const [mediaDir, setMediaDir] = useState<string>('');
  const [autoRestart, setAutoRestart] = useState<boolean>(true);
  const [envList, setEnvList] = useState<Array<{ key: string; value: string }>>([]);

  const loadData = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const [sData, vData] = await Promise.allSettled([
        getPythonSiteSettings(siteId),
        getPythonVersions(),
      ]);

      if (sData.status === 'fulfilled') {
        const s = sData.value;
        setSettings(s);
        setPythonVersion(s.python_version);
        setFrameworkPreset(s.framework_preset);
        setServerType(s.server_type);
        setEntrypointType(s.entrypoint_type);
        setEntrypointValue(s.entrypoint_value);
        setWorkers(s.workers);
        setAppDir(s.app_dir);
        setVenvDir(s.venv_dir);
        setStaticUrl(s.static_url || '');
        setStaticDir(s.static_dir || '');
        setMediaUrl(s.media_url || '');
        setMediaDir(s.media_dir || '');
        setAutoRestart(s.auto_restart);

        const envRows = Object.entries(s.env_vars || {}).map(([key, value]) => ({
          key,
          value,
        }));
        setEnvList(envRows);
      } else {
        throw new Error(sData.reason?.message || 'Gagal memuat pengaturan Python');
      }

      if (vData.status === 'fulfilled') {
        setAvailableVersions(vData.value);
      }
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memuat konfigurasi Python';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, [siteId]);

  const handleFrameworkPresetChange = (preset: string) => {
    setFrameworkPreset(preset);
    if (preset === 'django') {
      setServerType('gunicorn_wsgi');
      setEntrypointType('wsgi');
      setEntrypointValue('myproject.wsgi:application');
      setStaticUrl('/static/');
      setMediaUrl('/media/');
    } else if (preset === 'flask') {
      setServerType('gunicorn_wsgi');
      setEntrypointType('wsgi');
      setEntrypointValue('app:app');
      setStaticUrl('/static/');
      setMediaUrl('');
      setMediaDir('');
    } else if (preset === 'fastapi') {
      setServerType('uvicorn_asgi');
      setEntrypointType('asgi');
      setEntrypointValue('main:app');
      setStaticUrl('');
      setStaticDir('');
      setMediaUrl('');
      setMediaDir('');
    } else {
      setServerType('gunicorn_wsgi');
      setEntrypointType('wsgi');
      setEntrypointValue('wsgi:application');
    }
  };

  const handleAddEnv = () => {
    setEnvList([...envList, { key: '', value: '' }]);
  };

  const handleRemoveEnv = (index: number) => {
    setEnvList(envList.filter((_, i) => i !== index));
  };

  const handleEnvChange = (
    index: number,
    field: 'key' | 'value',
    val: string
  ) => {
    const updated = [...envList];
    updated[index][field] = val;
    setEnvList(updated);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSaving(true);
    setError(null);
    setSuccessMessage(null);

    // Build environment variables record
    const envObj: Record<string, string> = {};
    for (const item of envList) {
      const k = item.key.trim();
      if (k) {
        envObj[k] = item.value;
      }
    }

    const payload: UpdatePythonSiteSettingsRequest = {
      python_version: pythonVersion,
      framework_preset: frameworkPreset,
      server_type: serverType,
      entrypoint_type: entrypointType,
      entrypoint_value: entrypointValue,
      workers: workers,
      app_dir: appDir.trim() || undefined,
      venv_dir: venvDir.trim() || undefined,
      static_url: staticUrl.trim() || undefined,
      static_dir: staticDir.trim() || undefined,
      media_url: mediaUrl.trim() || undefined,
      media_dir: mediaDir.trim() || undefined,
      auto_restart: autoRestart,
      env_vars: envObj,
    };

    try {
      const res = await updatePythonSiteSettings(siteId, payload);
      setSettings(res);
      setSuccessMessage('Pengaturan Python berhasil disimpan dan diterapkan.');
      if (onSaved) onSaved();
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memperbarui pengaturan Python';
      setError(msg);
    } finally {
      setIsSaving(false);
    }
  };

  if (isLoading) {
    return (
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] p-8 text-center text-xs text-[var(--ink-muted)]">
        <Loader2 className="w-5 h-5 animate-spin mx-auto mb-2 text-[var(--primary)]" />
        Memuat konfigurasi Python...
      </div>
    );
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-xs space-y-6"
    >
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[var(--line)] pb-4">
        <div>
          <div className="flex items-center gap-2">
            <Sliders className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              Pengaturan Aplikasi Python (uv + venv)
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Konfigurasi WSGI/ASGI server (Gunicorn / Uvicorn), preset framework, worker, dan direktori statis
          </p>
        </div>

        <button
          type="submit"
          disabled={isSaving}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 disabled:opacity-50 transition-opacity"
        >
          {isSaving ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Save className="w-3.5 h-3.5" />
          )}
          Simpan Perubahan
        </button>
      </div>

      {/* Alerts */}
      {error && (
        <div className="p-3 bg-[var(--danger)]/10 border border-[var(--danger)]/20 rounded-[var(--radius-md)] text-xs text-[var(--danger)] flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}

      {successMessage && (
        <div className="p-3 bg-[var(--success)]/10 border border-[var(--success)]/20 rounded-[var(--radius-md)] text-xs text-[var(--success)] flex items-start gap-2">
          <CheckCircle2 className="w-4 h-4 shrink-0 mt-0.5" />
          <span>{successMessage}</span>
        </div>
      )}

      {/* Grid Settings */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Python Version */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <Terminal className="w-3.5 h-3.5 text-[var(--primary)]" />
            Versi Python (uv runtime)
          </label>
          <select
            value={pythonVersion}
            onChange={(e) => setPythonVersion(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            {availableVersions.length > 0 ? (
              availableVersions.map((v) => (
                <option key={v.version} value={v.version}>
                  Python {v.version} {v.is_default ? '(Default)' : ''}
                </option>
              ))
            ) : (
              <>
                <option value="3.12.9">Python 3.12.9 (Default / Stabil)</option>
                <option value="3.11.11">Python 3.11.11</option>
                <option value="3.10.16">Python 3.10.16</option>
                <option value="3.13.2">Python 3.13.2</option>
              </>
            )}
          </select>
          <p className="text-[11px] text-[var(--ink-muted)]">
            Versi Python dipasang pada server oleh administrator.
          </p>
          {user?.role === 'admin' && <RouteLink href="/runtime?tab=python" className="inline-flex min-h-10 items-center gap-1.5 text-xs font-medium text-[var(--primary)] underline underline-offset-2"><Plus size={13} />Pasang versi Python di Runtime</RouteLink>}
        </div>

        {/* Framework Preset */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <Layers className="w-3.5 h-3.5 text-[var(--primary)]" />
            Preset Framework
          </label>
          <select
            value={frameworkPreset}
            onChange={(e) => handleFrameworkPresetChange(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="django">Django (WSGI + static/media)</option>
            <option value="flask">Flask (WSGI + static)</option>
            <option value="fastapi">FastAPI (ASGI)</option>
            <option value="generic">Generic (Kustom WSGI/ASGI)</option>
          </select>
          <p className="text-[11px] text-[var(--ink-muted)]">
            Otomatis menyesuaikan server dan pemetaan lokasi file statis Nginx
          </p>
        </div>

        {/* Server Type */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <FileCode2 className="w-3.5 h-3.5 text-[var(--primary)]" />
            Tipe Server (WSGI / ASGI)
          </label>
          <select
            value={serverType}
            onChange={(e) => setServerType(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="gunicorn_wsgi">Gunicorn (WSGI - Django/Flask)</option>
            <option value="uvicorn_asgi">Uvicorn (ASGI - FastAPI/Starlette)</option>
            <option value="gunicorn_uvicorn_worker">Gunicorn + Uvicorn Worker (ASGI di Gunicorn)</option>
          </select>
        </div>

        {/* Workers */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <Cpu className="w-3.5 h-3.5 text-[var(--primary)]" />
            Jumlah Worker Proses
          </label>
          <input
            type="number"
            min={1}
            max={16}
            value={workers}
            onChange={(e) => setWorkers(parseInt(e.target.value, 10) || 1)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
          <p className="text-[11px] text-[var(--ink-muted)]">
            Rekomendasi: (2 * core) + 1, dibatasi oleh paket tenant
          </p>
        </div>

        {/* Entrypoint Type & Value */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            Tipe Titik Masuk (Entrypoint)
          </label>
          <select
            value={entrypointType}
            onChange={(e) => setEntrypointType(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="wsgi">WSGI Application</option>
            <option value="asgi">ASGI Application</option>
            <option value="custom">Perintah Kustom</option>
          </select>
        </div>

        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            Modul &amp; Callable Entrypoint
          </label>
          <input
            type="text"
            required
            value={entrypointValue}
            onChange={(e) => setEntrypointValue(e.target.value)}
            placeholder="contoh: myproject.wsgi:application atau main:app"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
          <p className="text-[11px] text-[var(--ink-muted)]">
            Format: &lt;module&gt;:&lt;variable&gt;
          </p>
        </div>

        {/* App Directory */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <Folder className="w-3.5 h-3.5 text-[var(--primary)]" />
            Direktori Aplikasi (Working Dir)
          </label>
          <input
            type="text"
            required
            value={appDir}
            onChange={(e) => setAppDir(e.target.value)}
            placeholder="/home/p_user/sites/domain.com/app"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        {/* Virtualenv Directory */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)] flex items-center gap-1.5">
            <Folder className="w-3.5 h-3.5 text-[var(--primary)]" />
            Direktori Virtualenv (.venv)
          </label>
          <input
            type="text"
            required
            value={venvDir}
            onChange={(e) => setVenvDir(e.target.value)}
            placeholder="/home/p_user/sites/domain.com/.venv"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        {/* Static URL & Dir */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            URL File Statis (Opsional)
          </label>
          <input
            type="text"
            value={staticUrl}
            onChange={(e) => setStaticUrl(e.target.value)}
            placeholder="/static/"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            Direktori Fisik File Statis (Opsional)
          </label>
          <input
            type="text"
            value={staticDir}
            onChange={(e) => setStaticDir(e.target.value)}
            placeholder="/home/p_user/sites/domain.com/static"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        {/* Media URL & Dir */}
        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            URL Media Upload (Opsional)
          </label>
          <input
            type="text"
            value={mediaUrl}
            onChange={(e) => setMediaUrl(e.target.value)}
            placeholder="/media/"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        <div className="space-y-1.5">
          <label className="text-xs font-semibold text-[var(--ink)]">
            Direktori Fisik File Media (Opsional)
          </label>
          <input
            type="text"
            value={mediaDir}
            onChange={(e) => setMediaDir(e.target.value)}
            placeholder="/home/p_user/sites/domain.com/media"
            className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>
      </div>

      {/* Auto Restart Toggle */}
      <div className="pt-2 border-t border-[var(--line)]">
        <label className="flex items-center gap-2 cursor-pointer text-xs font-medium text-[var(--ink)]">
          <input
            type="checkbox"
            checked={autoRestart}
            onChange={(e) => setAutoRestart(e.target.checked)}
            className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
          />
          <span>Aktifkan restart otomatis (systemd Restart=on-failure)</span>
        </label>
      </div>

      {/* Environment Variables */}
      <div className="pt-4 border-t border-[var(--line)] space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-xs font-semibold text-[var(--ink)]">
              Variabel Lingkungan (Environment Variables)
            </h3>
            <p className="text-[11px] text-[var(--ink-muted)]">
              Disimpan aman di file environment unit systemd dengan izin khusus tenant (mode 0600)
            </p>
          </div>
          <button
            type="button"
            onClick={handleAddEnv}
            className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:bg-[var(--line)] transition-colors"
          >
            <Plus className="w-3 h-3" />
            Tambah Variabel
          </button>
        </div>

        {envList.length === 0 ? (
          <div className="p-3 bg-[var(--raised)] rounded-[var(--radius-md)] text-center text-xs text-[var(--ink-muted)] border border-dashed border-[var(--line)]">
            Belum ada variabel lingkungan yang ditambahkan.
          </div>
        ) : (
          <div className="space-y-2">
            {envList.map((item, idx) => (
              <div key={idx} className="flex items-center gap-2">
                <input
                  type="text"
                  placeholder="NAMA_VARIABEL"
                  value={item.key}
                  onChange={(e) => handleEnvChange(idx, 'key', e.target.value)}
                  className="w-1/3 text-xs px-3 py-1.5 font-mono uppercase rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                />
                <input
                  type="text"
                  placeholder="Nilai variabel"
                  value={item.value}
                  onChange={(e) => handleEnvChange(idx, 'value', e.target.value)}
                  className="flex-1 text-xs px-3 py-1.5 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                />
                <button
                  type="button"
                  onClick={() => handleRemoveEnv(idx)}
                  className="p-1.5 text-[var(--ink-muted)] hover:text-[var(--danger)] transition-colors"
                  title="Hapus variabel"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </form>
  );
};
