import { panelHttpUrl } from '../../lib/panelUrl';
import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import {
  GitBranch,
  Play,
  RotateCw,
  Copy,
  Check,
  Key,
  Webhook,
  History,
  AlertCircle,
  CheckCircle2,
  FileText,
  Eye,
  EyeOff,
  ArrowLeftRight,
  Terminal,
  X,
  Loader2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  fetchSiteGitConfig,
  updateSiteGitConfig,
  generateSiteDeployKey,
  triggerSiteDeploy,
  rollbackSiteDeploy,
  fetchSiteDeployments,
  fetchSiteDeploymentDetail,
} from '../../api/client';
import {
  SiteGitConfig,
  UpsertGitConfigRequest,
  SiteDeployment,
} from '../../types/api';

export interface SiteDeployTabProps {
  siteId: number;
  onSiteUpdated?: () => void;
  initialConfig?: SiteGitConfig;
}

export const SiteDeployTab: React.FC<SiteDeployTabProps> = ({
  siteId,
  onSiteUpdated,
  initialConfig,
}) => {
  const { t } = useTranslation();

  // Configuration state
  const [config, setConfig] = useState<SiteGitConfig | null>(initialConfig || null);
  const [repoUrl, setRepoUrl] = useState(initialConfig?.repo_url || '');
  const [branch, setBranch] = useState(initialConfig?.branch || 'main');
  const [docrootSubpath, setDocrootSubpath] = useState(
    initialConfig?.docroot_subpath || ''
  );
  const [buildCmd, setBuildCmd] = useState(initialConfig?.build_cmd || '');
  const [postDeployCmd, setPostDeployCmd] = useState(
    initialConfig?.post_deploy_cmd || ''
  );
  const [autoDeploy, setAutoDeploy] = useState(
    initialConfig?.auto_deploy ?? true
  );
  const [keepReleases, setKeepReleases] = useState(
    initialConfig?.keep_releases || 5
  );

  // Secret display & copy states
  const [showSecret, setShowSecret] = useState(false);
  const [copiedKey, setCopiedKey] = useState(false);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);

  // Deployments history state
  const [deployments, setDeployments] = useState<SiteDeployment[]>([]);
  const [isLoading, setIsLoading] = useState(!initialConfig);
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [isGeneratingKey, setIsGeneratingKey] = useState(false);
  const [isDeploying, setIsDeploying] = useState(false);

  // Log modal state
  const [logModalDeployment, setLogModalDeployment] = useState<SiteDeployment | null>(null);
  const [isLoadingLog, setIsLoadingLog] = useState(false);

  // Rollback confirmation modal state
  const [rollbackTarget, setRollbackTarget] = useState<SiteDeployment | null>(null);
  const [isRollingBack, setIsRollingBack] = useState(false);

  // Toast feedback
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  const showToast = useCallback(
    (text: string, type: 'success' | 'error' = 'success') => {
      setToastMessage({ text, type });
      setTimeout(() => setToastMessage(null), 4000);
    },
    []
  );

  const loadData = useCallback(async () => {
    setIsLoading(true);
    try {
      const [cfg, deps] = await Promise.all([
        fetchSiteGitConfig(siteId),
        fetchSiteDeployments(siteId, 20),
      ]);
      setConfig(cfg);
      setRepoUrl(cfg.repo_url || '');
      setBranch(cfg.branch || 'main');
      setDocrootSubpath(cfg.docroot_subpath || '');
      setBuildCmd(cfg.build_cmd || '');
      setPostDeployCmd(cfg.post_deploy_cmd || '');
      setAutoDeploy(cfg.auto_deploy);
      setKeepReleases(cfg.keep_releases || 5);
      setDeployments(deps);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat data Git deploy';
      showToast(msg, 'error');
    } finally {
      setIsLoading(false);
    }
  }, [siteId, showToast]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Auto-refresh when any deployment is running
  useEffect(() => {
    const hasRunning = deployments.some((d) => d.status === 'running' || d.status === 'queued');
    if (!hasRunning) return;

    const interval = setInterval(async () => {
      try {
        const freshDeps = await fetchSiteDeployments(siteId, 20);
        setDeployments(freshDeps);
        const stillRunning = freshDeps.some((d) => d.status === 'running' || d.status === 'queued');
        if (!stillRunning) {
          clearInterval(interval);
          if (onSiteUpdated) onSiteUpdated();
        }
      } catch {
        // silent fail during poll
      }
    }, 2500);

    return () => clearInterval(interval);
  }, [deployments, siteId, onSiteUpdated]);

  const handleSaveConfig = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSavingConfig(true);
    try {
      const payload: UpsertGitConfigRequest = {
        repo_url: repoUrl.trim(),
        branch: branch.trim() || 'main',
        docroot_subpath: docrootSubpath.trim() || null,
        build_cmd: buildCmd.trim() || null,
        post_deploy_cmd: postDeployCmd.trim() || null,
        auto_deploy: autoDeploy,
        keep_releases: Number(keepReleases) || 5,
      };
      const updated = await updateSiteGitConfig(siteId, payload);
      setConfig(updated);
      showToast('Konfigurasi Git berhasil disimpan', 'success');
      if (onSiteUpdated) onSiteUpdated();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal menyimpan konfigurasi';
      showToast(msg, 'error');
    } finally {
      setIsSavingConfig(false);
    }
  };

  const handleGenerateKey = async () => {
    setIsGeneratingKey(true);
    try {
      const res = await generateSiteDeployKey(siteId);
      setConfig((prev) =>
        prev ? { ...prev, deploy_key_public: res.public_key } : null
      );
      showToast('Kunci deploy Ed25519 baru berhasil dibuat', 'success');
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal membuat kunci deploy';
      showToast(msg, 'error');
    } finally {
      setIsGeneratingKey(false);
    }
  };

  const handleRotateWebhookSecret = async () => {
    setIsSavingConfig(true);
    try {
      const payload: UpsertGitConfigRequest = {
        repo_url: repoUrl.trim(),
        branch: branch.trim() || 'main',
        docroot_subpath: docrootSubpath.trim() || null,
        build_cmd: buildCmd.trim() || null,
        post_deploy_cmd: postDeployCmd.trim() || null,
        auto_deploy: autoDeploy,
        keep_releases: Number(keepReleases) || 5,
        rotate_webhook_secret: true,
      };
      const updated = await updateSiteGitConfig(siteId, payload);
      setConfig(updated);
      showToast('Webhook secret baru berhasil dibuat', 'success');
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memperbarui secret';
      showToast(msg, 'error');
    } finally {
      setIsSavingConfig(false);
    }
  };

  const handleTriggerDeploy = async (overrideBranch?: string) => {
    setIsDeploying(true);
    try {
      const payload = overrideBranch ? { branch: overrideBranch } : undefined;
      const res = await triggerSiteDeploy(siteId, payload);
      showToast('Deployment baru telah dijadwalkan', 'success');
      setDeployments((prev) => [res, ...prev]);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memicu deployment';
      showToast(msg, 'error');
    } finally {
      setIsDeploying(false);
    }
  };

  const handleConfirmRollback = async () => {
    if (!rollbackTarget) return;
    setIsRollingBack(true);
    try {
      await rollbackSiteDeploy(siteId, rollbackTarget.release_id);
      showToast(`Berhasil rollback ke release ${rollbackTarget.release_id}`, 'success');
      setRollbackTarget(null);
      // Reload deployments & trigger callback
      const freshDeps = await fetchSiteDeployments(siteId, 20);
      setDeployments(freshDeps);
      if (onSiteUpdated) onSiteUpdated();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Rollback gagal dilakukan';
      showToast(msg, 'error');
    } finally {
      setIsRollingBack(false);
    }
  };

  const handleOpenLogModal = async (dep: SiteDeployment) => {
    setLogModalDeployment(dep);
    if (!dep.log) {
      setIsLoadingLog(true);
      try {
        const full = await fetchSiteDeploymentDetail(siteId, dep.id);
        setLogModalDeployment(full);
      } catch {
        // use existing partial
      } finally {
        setIsLoadingLog(false);
      }
    }
  };

  const copyToClipboard = (text: string, setCopied: (v: boolean) => void) => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const webhookFullUrl = typeof window !== 'undefined'
    ? panelHttpUrl(`/api/v1/webhooks/deploy/${siteId}`).toString()
    : `/api/v1/webhooks/deploy/${siteId}`;

  return (
    <div className="space-y-6">
      {/* Toast Notification */}
      {toastMessage && (
        <div
          className={`fixed bottom-5 right-5 z-50 px-4 py-3 rounded-[var(--radius-md)] border shadow-lg flex items-center gap-3 text-xs font-semibold ${
            toastMessage.type === 'success'
              ? 'bg-[var(--success-surface)] text-[var(--success)] border-[var(--success-border)]'
              : 'bg-[var(--danger-surface)] text-[var(--danger)] border-[var(--danger-border)]'
          }`}
        >
          {toastMessage.type === 'success' ? (
            <CheckCircle2 className="w-4 h-4 shrink-0" />
          ) : (
            <AlertCircle className="w-4 h-4 shrink-0" />
          )}
          <span>{toastMessage.text}</span>
        </div>
      )}

      {/* Header & Quick Action */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
        <div>
          <div className="flex items-center gap-2">
            <GitBranch className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              {t('git_deploy.title', 'Git Deployment & Webhook')}
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-2xl">
            {t(
              'git_deploy.subtitle',
              'Deploy otomatis dan zero-downtime atomic symlink swap langsung dari repositori Git seperti GitHub atau GitLab.'
            )}
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            onClick={loadData}
            disabled={isLoading}
            className="p-2 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[var(--ink-muted)] transition-colors"
            title="Segarkan data"
          >
            <RotateCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} />
          </button>
          <button
            type="button"
            onClick={() => handleTriggerDeploy()}
            disabled={isDeploying || !repoUrl.trim()}
            className="px-4 py-2 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] text-xs font-semibold flex items-center gap-2 transition-colors disabled:opacity-50 shadow-xs"
          >
            {isDeploying ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Play className="w-4 h-4 fill-current" />
            )}
            <span>
              {isDeploying
                ? t('git_deploy.deploying_btn', 'Sedang Mendeploy...')
                : t('git_deploy.trigger_deploy_btn', 'Deploy Sekarang')}
            </span>
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Card 1: Git Repository Configuration */}
        <div className="p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs flex flex-col justify-between">
          <div>
            <div className="flex items-center gap-2 pb-3 border-b border-[var(--line)]">
              <GitBranch className="w-4 h-4 text-[var(--primary)]" />
              <h3 className="text-sm font-semibold text-[var(--ink)]">
                {t('git_deploy.repo_config_title', 'Konfigurasi Repositori Git')}
              </h3>
            </div>
            <p className="text-xs text-[var(--ink-muted)] mt-2 mb-4">
              {t(
                'git_deploy.repo_config_desc',
                'Tentukan URL repositori Git dan branch yang akan dideploy ke situs ini.'
              )}
            </p>

            <form id="git-config-form" onSubmit={handleSaveConfig} className="space-y-4">
              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                  {t('git_deploy.repo_url_label', 'URL Repositori')}
                </label>
                <input
                  type="text"
                  value={repoUrl}
                  onChange={(e) => setRepoUrl(e.target.value)}
                  placeholder={t(
                    'git_deploy.repo_url_placeholder',
                    'git@github.com:user/repo.git atau https://github.com/user/repo.git'
                  )}
                  className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  required
                />
                <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                  {t(
                    'git_deploy.repo_url_hint',
                    'Mendukung SSH (git@...) dan HTTPS. Untuk repositori privat, gunakan SSH dengan Deploy Key di bawah.'
                  )}
                </p>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    {t('git_deploy.branch_label', 'Branch Target')}
                  </label>
                  <input
                    type="text"
                    value={branch}
                    onChange={(e) => setBranch(e.target.value)}
                    placeholder={t('git_deploy.branch_placeholder', 'main atau master')}
                    className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                    required
                  />
                </div>
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    {t('git_deploy.docroot_subpath_label', 'Subdirektori Root (Docroot)')}
                  </label>
                  <input
                    type="text"
                    value={docrootSubpath}
                    onChange={(e) => setDocrootSubpath(e.target.value)}
                    placeholder={t('git_deploy.docroot_subpath_placeholder', 'public atau dist')}
                    className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  />
                </div>
              </div>

              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                  {t('git_deploy.build_cmd_label', 'Perintah Build (Build Script)')}
                </label>
                <input
                  type="text"
                  value={buildCmd}
                  onChange={(e) => setBuildCmd(e.target.value)}
                  placeholder={t(
                    'git_deploy.build_cmd_placeholder',
                    'Contoh: composer install --no-dev -o && npm run build'
                  )}
                  className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                  {t(
                    'git_deploy.build_cmd_hint',
                    'Dijalankan di direktori rilis baru sebagai pengguna Linux tenant di dalam cgroup terisolasi.'
                  )}
                </p>
              </div>

              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                  {t('git_deploy.post_deploy_cmd_label', 'Perintah Pasca Deploy (Post-deploy Hook)')}
                </label>
                <input
                  type="text"
                  value={postDeployCmd}
                  onChange={(e) => setPostDeployCmd(e.target.value)}
                  placeholder={t(
                    'git_deploy.post_deploy_cmd_placeholder',
                    'Contoh: php artisan migrate --force && php artisan optimize:clear'
                  )}
                  className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                  {t(
                    'git_deploy.post_deploy_cmd_hint',
                    'Dijalankan setelah atomic symlink berhasil diarahkan ke rilis baru.'
                  )}
                </p>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 items-center pt-1">
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    {t('git_deploy.keep_releases_label', 'Jumlah Rilis Tersimpan')}
                  </label>
                  <input
                    type="number"
                    min="1"
                    max="20"
                    value={keepReleases}
                    onChange={(e) => setKeepReleases(Number(e.target.value))}
                    className="w-24 px-3 py-2 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  />
                </div>
                <div className="flex items-center gap-2 pt-4">
                  <input
                    type="checkbox"
                    id="auto-deploy-checkbox"
                    checked={autoDeploy}
                    onChange={(e) => setAutoDeploy(e.target.checked)}
                    className="w-4 h-4 rounded-[var(--radius-xs)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                  <label
                    htmlFor="auto-deploy-checkbox"
                    className="text-xs font-medium text-[var(--ink)] cursor-pointer select-none"
                  >
                    {t('git_deploy.auto_deploy_label', 'Auto Deploy via Webhook')}
                  </label>
                </div>
              </div>
            </form>
          </div>

          <div className="pt-5 mt-4 border-t border-[var(--line)] flex justify-end">
            <button
              type="submit"
              form="git-config-form"
              disabled={isSavingConfig}
              className="px-4 py-2 rounded-[var(--radius-sm)] bg-[var(--surface-raised)] border border-[var(--line)] hover:bg-[var(--raised)] text-xs font-semibold text-[var(--ink)] flex items-center gap-2 transition-colors disabled:opacity-50"
            >
              {isSavingConfig && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              <span>{t('git_deploy.save_config_btn', 'Simpan Konfigurasi')}</span>
            </button>
          </div>
        </div>

        {/* Right Column: Deploy Key & Webhook */}
        <div className="space-y-6">
          {/* Card 2: Ed25519 Deploy Key */}
          <div className="p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)]">
              <div className="flex items-center gap-2">
                <Key className="w-4 h-4 text-[var(--primary)]" />
                <h3 className="text-sm font-semibold text-[var(--ink)]">
                  {t('git_deploy.deploy_key_title', 'Kunci Deploy Ed25519')}
                </h3>
              </div>
              <button
                type="button"
                onClick={handleGenerateKey}
                disabled={isGeneratingKey}
                className="px-2.5 py-1 text-xs font-medium rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] hover:bg-[var(--surface)] text-[var(--ink)] transition-colors flex items-center gap-1.5 disabled:opacity-50"
              >
                {isGeneratingKey ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <RotateCw className="w-3 h-3" />
                )}
                <span>{t('git_deploy.generate_key_btn', 'Buat Kunci Deploy')}</span>
              </button>
            </div>
            <p className="text-xs text-[var(--ink-muted)] mt-2 mb-3">
              {t(
                'git_deploy.deploy_key_desc',
                "Gunakan public key ini di pengaturan 'Deploy Keys' repositori GitHub/GitLab Anda (cukup izin read-only)."
              )}
            </p>

            {config?.deploy_key_public ? (
              <div className="space-y-2">
                <div className="relative">
                  <textarea
                    readOnly
                    rows={3}
                    value={config.deploy_key_public}
                    className="w-full p-2.5 text-[11px] font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] resize-none select-all"
                  />
                  <button
                    type="button"
                    onClick={() =>
                      copyToClipboard(config.deploy_key_public || '', setCopiedKey)
                    }
                    className="absolute top-2 right-2 px-2 py-1 rounded-[var(--radius-xs)] bg-[var(--surface)] border border-[var(--line)] hover:bg-[var(--raised)] text-[11px] font-medium text-[var(--ink)] flex items-center gap-1 shadow-xs transition-colors"
                  >
                    {copiedKey ? (
                      <>
                        <Check className="w-3 h-3 text-[var(--success)]" />
                        <span className="text-[var(--success)]">
                          {t('git_deploy.copied_key', 'Tersalin!')}
                        </span>
                      </>
                    ) : (
                      <>
                        <Copy className="w-3 h-3" />
                        <span>{t('git_deploy.copy_key_btn', 'Salin')}</span>
                      </>
                    )}
                  </button>
                </div>
              </div>
            ) : (
              <div className="p-4 rounded-[var(--radius-sm)] border border-dashed border-[var(--line)] bg-[var(--raised)] text-center text-xs text-[var(--ink-muted)]">
                {t(
                  'git_deploy.no_deploy_key',
                  'Belum ada deploy key yang dibuat. Klik tombol di atas untuk membuat kunci Ed25519 baru.'
                )}
              </div>
            )}
          </div>

          {/* Card 3: Webhook Integration */}
          <div className="p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)]">
              <div className="flex items-center gap-2">
                <Webhook className="w-4 h-4 text-[var(--primary)]" />
                <h3 className="text-sm font-semibold text-[var(--ink)]">
                  {t('git_deploy.webhook_title', 'Integrasi Webhook')}
                </h3>
              </div>
              <button
                type="button"
                onClick={handleRotateWebhookSecret}
                disabled={isSavingConfig}
                className="px-2.5 py-1 text-xs font-medium rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] hover:bg-[var(--surface)] text-[var(--ink)] transition-colors flex items-center gap-1.5 disabled:opacity-50"
              >
                <RotateCw className="w-3 h-3" />
                <span>{t('git_deploy.generate_secret_btn', 'Putar Secret')}</span>
              </button>
            </div>
            <p className="text-xs text-[var(--ink-muted)] mt-2 mb-3">
              {t(
                'git_deploy.webhook_desc',
                'Gunakan endpoint ini di GitHub atau GitLab Webhooks untuk memicu deployment otomatis saat ada push baru.'
              )}
            </p>

            <div className="space-y-3">
              {/* Webhook URL */}
              <div>
                <label className="block text-[11px] font-semibold text-[var(--ink-muted)] mb-1">
                  {t('git_deploy.webhook_url_label', 'URL Webhook')}
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    readOnly
                    value={webhookFullUrl}
                    className="grow px-3 py-1.5 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] select-all"
                  />
                  <button
                    type="button"
                    onClick={() => copyToClipboard(webhookFullUrl, setCopiedUrl)}
                    className="px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-xs font-medium text-[var(--ink)] flex items-center gap-1 shrink-0"
                  >
                    {copiedUrl ? (
                      <Check className="w-3.5 h-3.5 text-[var(--success)]" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                    <span>{copiedUrl ? 'Tersalin' : 'Salin'}</span>
                  </button>
                </div>
              </div>

              {/* Webhook Secret */}
              <div>
                <label className="block text-[11px] font-semibold text-[var(--ink-muted)] mb-1">
                  {t('git_deploy.webhook_secret_label', 'Webhook Secret (HMAC-SHA256)')}
                </label>
                <div className="flex items-center gap-2">
                  <div className="relative grow">
                    <input
                      type={showSecret ? 'text' : 'password'}
                      readOnly
                      value={config?.webhook_secret || '••••••••••••••••••••••••••••••••'}
                      className="w-full px-3 py-1.5 text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] pr-8 select-all"
                    />
                    <button
                      type="button"
                      onClick={() => setShowSecret(!showSecret)}
                      className="absolute right-2 top-2 text-[var(--ink-muted)] hover:text-[var(--ink)]"
                      title={showSecret ? 'Sembunyikan' : 'Tampilkan'}
                    >
                      {showSecret ? (
                        <EyeOff className="w-3.5 h-3.5" />
                      ) : (
                        <Eye className="w-3.5 h-3.5" />
                      )}
                    </button>
                  </div>
                  {config?.webhook_secret && (
                    <button
                      type="button"
                      onClick={() =>
                        copyToClipboard(config.webhook_secret || '', setCopiedSecret)
                      }
                      className="px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-xs font-medium text-[var(--ink)] flex items-center gap-1 shrink-0"
                    >
                      {copiedSecret ? (
                        <Check className="w-3.5 h-3.5 text-[var(--success)]" />
                      ) : (
                        <Copy className="w-3.5 h-3.5" />
                      )}
                      <span>{copiedSecret ? 'Tersalin' : 'Salin'}</span>
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Deployment History Table */}
      <div className="p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
        <div className="flex items-center justify-between pb-3 border-b border-[var(--line)]">
          <div className="flex items-center gap-2">
            <History className="w-4 h-4 text-[var(--primary)]" />
            <h3 className="text-sm font-semibold text-[var(--ink)]">
              {t('git_deploy.history_title', 'Riwayat Deployment')}
            </h3>
          </div>
          <span className="text-xs text-[var(--ink-muted)]">
            {deployments.length} rilis tercatat
          </span>
        </div>
        <p className="text-xs text-[var(--ink-muted)] mt-2 mb-4">
          {t(
            'git_deploy.history_desc',
            'Daftar rilis sebelumnya dengan opsi rollback instan 1-klik dan audit log eksekusi.'
          )}
        </p>

        {deployments.length === 0 ? (
          <div className="p-8 text-center text-xs text-[var(--ink-muted)] border border-dashed border-[var(--line)] rounded-[var(--radius-sm)]">
            {t('git_deploy.no_deployments_yet', 'Belum ada deployment yang tercatat.')}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs border-collapse">
              <thead>
                <tr className="border-b border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)]">
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_release', 'Rilis ID')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_commit', 'Commit')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_branch', 'Branch')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_trigger', 'Pemicu')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_duration', 'Durasi')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_status', 'Status')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold">
                    {t('git_deploy.col_date', 'Tanggal')}
                  </th>
                  <th className="py-2.5 px-3 font-semibold text-right">
                    {t('git_deploy.col_actions', 'Aksi')}
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)]">
                {deployments.map((d) => {
                  const isCurrent = d.is_current;
                  const isRunning = d.status === 'running' || d.status === 'queued';
                  const isSuccess = d.status === 'success';
                  const isFailed = d.status === 'failed';

                  return (
                    <tr
                      key={d.id}
                      className={`hover:bg-[var(--raised)] transition-colors ${
                        isCurrent ? 'bg-[var(--raised)]/40 font-medium' : ''
                      }`}
                    >
                      <td className="py-2.5 px-3 font-mono text-[11px] text-[var(--ink)] flex items-center gap-2">
                        <span>{d.release_id}</span>
                        {isCurrent && (
                          <span className="px-1.5 py-0.5 rounded-[var(--radius-xs)] bg-[var(--primary)] text-white text-[10px] font-semibold">
                            {t('git_deploy.current_badge', 'Aktif')}
                          </span>
                        )}
                      </td>
                      <td className="py-2.5 px-3 text-[11px]">
                        {d.commit_hash ? (
                          <div className="space-y-0.5">
                            <span className="font-mono text-[var(--primary)] font-semibold">
                              {d.commit_hash.slice(0, 7)}
                            </span>
                            {d.commit_message && (
                              <p className="text-[var(--ink-muted)] truncate max-w-xs">
                                {d.commit_message}
                              </p>
                            )}
                          </div>
                        ) : (
                          <span className="text-[var(--ink-muted)]">-</span>
                        )}
                      </td>
                      <td className="py-2.5 px-3 font-mono text-[11px] text-[var(--ink)]">
                        {d.branch}
                      </td>
                      <td className="py-2.5 px-3 capitalize text-[11px] text-[var(--ink-muted)]">
                        {d.trigger_type.replace('_', ' ')}
                      </td>
                      <td className="py-2.5 px-3 text-[11px] text-[var(--ink-muted)] font-mono">
                        {d.duration_ms != null
                          ? `${(d.duration_ms / 1000).toFixed(1)}s`
                          : '-'}
                      </td>
                      <td className="py-2.5 px-3">
                        <span
                          className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-semibold ${
                            isRunning
                              ? 'bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400'
                              : isSuccess
                              ? 'bg-emerald-100 dark:bg-emerald-900/30 text-emerald-700 dark:text-emerald-400'
                              : 'bg-rose-100 dark:bg-rose-900/30 text-rose-700 dark:text-rose-400'
                          }`}
                        >
                          {isRunning && <Loader2 className="w-3 h-3 animate-spin" />}
                          {isSuccess && <CheckCircle2 className="w-3 h-3" />}
                          {isFailed && <AlertCircle className="w-3 h-3" />}
                          <span className="capitalize">{d.status}</span>
                        </span>
                      </td>
                      <td className="py-2.5 px-3 text-[11px] text-[var(--ink-muted)] whitespace-nowrap">
                        {new Date(d.created_at * 1000).toLocaleString('id-ID', {
                          dateStyle: 'short',
                          timeStyle: 'short',
                        })}
                      </td>
                      <td className="py-2.5 px-3 text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            type="button"
                            onClick={() => handleOpenLogModal(d)}
                            className="px-2 py-1 rounded-[var(--radius-xs)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[11px] font-medium text-[var(--ink)] flex items-center gap-1"
                            title="Lihat output log"
                          >
                            <FileText className="w-3 h-3" />
                            <span>{t('git_deploy.view_logs_btn', 'Log')}</span>
                          </button>
                          {!isCurrent && isSuccess && (
                            <button
                              type="button"
                              onClick={() => setRollbackTarget(d)}
                              className="px-2 py-1 rounded-[var(--radius-xs)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[11px] font-medium text-[var(--primary)] hover:text-[var(--primary-hover)] flex items-center gap-1"
                              title="Rollback langsung ke rilis ini"
                            >
                              <ArrowLeftRight className="w-3 h-3" />
                              <span>{t('git_deploy.rollback_btn', 'Rollback')}</span>
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Log Modal */}
      {logModalDeployment && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs"
          onDismiss={() => setLogModalDeployment(null)}
        >
          <div className="w-full max-w-3xl rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-2xl overflow-hidden flex flex-col max-h-[85vh] dialog-panel">
            <div className="p-4 border-b border-[var(--line)] bg-[var(--raised)] flex items-center justify-between dialog-header">
              <div className="flex items-center gap-2">
                <Terminal className="w-4 h-4 text-[var(--primary)]" />
                <h3 className="text-sm font-semibold text-[var(--ink)]">
                  {t('git_deploy.logs_modal_title', 'Log Eksekusi Deployment')}:{' '}
                  <span className="font-mono text-xs text-[var(--ink-muted)]">{logModalDeployment.release_id}</span>
                </h3>
              </div>
              <button
                type="button"
                onClick={() => setLogModalDeployment(null)}
                className="p-1 rounded-[var(--radius-xs)] hover:bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="p-4 overflow-y-auto bg-neutral-950 text-neutral-100 font-mono text-xs grow min-h-[300px] dialog-body">
              {isLoadingLog ? (
                <div className="flex items-center justify-center h-48 gap-2 text-neutral-400">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  <span>Memuat log deployment...</span>
                </div>
              ) : logModalDeployment.log ? (
                <pre className="whitespace-pre-wrap leading-relaxed select-text">{logModalDeployment.log}</pre>
              ) : logModalDeployment.error_message ? (
                <pre className="text-rose-400 whitespace-pre-wrap leading-relaxed">
                  {logModalDeployment.error_message}
                </pre>
              ) : (
                <div className="text-neutral-500 italic">Belum ada keluaran log yang dicatat untuk rilis ini.</div>
              )}
            </div>

            <div className="p-3 border-t border-[var(--line)] bg-[var(--surface)] flex justify-between items-center text-xs dialog-footer">
              <span className="text-[var(--ink-muted)]">
                Status: <strong className="capitalize">{logModalDeployment.status}</strong>
              </span>
              <button
                type="button"
                onClick={() => setLogModalDeployment(null)}
                className="px-3 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] hover:bg-[var(--surface)] font-medium text-[var(--ink)]"
              >
                Tutup
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* Rollback Confirmation Modal */}
      {rollbackTarget && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs"
          onDismiss={() => setRollbackTarget(null)}
          dismissDisabled={isRollingBack}
        >
          <div className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-2xl p-5 space-y-4 dialog-panel">
            <div className="flex items-center gap-3 text-[var(--primary)] dialog-header">
              <div className="p-2.5 rounded-full bg-[var(--primary)]/10">
                <ArrowLeftRight className="w-5 h-5" />
              </div>
              <h3 className="text-base font-semibold text-[var(--ink)]">
                {t('git_deploy.rollback_confirm_title', 'Konfirmasi Rollback')}
              </h3>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-xs text-[var(--ink-muted)] leading-relaxed">
                {t(
                  'git_deploy.rollback_confirm_desc',
                  'Apakah Anda yakin ingin melakukan rollback ke rilis {{release_id}}? Symlink aktif akan langsung dialihkan ke versi tersebut.',
                  { release_id: rollbackTarget.release_id },
                )}
              </p>
            </div>
            <div className="flex justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setRollbackTarget(null)}
                disabled={isRollingBack}
                className="px-3 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-xs font-semibold text-[var(--ink)]"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={handleConfirmRollback}
                disabled={isRollingBack}
                className="px-4 py-1.5 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] text-xs font-semibold flex items-center gap-2"
              >
                {isRollingBack && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                <span>Rollback Sekarang</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
