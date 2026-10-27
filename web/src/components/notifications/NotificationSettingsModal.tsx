import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  X,
  Mail,
  Send,
  Sliders,
  Check,
  AlertTriangle,
  RefreshCw,
  Lock,
  Server,
  User,
} from 'lucide-react';
import {
  fetchNotificationSettings,
  updateNotificationSettings,
  testSmtpConnection,
} from '../../api/client';
import { NotificationSettings } from '../../types/api';

interface NotificationSettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSaved?: () => void;
}

export const NotificationSettingsModal: React.FC<NotificationSettingsModalProps> = ({
  isOpen,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<'smtp' | 'thresholds'>('smtp');
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isTestingSmtp, setIsTestingSmtp] = useState(false);
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(
    null
  );
  const [testEmailAddress, setTestEmailAddress] = useState('');

  // Form State
  const [smtpEnabled, setSmtpEnabled] = useState(false);
  const [smtpHost, setSmtpHost] = useState('');
  const [smtpPort, setSmtpPort] = useState(587);
  const [smtpUsername, setSmtpUsername] = useState('');
  const [smtpPassword, setSmtpPassword] = useState('');
  const [hasExistingPassword, setHasExistingPassword] = useState(false);
  const [smtpEncryption, setSmtpEncryption] = useState('starttls');
  const [fromAddress, setFromAddress] = useState('');
  const [fromName, setFromName] = useState('Qinpanel Alerts');
  const [adminEmail, setAdminEmail] = useState('');

  const [sslWarningDays, setSslWarningDays] = useState(14);
  const [quotaWarningPercent, setQuotaWarningPercent] = useState(90);
  const [notifySslExpiring, setNotifySslExpiring] = useState(true);
  const [notifyQuotaWarning, setNotifyQuotaWarning] = useState(true);
  const [notifyJobFailed, setNotifyJobFailed] = useState(true);
  const [notifyDeployFailed, setNotifyDeployFailed] = useState(true);

  useEffect(() => {
    if (isOpen) {
      loadSettings();
    } else {
      setFeedback(null);
    }
  }, [isOpen]);

  const loadSettings = async () => {
    setIsLoading(true);
    setFeedback(null);
    try {
      const data: NotificationSettings = await fetchNotificationSettings();
      setSmtpEnabled(data.smtp_enabled);
      setSmtpHost(data.smtp_host || '');
      setSmtpPort(data.smtp_port || 587);
      setSmtpUsername(data.smtp_username || '');
      setHasExistingPassword(data.smtp_has_password);
      setSmtpPassword(data.smtp_has_password ? '********' : '');
      setSmtpEncryption(data.smtp_encryption || 'starttls');
      setFromAddress(data.from_address || '');
      setFromName(data.from_name || 'Qinpanel Alerts');
      setAdminEmail(data.admin_email || '');
      setTestEmailAddress(data.admin_email || '');

      setSslWarningDays(data.ssl_warning_days || 14);
      setQuotaWarningPercent(data.quota_warning_percent || 90);
      setNotifySslExpiring(data.notify_ssl_expiring);
      setNotifyQuotaWarning(data.notify_quota_warning);
      setNotifyJobFailed(data.notify_job_failed);
      setNotifyDeployFailed(data.notify_deploy_failed);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat pengaturan notifikasi';
      setFeedback({ type: 'error', message: msg });
    } finally {
      setIsLoading(false);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSaving(true);
    setFeedback(null);

    try {
      const payload: Parameters<typeof updateNotificationSettings>[0] = {
        smtp_enabled: smtpEnabled,
        smtp_host: smtpHost.trim(),
        smtp_port: smtpPort,
        smtp_username: smtpUsername.trim() || null,
        smtp_encryption: smtpEncryption,
        from_address: fromAddress.trim(),
        from_name: fromName.trim(),
        admin_email: adminEmail.trim(),
        ssl_warning_days: sslWarningDays,
        quota_warning_percent: quotaWarningPercent,
        notify_ssl_expiring: notifySslExpiring,
        notify_quota_warning: notifyQuotaWarning,
        notify_job_failed: notifyJobFailed,
        notify_deploy_failed: notifyDeployFailed,
      };

      // Only include password if user typed a new password
      if (smtpPassword && smtpPassword !== '********') {
        payload.smtp_password = smtpPassword;
      }

      await updateNotificationSettings(payload);
      setFeedback({
        type: 'success',
        message: t('notifications.settingsModal.saveSuccess'),
      });
      if (onSaved) onSaved();
      setTimeout(() => {
        onClose();
      }, 1000);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal menyimpan pengaturan';
      setFeedback({ type: 'error', message: msg });
    } finally {
      setIsSaving(false);
    }
  };

  const handleTestSmtp = async () => {
    if (!smtpHost) {
      setFeedback({
        type: 'error',
        message: 'Host SMTP harus diisi sebelum mengirim email uji coba',
      });
      return;
    }
    setIsTestingSmtp(true);
    setFeedback(null);

    try {
      const payload = {
        host: smtpHost.trim(),
        port: smtpPort,
        username: smtpUsername.trim() || undefined,
        password: smtpPassword && smtpPassword !== '********' ? smtpPassword : '********',
        encryption: smtpEncryption,
        to_address: testEmailAddress.trim() || adminEmail.trim() || undefined,
      };

      const res = await testSmtpConnection(payload);
      if (res.success) {
        setFeedback({
          type: 'success',
          message: res.message || t('notifications.settingsModal.testSuccess'),
        });
      } else {
        setFeedback({
          type: 'error',
          message: res.error || t('notifications.settingsModal.testFailed'),
        });
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t('notifications.settingsModal.testFailed');
      setFeedback({ type: 'error', message: msg });
    } finally {
      setIsTestingSmtp(false);
    }
  };

  if (!isOpen) return null;

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-200"
      role="dialog"
      aria-modal="true"
      onDismiss={onClose}
    >
      <div className="relative w-full max-w-2xl bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-2xl flex flex-col max-h-[90vh] overflow-hidden dialog-panel">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--line)] dialog-header">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-[var(--radius-md)] bg-[var(--primary)]/10 text-[var(--primary)] flex items-center justify-center">
              <Mail className="w-5 h-5" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-[var(--ink)]">
                {t('notifications.settingsModal.title')}
              </h2>
              <p className="text-xs text-[var(--ink-muted)]">{t('notifications.settingsModal.desc')}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Navigation Tabs */}
        <div className="flex border-b border-[var(--line)] px-6 bg-[var(--surface-sunken)]/40">
          <button
            type="button"
            onClick={() => setActiveTab('smtp')}
            className={`flex items-center gap-2 py-3 px-4 text-xs font-medium border-b-2 transition-colors cursor-pointer ${
              activeTab === 'smtp'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Server className="w-3.5 h-3.5" />
            {t('notifications.settingsModal.tabSmtp')}
          </button>
          <button
            type="button"
            onClick={() => setActiveTab('thresholds')}
            className={`flex items-center gap-2 py-3 px-4 text-xs font-medium border-b-2 transition-colors cursor-pointer ${
              activeTab === 'thresholds'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Sliders className="w-3.5 h-3.5" />
            {t('notifications.settingsModal.tabThresholds')}
          </button>
        </div>

        {/* Modal Body */}
        <div className="flex-1 overflow-y-auto p-6 space-y-4 dialog-body">
          {feedback && (
            <div
              className={`p-3 rounded-[var(--radius-md)] text-xs flex items-center gap-2 ${
                feedback.type === 'success'
                  ? 'bg-[var(--success)]/10 text-[var(--success)] border border-[var(--success)]/30'
                  : 'bg-[var(--danger)]/10 text-[var(--danger)] border border-[var(--danger)]/30'
              }`}
            >
              {feedback.type === 'success' ? (
                <Check className="w-4 h-4 shrink-0" />
              ) : (
                <AlertTriangle className="w-4 h-4 shrink-0" />
              )}
              <span>{feedback.message}</span>
            </div>
          )}

          {isLoading ? (
            <div className="py-12 flex flex-col items-center justify-center gap-3 text-[var(--ink-muted)]">
              <RefreshCw className="w-6 h-6 animate-spin text-[var(--primary)]" />
              <span className="text-xs">{t('common.loading')}</span>
            </div>
          ) : (
            <form id="notification-settings-form" onSubmit={handleSave} className="space-y-4">
              {activeTab === 'smtp' && (
                <div className="space-y-4">
                  {/* Enable SMTP Switch */}
                  <div className="flex items-center justify-between p-3.5 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)]">
                    <div>
                      <label
                        htmlFor="smtp-enabled-toggle"
                        className="text-xs font-semibold text-[var(--ink)] cursor-pointer"
                      >
                        {t('notifications.settingsModal.smtpEnabled')}
                      </label>
                      <p className="text-[11px] text-[var(--ink-muted)] mt-0.5">
                        Kirimkan salinan peringatan kritis langsung ke email administrator.
                      </p>
                    </div>
                    <input
                      id="smtp-enabled-toggle"
                      type="checkbox"
                      checked={smtpEnabled}
                      onChange={(e) => setSmtpEnabled(e.target.checked)}
                      className="w-4 h-4 accent-[var(--primary)] cursor-pointer"
                    />
                  </div>

                  {/* SMTP Server Details */}
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                    <div className="md:col-span-2">
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.smtpHost')}
                      </label>
                      <input
                        type="text"
                        value={smtpHost}
                        onChange={(e) => setSmtpHost(e.target.value)}
                        placeholder="smtp.example.com atau mail.domain.tld"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.smtpPort')}
                      </label>
                      <input
                        type="number"
                        value={smtpPort}
                        onChange={(e) => setSmtpPort(parseInt(e.target.value) || 587)}
                        min={1}
                        max={65535}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                    </div>
                  </div>

                  {/* Encryption & Credentials */}
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.encryption')}
                      </label>
                      <select
                        value={smtpEncryption}
                        onChange={(e) => setSmtpEncryption(e.target.value)}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none cursor-pointer"
                      >
                        <option value="starttls">STARTTLS (587)</option>
                        <option value="tls">Implicit SSL/TLS (465)</option>
                        <option value="none">None (25)</option>
                      </select>
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.smtpUser')}
                      </label>
                      <div className="relative">
                        <User className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                        <input
                          type="text"
                          value={smtpUsername}
                          onChange={(e) => setSmtpUsername(e.target.value)}
                          placeholder="user@example.com"
                          className="w-full pl-8 pr-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                        />
                      </div>
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.smtpPassword')}
                      </label>
                      <div className="relative">
                        <Lock className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                        <input
                          type="password"
                          value={smtpPassword}
                          onChange={(e) => setSmtpPassword(e.target.value)}
                          placeholder={
                            hasExistingPassword
                              ? t('notifications.settingsModal.smtpPasswordPlaceholder')
                              : '••••••••'
                          }
                          className="w-full pl-8 pr-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                        />
                      </div>
                    </div>
                  </div>

                  {/* Sender & Recipient Email */}
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.fromAddress')}
                      </label>
                      <input
                        type="email"
                        value={fromAddress}
                        onChange={(e) => setFromAddress(e.target.value)}
                        placeholder="noreply@domain.tld"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.fromName')}
                      </label>
                      <input
                        type="text"
                        value={fromName}
                        onChange={(e) => setFromName(e.target.value)}
                        placeholder="Qinpanel Alerts"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        {t('notifications.settingsModal.adminEmail')}
                      </label>
                      <input
                        type="email"
                        value={adminEmail}
                        onChange={(e) => {
                          setAdminEmail(e.target.value);
                          if (!testEmailAddress) setTestEmailAddress(e.target.value);
                        }}
                        placeholder="admin@domain.tld"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                    </div>
                  </div>

                  {/* Test SMTP Section */}
                  <div className="pt-3 border-t border-[var(--line)]">
                    <label className="block text-xs font-medium text-[var(--ink)] mb-1.5">
                      {t('notifications.settingsModal.testRecipient')}
                    </label>
                    <div className="flex gap-2">
                      <input
                        type="email"
                        value={testEmailAddress}
                        onChange={(e) => setTestEmailAddress(e.target.value)}
                        placeholder="tujuan@example.com"
                        className="flex-1 px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:border-[var(--primary)] focus:outline-none"
                      />
                      <button
                        type="button"
                        onClick={handleTestSmtp}
                        disabled={isTestingSmtp || !smtpHost}
                        className="px-3 py-1.5 text-xs font-medium rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-sunken)] hover:border-[var(--primary)]/50 transition-colors flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
                      >
                        {isTestingSmtp ? (
                          <>
                            <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                            <span>{t('notifications.settingsModal.testingSmtp')}</span>
                          </>
                        ) : (
                          <>
                            <Send className="w-3.5 h-3.5 text-[var(--primary)]" />
                            <span>{t('notifications.settingsModal.testSmtpBtn')}</span>
                          </>
                        )}
                      </button>
                    </div>
                  </div>
                </div>
              )}

              {activeTab === 'thresholds' && (
                <div className="space-y-5">
                  {/* SSL Expiry Days Slider */}
                  <div className="p-4 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)] space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs font-semibold text-[var(--ink)]">
                        {t('notifications.settingsModal.sslThresholdLabel')}
                      </label>
                      <span className="text-xs font-bold font-mono px-2 py-0.5 rounded bg-[var(--primary)]/10 text-[var(--primary)]">
                        {sslWarningDays} hari
                      </span>
                    </div>
                    <p className="text-[11px] text-[var(--ink-muted)]">
                      {t('notifications.settingsModal.sslThresholdDesc')}
                    </p>
                    <input
                      type="range"
                      min={1}
                      max={60}
                      value={sslWarningDays}
                      onChange={(e) => setSslWarningDays(parseInt(e.target.value))}
                      className="w-full accent-[var(--primary)] cursor-pointer"
                    />
                    <div className="flex justify-between text-[10px] text-[var(--ink-muted)]">
                      <span>1 hari</span>
                      <span>14 hari (Default)</span>
                      <span>60 hari</span>
                    </div>
                  </div>

                  {/* Quota Warning Percent Slider */}
                  <div className="p-4 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)] space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs font-semibold text-[var(--ink)]">
                        {t('notifications.settingsModal.quotaThresholdLabel')}
                      </label>
                      <span className="text-xs font-bold font-mono px-2 py-0.5 rounded bg-[var(--accent)]/15 text-[var(--accent)]">
                        {quotaWarningPercent}%
                      </span>
                    </div>
                    <p className="text-[11px] text-[var(--ink-muted)]">
                      {t('notifications.settingsModal.quotaThresholdDesc')}
                    </p>
                    <input
                      type="range"
                      min={50}
                      max={99}
                      value={quotaWarningPercent}
                      onChange={(e) => setQuotaWarningPercent(parseInt(e.target.value))}
                      className="w-full accent-[var(--accent)] cursor-pointer"
                    />
                    <div className="flex justify-between text-[10px] text-[var(--ink-muted)]">
                      <span>50%</span>
                      <span>90% (Default)</span>
                      <span>99%</span>
                    </div>
                  </div>

                  {/* Alert Types Toggles */}
                  <div className="space-y-2">
                    <h4 className="text-xs font-semibold text-[var(--ink)]">
                      {t('notifications.settingsModal.alertTypesTitle')}
                    </h4>
                    <div className="space-y-1.5">
                      <label className="flex items-center justify-between p-2.5 rounded-[var(--radius-sm)] hover:bg-[var(--raised)] cursor-pointer">
                        <span className="text-xs text-[var(--ink)]">
                          {t('notifications.settingsModal.notifySslExpiring')}
                        </span>
                        <input
                          type="checkbox"
                          checked={notifySslExpiring}
                          onChange={(e) => setNotifySslExpiring(e.target.checked)}
                          className="w-4 h-4 accent-[var(--primary)] cursor-pointer"
                        />
                      </label>
                      <label className="flex items-center justify-between p-2.5 rounded-[var(--radius-sm)] hover:bg-[var(--raised)] cursor-pointer">
                        <span className="text-xs text-[var(--ink)]">
                          {t('notifications.settingsModal.notifyQuotaWarning')}
                        </span>
                        <input
                          type="checkbox"
                          checked={notifyQuotaWarning}
                          onChange={(e) => setNotifyQuotaWarning(e.target.checked)}
                          className="w-4 h-4 accent-[var(--primary)] cursor-pointer"
                        />
                      </label>
                      <label className="flex items-center justify-between p-2.5 rounded-[var(--radius-sm)] hover:bg-[var(--raised)] cursor-pointer">
                        <span className="text-xs text-[var(--ink)]">
                          {t('notifications.settingsModal.notifyJobFailed')}
                        </span>
                        <input
                          type="checkbox"
                          checked={notifyJobFailed}
                          onChange={(e) => setNotifyJobFailed(e.target.checked)}
                          className="w-4 h-4 accent-[var(--primary)] cursor-pointer"
                        />
                      </label>
                      <label className="flex items-center justify-between p-2.5 rounded-[var(--radius-sm)] hover:bg-[var(--raised)] cursor-pointer">
                        <span className="text-xs text-[var(--ink)]">
                          {t('notifications.settingsModal.notifyDeployFailed')}
                        </span>
                        <input
                          type="checkbox"
                          checked={notifyDeployFailed}
                          onChange={(e) => setNotifyDeployFailed(e.target.checked)}
                          className="w-4 h-4 accent-[var(--primary)] cursor-pointer"
                        />
                      </label>
                    </div>
                  </div>
                </div>
              )}
            </form>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 px-6 py-3.5 border-t border-[var(--line)] bg-[var(--surface-sunken)]/40 dialog-footer">
          <button
            type="button"
            onClick={onClose}
            className="px-3.5 py-1.5 text-xs font-medium rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors cursor-pointer"
          >
            {t('common.cancel')}
          </button>
          <button
            type="submit"
            form="notification-settings-form"
            disabled={isSaving || isLoading}
            className="px-4 py-1.5 text-xs font-medium rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary)]/90 transition-colors flex items-center gap-1.5 cursor-pointer disabled:opacity-50 shadow-xs"
          >
            {isSaving ? (
              <>
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                <span>{t('common.loading')}</span>
              </>
            ) : (
              <span>{t('common.save')}</span>
            )}
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
};
