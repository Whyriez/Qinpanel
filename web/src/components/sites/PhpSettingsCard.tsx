import React, { useState, useEffect } from 'react';
import {
  Save,
  AlertCircle,
  CheckCircle2,
  Cpu,
  FileCode,
} from 'lucide-react';
import {
  fetchSitePhpSettings,
  updateSitePhpSettings,
} from '../../api/client';
import {
  PhpSettings,
  PhpSettingsLimits,
  SitePhpSettingsResponse,
} from '../../types/api';

export interface PhpSettingsCardProps {
  siteId: number;
  phpVersion: string;
  onSaved?: () => void;
}

export const PhpSettingsCard: React.FC<PhpSettingsCardProps> = ({
  siteId,
  phpVersion,
  onSaved,
}) => {
  const [settings, setSettings] = useState<PhpSettings | null>(null);
  const [limits, setLimits] = useState<PhpSettingsLimits | null>(null);
  const [adminIni, setAdminIni] = useState<string>('');
  const [isAdmin, setIsAdmin] = useState<boolean>(false);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isSaving, setIsSaving] = useState<boolean>(false);
  const [statusMessage, setStatusMessage] = useState<{
    type: 'success' | 'error';
    text: string;
  } | null>(null);

  const loadSettings = async () => {
    setIsLoading(true);
    setStatusMessage(null);
    try {
      const data: SitePhpSettingsResponse = await fetchSitePhpSettings(siteId);
      setSettings(data.settings);
      setLimits(data.limits);
      setAdminIni(data.admin_ini || '');
      setIsAdmin(data.is_admin);
    } catch (err: unknown) {
      const errorMsg =
        err instanceof Error ? err.message : 'Gagal memuat pengaturan PHP';
      setStatusMessage({ type: 'error', text: errorMsg });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadSettings();
  }, [siteId]);

  const handleChange = <K extends keyof PhpSettings>(
    field: K,
    value: PhpSettings[K]
  ) => {
    if (!settings) return;
    setSettings({
      ...settings,
      [field]: value,
    });
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!settings || !limits) return;

    // Client-side validations
    if (settings.memory_limit_mb > limits.memory_limit_mb) {
      setStatusMessage({
        type: 'error',
        text: `Batas memori tidak boleh melebihi jatah paket (${limits.memory_limit_mb} MB).`,
      });
      return;
    }
    if (settings.upload_max_filesize_mb > limits.upload_limit_mb) {
      setStatusMessage({
        type: 'error',
        text: `Batas upload berkas tidak boleh melebihi jatah paket (${limits.upload_limit_mb} MB).`,
      });
      return;
    }
    if (settings.post_max_size_mb < settings.upload_max_filesize_mb) {
      setStatusMessage({
        type: 'error',
        text: 'Ukuran post_max_size harus lebih besar atau sama dengan upload_max_filesize.',
      });
      return;
    }

    setIsSaving(true);
    setStatusMessage(null);
    try {
      await updateSitePhpSettings(siteId, {
        settings,
        admin_ini: isAdmin ? adminIni : undefined,
      });
      setStatusMessage({
        type: 'success',
        text: 'Pengaturan PHP berhasil disimpan dan diterapkan.',
      });
      if (onSaved) {
        onSaved();
      }
    } catch (err: unknown) {
      const errorMsg =
        err instanceof Error ? err.message : 'Gagal menyimpan pengaturan PHP';
      setStatusMessage({ type: 'error', text: errorMsg });
    } finally {
      setIsSaving(false);
    }
  };

  if (isLoading) {
    return (
      <div className="p-8 text-center text-[var(--ink-muted)]">
        Memuat konfigurasi PHP...
      </div>
    );
  }

  if (!settings || !limits) {
    return (
      <div className="p-6 bg-[var(--surface)] rounded-[var(--radius-md)] border border-[var(--line)]">
        <p className="text-sm text-red-500">
          Gagal mengambil konfigurasi PHP. Silakan muat ulang halaman.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {statusMessage && (
        <div
          role="alert"
          className={`p-4 rounded-[var(--radius-md)] border flex items-center gap-3 text-sm ${
            statusMessage.type === 'success'
              ? 'bg-emerald-50 text-emerald-800 border-emerald-200 dark:bg-emerald-950/30 dark:text-emerald-300 dark:border-emerald-800'
              : 'bg-red-50 text-red-800 border-red-200 dark:bg-red-950/30 dark:text-red-300 dark:border-red-800'
          }`}
        >
          {statusMessage.type === 'success' ? (
            <CheckCircle2 className="w-5 h-5 shrink-0 text-emerald-600 dark:text-emerald-400" />
          ) : (
            <AlertCircle className="w-5 h-5 shrink-0 text-red-600 dark:text-red-400" />
          )}
          <span>{statusMessage.text}</span>
        </div>
      )}

      <form onSubmit={handleSave} className="space-y-6">
        {/* Curated Directives Card */}
        <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-6 space-y-6">
          <div className="flex items-center justify-between border-b border-[var(--line)] pb-4">
            <div>
              <h3 className="text-base font-semibold text-[var(--ink)] flex items-center gap-2">
                <Cpu className="w-5 h-5 text-[var(--primary)]" />
                Direktif PHP Terkurasi ({phpVersion})
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-1">
                Sesuaikan batas sumber daya dan eksekusi PHP yang dibatasi secara aman oleh paket hosting.
              </p>
            </div>
            <div className="text-xs px-2.5 py-1 rounded bg-[var(--raised)] border border-[var(--line)] text-[var(--ink-muted)]">
              Maks RAM Paket: {limits.memory_limit_mb} MB
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            {/* Memory Limit */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-memory-limit"
                className="text-xs font-medium text-[var(--ink)] flex items-center justify-between"
              >
                <span>Batas Memori (memory_limit)</span>
                <span className="text-[10px] text-[var(--ink-muted)]">
                  Maks: {limits.memory_limit_mb} MB
                </span>
              </label>
              <div className="relative">
                <input
                  id="php-memory-limit"
                  type="number"
                  min={16}
                  max={limits.memory_limit_mb}
                  value={settings.memory_limit_mb}
                  onChange={(e) =>
                    handleChange('memory_limit_mb', parseInt(e.target.value) || 16)
                  }
                  className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
                />
                <span className="absolute right-3 top-2.5 text-xs text-[var(--ink-muted)]">
                  MB
                </span>
              </div>
            </div>

            {/* Upload Max Filesize */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-upload-limit"
                className="text-xs font-medium text-[var(--ink)] flex items-center justify-between"
              >
                <span>Batas Unggah Berkas (upload_max_filesize)</span>
                <span className="text-[10px] text-[var(--ink-muted)]">
                  Maks: {limits.upload_limit_mb} MB
                </span>
              </label>
              <div className="relative">
                <input
                  id="php-upload-limit"
                  type="number"
                  min={1}
                  max={limits.upload_limit_mb}
                  value={settings.upload_max_filesize_mb}
                  onChange={(e) =>
                    handleChange('upload_max_filesize_mb', parseInt(e.target.value) || 1)
                  }
                  className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
                />
                <span className="absolute right-3 top-2.5 text-xs text-[var(--ink-muted)]">
                  MB
                </span>
              </div>
            </div>

            {/* Post Max Size */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-post-max"
                className="text-xs font-medium text-[var(--ink)] flex items-center justify-between"
              >
                <span>Ukuran POST Maksimal (post_max_size)</span>
                <span className="text-[10px] text-[var(--ink-muted)]">
                  Harus &gt;= upload_max_filesize
                </span>
              </label>
              <div className="relative">
                <input
                  id="php-post-max"
                  type="number"
                  min={1}
                  max={limits.upload_limit_mb * 2}
                  value={settings.post_max_size_mb}
                  onChange={(e) =>
                    handleChange('post_max_size_mb', parseInt(e.target.value) || 1)
                  }
                  className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
                />
                <span className="absolute right-3 top-2.5 text-xs text-[var(--ink-muted)]">
                  MB
                </span>
              </div>
            </div>

            {/* Max Execution Time */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-max-exec"
                className="text-xs font-medium text-[var(--ink)] flex items-center justify-between"
              >
                <span>Waktu Eksekusi Maksimal (max_execution_time)</span>
                <span className="text-[10px] text-[var(--ink-muted)]">
                  Maks: {limits.max_execution_time} dtk
                </span>
              </label>
              <div className="relative">
                <input
                  id="php-max-exec"
                  type="number"
                  min={1}
                  max={limits.max_execution_time}
                  value={settings.max_execution_time}
                  onChange={(e) =>
                    handleChange('max_execution_time', parseInt(e.target.value) || 1)
                  }
                  className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
                />
                <span className="absolute right-3 top-2.5 text-xs text-[var(--ink-muted)]">
                  detik
                </span>
              </div>
            </div>

            {/* Max Input Vars */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-max-input-vars"
                className="text-xs font-medium text-[var(--ink)] flex items-center justify-between"
              >
                <span>Batas Variabel Input (max_input_vars)</span>
                <span className="text-[10px] text-[var(--ink-muted)]">
                  Maks: {limits.max_input_vars.toLocaleString()}
                </span>
              </label>
              <input
                id="php-max-input-vars"
                type="number"
                min={100}
                max={limits.max_input_vars}
                value={settings.max_input_vars}
                onChange={(e) =>
                  handleChange('max_input_vars', parseInt(e.target.value) || 100)
                }
                className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
              />
            </div>

            {/* Timezone */}
            <div className="space-y-1.5">
              <label
                htmlFor="php-timezone"
                className="text-xs font-medium text-[var(--ink)]"
              >
                Zona Waktu (date.timezone)
              </label>
              <input
                id="php-timezone"
                type="text"
                value={settings.date_timezone}
                placeholder="UTC atau Asia/Jakarta"
                onChange={(e) => handleChange('date_timezone', e.target.value)}
                className="w-full px-3 py-2 text-sm bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
              />
            </div>
          </div>

          {/* Toggle Switches */}
          <div className="pt-4 border-t border-[var(--line)] grid grid-cols-1 sm:grid-cols-2 gap-4">
            <label className="flex items-start gap-3 p-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
              <input
                type="checkbox"
                checked={settings.display_errors}
                onChange={(e) => handleChange('display_errors', e.target.checked)}
                className="mt-0.5 rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
              />
              <div className="text-xs">
                <span className="font-semibold text-[var(--ink)] block">
                  Tampilkan Error (display_errors)
                </span>
                <span className="text-[var(--ink-muted)] mt-0.5 block">
                  Aktifkan hanya untuk debugging. Nonaktifkan di produksi untuk keamanan.
                </span>
              </div>
            </label>

            <label className="flex items-start gap-3 p-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
              <input
                type="checkbox"
                checked={settings.opcache_enable}
                onChange={(e) => handleChange('opcache_enable', e.target.checked)}
                className="mt-0.5 rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
              />
              <div className="text-xs">
                <span className="font-semibold text-[var(--ink)] block">
                  OPcache Bytecode Cache
                </span>
                <span className="text-[var(--ink-muted)] mt-0.5 block">
                  Meningkatkan kecepatan eksekusi skrip PHP secara signifikan.
                </span>
              </div>
            </label>
          </div>
        </div>

        {/* Admin Raw INI Directive Editor */}
        {isAdmin && (
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-6 space-y-4">
            <div className="flex items-center justify-between border-b border-[var(--line)] pb-3">
              <div className="flex items-center gap-2">
                <FileCode className="w-5 h-5 text-amber-500" />
                <h3 className="text-base font-semibold text-[var(--ink)]">
                  Direktif INI Tambahan (Khusus Administrator)
                </h3>
              </div>
              <span className="text-xs px-2 py-0.5 rounded bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-medium">
                Admin Only
              </span>
            </div>

            <p className="text-xs text-[var(--ink-muted)]">
              Masukkan direktif INI tingkat lanjut baris demi baris (contoh:{' '}
              <code>allow_url_fopen = On</code>, <code>error_reporting = E_ALL</code>).
              Direktif berbahaya atau yang mengancam isolasi (seperti <code>open_basedir</code>,{' '}
              <code>extension</code>) akan ditolak secara otomatis oleh sistem.
            </p>

            <div>
              <textarea
                rows={5}
                value={adminIni}
                onChange={(e) => setAdminIni(e.target.value)}
                placeholder="allow_url_fopen = On&#10;default_socket_timeout = 60"
                className="w-full font-mono text-xs p-3 bg-[var(--surface-input)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)] resize-y"
              />
            </div>
          </div>
        )}

        {/* Submit Button */}
        <div className="flex justify-end">
          <button
            type="submit"
            disabled={isSaving}
            className="flex items-center gap-2 px-5 py-2.5 rounded-[var(--radius-md)] bg-[var(--primary)] hover:bg-[var(--primary-hover)] text-white text-sm font-medium transition-colors disabled:opacity-50"
          >
            <Save className="w-4 h-4" />
            {isSaving ? 'Menyimpan...' : 'Simpan Pengaturan PHP'}
          </button>
        </div>
      </form>
    </div>
  );
};
export default PhpSettingsCard;
