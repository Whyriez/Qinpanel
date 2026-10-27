import React, { useState, useEffect } from 'react';
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
  Hash,
  RotateCw,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getNodeSiteSettings,
  updateNodeSiteSettings,
  fetchNodeVersions,
} from '../../api/client';
import {
  NodeSiteSettingsDto,
  UpdateNodeSiteSettingsRequest,
  NodeVersionItem,
} from '../../types/api';

export interface NodeSettingsCardProps {
  siteId: number;
  onSaved?: () => void;
}

export const NodeSettingsCard: React.FC<NodeSettingsCardProps> = ({
  siteId,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<NodeSiteSettingsDto | null>(null);
  const [installedVersions, setInstalledVersions] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isSaving, setIsSaving] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Form states
  const [nodeVersion, setNodeVersion] = useState<string>('22.14.0');
  const [port, setPort] = useState<number>(20000);
  const [appDir, setAppDir] = useState<string>('');
  const [entrypointType, setEntrypointType] = useState<string>('script');
  const [entrypointValue, setEntrypointValue] = useState<string>('index.js');
  const [autoRestart, setAutoRestart] = useState<boolean>(true);
  const [envList, setEnvList] = useState<Array<{ key: string; value: string }>>([]);

  const loadData = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const [sData, vData] = await Promise.allSettled([
        getNodeSiteSettings(siteId),
        fetchNodeVersions(),
      ]);

      if (sData.status === 'fulfilled') {
        const s = sData.value;
        setSettings(s);
        setNodeVersion(s.node_version);
        setPort(s.port);
        setAppDir(s.app_dir);
        setEntrypointType(s.entrypoint_type);
        setEntrypointValue(s.entrypoint_value || 'index.js');
        setAutoRestart(s.auto_restart);

        const envRows = Object.entries(s.env_vars || {}).map(([key, value]) => ({
          key,
          value,
        }));
        setEnvList(envRows);
      } else {
        throw new Error(sData.reason?.message || 'Gagal memuat pengaturan Node.js');
      }

      if (vData.status === 'fulfilled' && vData.value?.versions) {
        const installed = vData.value.versions
          .filter((v: NodeVersionItem) => v.is_installed)
          .map((v: NodeVersionItem) => v.version);
        if (installed.length > 0) {
          setInstalledVersions(installed);
        } else {
          setInstalledVersions(['22.14.0', '20.18.3', '18.20.7']);
        }
      } else {
        setInstalledVersions(['22.14.0', '20.18.3', '18.20.7']);
      }
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memuat konfigurasi Node.js';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, [siteId]);

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
    const envVarsRecord: Record<string, string> = {};
    for (const item of envList) {
      const k = item.key.trim();
      if (k) {
        envVarsRecord[k] = item.value;
      }
    }

    const payload: UpdateNodeSiteSettingsRequest = {
      node_version: nodeVersion,
      port,
      app_dir: appDir.trim() || undefined,
      entrypoint_type: entrypointType,
      entrypoint_value: entrypointValue.trim() || undefined,
      auto_restart: autoRestart,
      env_vars: envVarsRecord,
    };

    try {
      const res = await updateNodeSiteSettings(siteId, payload);
      setSettings(res);
      setSuccessMessage(t('sites.node.settingsSaved'));
      if (onSaved) onSaved();
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal menyimpan pengaturan Node.js';
      setError(msg);
    } finally {
      setIsSaving(false);
    }
  };

  if (isLoading && !settings) {
    return (
      <div className="p-8 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] text-center text-xs text-[var(--ink-muted)]">
        <Loader2 className="w-5 h-5 animate-spin mx-auto mb-2 text-[var(--primary)]" />
        Memuat konfigurasi Node.js...
      </div>
    );
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-6 space-y-6"
    >
      <div className="flex items-center justify-between border-b border-[var(--line)] pb-4">
        <div className="flex items-center gap-2.5">
          <Sliders className="w-4 h-4 text-[var(--primary)]" />
          <h4 className="text-sm font-semibold text-[var(--ink)]">
            {t('sites.node.settingsTitle')}
          </h4>
        </div>
      </div>

      {error && (
        <div className="p-3 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}

      {successMessage && (
        <div className="p-3 rounded-[var(--radius-md)] bg-emerald-500/10 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 text-xs flex items-start gap-2">
          <CheckCircle2 className="w-4 h-4 shrink-0 mt-0.5" />
          <span>{successMessage}</span>
        </div>
      )}

      <div className="grid grid-cols-1 sm:grid-cols-2 gap-5">
        {/* Node.js Version */}
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-[var(--ink)] flex items-center gap-1.5">
            <Terminal className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
            {t('sites.node.nodeVersion')}
          </label>
          <select
            value={nodeVersion}
            onChange={(e) => setNodeVersion(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            {installedVersions.map((v) => (
              <option key={v} value={v}>
                Node.js v{v}
              </option>
            ))}
            {!installedVersions.includes(nodeVersion) && (
              <option value={nodeVersion}>Node.js v{nodeVersion}</option>
            )}
          </select>
          <p className="text-[11px] text-[var(--ink-muted)]">
            Versi binary Node.js terisolasi yang dieksekusi di slice tenant.
          </p>
        </div>

        {/* Port */}
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-[var(--ink)] flex items-center gap-1.5">
            <Hash className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
            {t('sites.node.port')}
          </label>
          <input
            type="number"
            min={1024}
            max={65535}
            value={port}
            onChange={(e) => setPort(parseInt(e.target.value, 10) || 20000)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] font-mono focus:outline-none focus:border-[var(--primary)]"
            required
          />
          <p className="text-[11px] text-[var(--ink-muted)]">
            Port loopback lokal unik (20000 - 29999). Nginx mem-proxy traffic ke 127.0.0.1:{port}.
          </p>
        </div>

        {/* Application Directory */}
        <div className="space-y-1.5 sm:col-span-2">
          <label className="text-xs font-medium text-[var(--ink)] flex items-center gap-1.5">
            <Folder className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
            {t('sites.node.appDir')}
          </label>
          <input
            type="text"
            value={appDir}
            onChange={(e) => setAppDir(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] font-mono focus:outline-none focus:border-[var(--primary)]"
            required
          />
          <p className="text-[11px] text-[var(--ink-muted)]">
            Direktori kerja (WorkingDirectory) aplikasi Node.js Anda.
          </p>
        </div>

        {/* Entrypoint Type */}
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-[var(--ink)]">
            {t('sites.node.entrypointType')}
          </label>
          <select
            value={entrypointType}
            onChange={(e) => setEntrypointType(e.target.value)}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="script">{t('sites.node.entrypointScript')}</option>
            <option value="npm_start">{t('sites.node.entrypointNpmStart')}</option>
            <option value="npm_custom">{t('sites.node.entrypointNpmCustom')}</option>
          </select>
        </div>

        {/* Entrypoint Value */}
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-[var(--ink)]">
            {t('sites.node.entrypointValue')}
          </label>
          <input
            type="text"
            value={entrypointValue}
            onChange={(e) => setEntrypointValue(e.target.value)}
            placeholder={
              entrypointType === 'script'
                ? 'index.js'
                : entrypointType === 'npm_custom'
                ? 'start:prod'
                : ''
            }
            disabled={entrypointType === 'npm_start'}
            className="w-full text-xs px-3 py-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] font-mono focus:outline-none focus:border-[var(--primary)] disabled:opacity-50"
          />
        </div>
      </div>

      {/* Auto Restart Checkbox */}
      <div className="pt-2 border-t border-[var(--line)]">
        <label className="flex items-center gap-2 cursor-pointer select-none">
          <input
            type="checkbox"
            checked={autoRestart}
            onChange={(e) => setAutoRestart(e.target.checked)}
            className="w-4 h-4 rounded text-[var(--primary)] focus:ring-[var(--primary)]"
          />
          <span className="text-xs text-[var(--ink)] font-medium flex items-center gap-1.5">
            <RotateCw className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
            {t('sites.node.autoRestart')}
          </span>
        </label>
      </div>

      {/* Environment Variables Table */}
      <div className="pt-4 border-t border-[var(--line)] space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h5 className="text-xs font-semibold text-[var(--ink)]">
              {t('sites.node.envVars')}
            </h5>
            <p className="text-[11px] text-[var(--ink-muted)]">
              Disimpan aman dengan izin 0600 di berkas environment unit systemd.
            </p>
          </div>
          <button
            type="button"
            onClick={handleAddEnv}
            className="text-xs px-2.5 py-1 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:border-[var(--primary)] transition-colors flex items-center gap-1"
          >
            <Plus className="w-3.5 h-3.5" />
            {t('sites.node.addEnv')}
          </button>
        </div>

        {envList.length === 0 ? (
          <div className="p-4 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-dashed border-[var(--line)] text-center text-[11px] text-[var(--ink-muted)]">
            Belum ada variabel lingkungan kustom. Variabel PORT ({port}) dan NODE_ENV otomatis tersedia.
          </div>
        ) : (
          <div className="space-y-2">
            {envList.map((item, idx) => (
              <div key={idx} className="flex items-center gap-2">
                <input
                  type="text"
                  placeholder={t('sites.node.envKey')}
                  value={item.key}
                  onChange={(e) => handleEnvChange(idx, 'key', e.target.value)}
                  className="w-1/3 text-xs px-2.5 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] font-mono focus:outline-none focus:border-[var(--primary)]"
                />
                <input
                  type="text"
                  placeholder={t('sites.node.envValue')}
                  value={item.value}
                  onChange={(e) => handleEnvChange(idx, 'value', e.target.value)}
                  className="flex-1 text-xs px-2.5 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] font-mono focus:outline-none focus:border-[var(--primary)]"
                />
                <button
                  type="button"
                  onClick={() => handleRemoveEnv(idx)}
                  className="p-1.5 rounded text-[var(--ink-muted)] hover:text-red-500 hover:bg-[var(--raised)] transition-colors"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Submit Button */}
      <div className="pt-4 border-t border-[var(--line)] flex justify-end">
        <button
          type="submit"
          disabled={isSaving}
          className="text-xs px-4 py-2 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary)]/90 disabled:opacity-50 transition-colors flex items-center gap-1.5 shadow-xs font-medium"
        >
          {isSaving ? (
            <>
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
              {t('sites.node.savingSettings')}
            </>
          ) : (
            <>
              <Save className="w-3.5 h-3.5" />
              {t('sites.node.saveSettings')}
            </>
          )}
        </button>
      </div>
    </form>
  );
};
