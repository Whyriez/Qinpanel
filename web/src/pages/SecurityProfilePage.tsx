import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Shield,
  Lock,
  User as UserIcon,
  Copy,
  Check,
  Download,
  AlertCircle,
  RefreshCw,
  Smartphone,
  ShieldAlert,
} from 'lucide-react';
import { useAuth } from '../context/AuthContext';
import {
  changePassword,
  setup2FA,
  enable2FA,
  disable2FA,
  fetch2FAStatus,
} from '../api/client';
import { StatusPill } from '../components/ui/StatusPill';
import { SshKeyManager } from '../components/ssh/SshKeyManager';

export const SecurityProfilePage: React.FC = () => {
  const { t } = useTranslation();
  const { user, refreshUser } = useAuth();

  // Change Password State
  const [loginUsername, setLoginUsername] = useState(user?.username || '');
  useEffect(() => { setLoginUsername(user?.username || ''); }, [user?.username]);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [passwordLoading, setPasswordLoading] = useState(false);
  const [passwordMessage, setPasswordMessage] = useState<{
    text: string;
    isError: boolean;
  } | null>(null);

  // 2FA Setup State
  const [totpEnabled, setTotpEnabled] = useState(user?.totp_enabled || false);
  const [remainingCodes, setRemainingCodes] = useState<number>(0);
  const [statusLoading, setStatusLoading] = useState(true);
  const [statusError, setStatusError] = useState('');
  const [isSettingUp2FA, setIsSettingUp2FA] = useState(false);
  const [qrBase64, setQrBase64] = useState<string | null>(null);
  const [secretKey, setSecretKey] = useState<string | null>(null);
  const [verifyCode, setVerifyCode] = useState('');
  const [setupLoading, setSetupLoading] = useState(false);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);
  const [copiedKey, setCopiedKey] = useState(false);
  const [copiedCodes, setCopiedCodes] = useState(false);

  // 2FA Disable State
  const [isDisabling2FA, setIsDisabling2FA] = useState(false);
  const [disableCode, setDisableCode] = useState('');
  const [disableLoading, setDisableLoading] = useState(false);
  const [disableError, setDisableError] = useState<string | null>(null);

  const load2FAStatus = useCallback(async () => {
    setStatusLoading(true);
    setStatusError('');
    try {
      const res = await fetch2FAStatus();
      setTotpEnabled(res.enabled);
      setRemainingCodes(res.remaining_recovery_codes || 0);
    } catch {
      setStatusError('Status autentikasi dua faktor belum dapat dimuat.');
    } finally {
      setStatusLoading(false);
    }
  }, []);

  useEffect(() => {
    load2FAStatus();
  }, [load2FAStatus]);

  const handleChangePassword = async (e: React.FormEvent) => {
    e.preventDefault();
    if (passwordLoading) return;
    setPasswordMessage(null);

    if (newPassword !== confirmPassword) {
      setPasswordMessage({
        text: 'Konfirmasi kata sandi tidak cocok.',
        isError: true,
      });
      return;
    }
    if (newPassword && newPassword.length < 8) {
      setPasswordMessage({
        text: 'Kata sandi baru minimal 8 karakter.',
        isError: true,
      });
      return;
    }

    setPasswordLoading(true);
    try {
      await changePassword(currentPassword, newPassword || undefined, user?.role === 'admin' ? loginUsername.trim() : undefined);
      await refreshUser();
      setPasswordMessage({
        text: 'Kredensial diperbarui. Sesi lain dan API token akun telah dicabut.',
        isError: false,
      });
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
    } catch (err: unknown) {
      setPasswordMessage({
        text: err instanceof Error ? err.message : 'Gagal memperbarui kata sandi.',
        isError: true,
      });
    } finally {
      setPasswordLoading(false);
    }
  };

  const handleStartSetup2FA = async () => {
    setSetupLoading(true);
    setSetupError(null);
    try {
      const res = await setup2FA();
      setQrBase64(res.qr_code_base64);
      setSecretKey(res.secret);
      setIsSettingUp2FA(true);
    } catch (err: unknown) {
      setSetupError(
        err instanceof Error ? err.message : 'Gagal menginisiasi setup 2FA.'
      );
    } finally {
      setSetupLoading(false);
    }
  };

  const handleVerifyAndEnable2FA = async (e: React.FormEvent) => {
    e.preventDefault();
    setSetupLoading(true);
    setSetupError(null);

    try {
      const res = await enable2FA(verifyCode.trim());
      setRecoveryCodes(res.recovery_codes);
      setTotpEnabled(true);
      await refreshUser();
      await load2FAStatus();
    } catch (err: unknown) {
      setSetupError(
        err instanceof Error ? err.message : 'Kode verifikasi tidak valid.'
      );
    } finally {
      setSetupLoading(false);
    }
  };

  const handleDisable2FA = async (e: React.FormEvent) => {
    e.preventDefault();
    setDisableLoading(true);
    setDisableError(null);

    try {
      await disable2FA(disableCode.trim());
      setTotpEnabled(false);
      setIsDisabling2FA(false);
      setDisableCode('');
      await refreshUser();
      await load2FAStatus();
    } catch (err: unknown) {
      setDisableError(
        err instanceof Error ? err.message : 'Gagal menonaktifkan 2FA.'
      );
    } finally {
      setDisableLoading(false);
    }
  };

  const handleCopyCodes = () => {
    if (recoveryCodes) {
      navigator.clipboard.writeText(recoveryCodes.join('\n'));
      setCopiedCodes(true);
      setTimeout(() => setCopiedCodes(false), 2000);
    }
  };

  const handleDownloadCodes = () => {
    if (recoveryCodes) {
      const element = document.createElement('a');
      const file = new Blob([recoveryCodes.join('\n')], { type: 'text/plain' });
      element.href = URL.createObjectURL(file);
      element.download = `qonpanel-recovery-codes-${user?.username}.txt`;
      document.body.appendChild(element);
      element.click();
      document.body.removeChild(element);
    }
  };

  return (
    <div className="security-profile">
      {/* Header */}
      <header className="access-page-heading">
        <h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">
          {t('profile.title')}
        </h1>
        <p className="text-xs text-[var(--ink-muted)] mt-1">
          {t('profile.subtitle')}
        </p>
      </header>

      <div className="profile-overview">
        {/* Profile Details Card */}
        <section className="profile-summary" aria-label={t('profile.myProfile')}>
          <div className="flex items-center gap-2 border-b border-[var(--line)] pb-3">
            <UserIcon className="w-4 h-4 text-[var(--primary)]" />
            <h2 className="font-semibold text-xs text-[var(--ink)]">
              {t('profile.myProfile')}
            </h2>
          </div>

          <div className="space-y-3 text-xs">
            <div>
              <span className="text-[var(--ink-muted)] block mb-0.5">
                {t('auth.username')}
              </span>
              <span className="font-semibold text-[var(--ink)]">
                {user?.username}
              </span>
            </div>

            <div>
              <span className="text-[var(--ink-muted)] block mb-0.5">
                {t('profile.role')}
              </span>
              <StatusPill
                label={user?.role === 'admin' ? 'Administrator' : 'Pengguna'}
                variant={user?.role === 'admin' ? 'info' : 'neutral'}
              />
            </div>

            <div>
              <span className="text-[var(--ink-muted)] block mb-0.5">
                {t('profile.email')}
              </span>
              <span className="text-[var(--ink)]">
                {user?.email || 'Belum diatur'}
              </span>
            </div>
          </div>
        </section>

        {/* Change Password Card */}
        <section className="profile-section profile-credentials" aria-labelledby="profile-credentials-title">
          <div className="flex items-center gap-2 border-b border-[var(--line)] pb-3">
            <Lock className="w-4 h-4 text-[var(--primary)]" />
            <h2 id="profile-credentials-title" className="font-semibold text-xs text-[var(--ink)]">
              {user?.role === 'admin' ? 'Username dan kata sandi' : t('profile.changePassword')}
            </h2>
          </div>

          {passwordMessage && (
            <div
              role={passwordMessage.isError ? 'alert' : 'status'}
              className={`p-2.5 rounded-[var(--radius-sm)] text-xs flex items-center gap-2 ${
                passwordMessage.isError
                  ? 'bg-[var(--danger)]/10 text-[var(--danger)] border border-[var(--danger)]/20'
                  : 'bg-[var(--success)]/10 text-[var(--success)] border border-[var(--success)]/20'
              }`}
            >
              <AlertCircle className="w-3.5 h-3.5 shrink-0" />
              <span>{passwordMessage.text}</span>
            </div>
          )}

          <p className="text-xs text-[var(--ink-muted)]">Konfirmasi dengan kata sandi saat ini. Perubahan mencabut sesi lain dan API token akun.</p>
          <form onSubmit={handleChangePassword} className="space-y-3">
            {user?.role === 'admin' && <div className="space-y-1">
              <label htmlFor="account-username" className="block text-[11px] font-medium text-[var(--ink)]">Username login</label>
              <input id="account-username" autoComplete="username" required minLength={3} maxLength={32} pattern="([A-Za-z0-9_]|-)+"
                value={loginUsername} onChange={e => setLoginUsername(e.target.value)} disabled={passwordLoading}
                aria-describedby="account-username-hint"
                className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]" />
              <p id="account-username-hint" className="text-xs text-[var(--ink-muted)]">3-32 karakter: huruf, angka, garis bawah, atau tanda hubung.</p>
            </div>}
            <div className="space-y-1">
              <label htmlFor="account-current-password" className="block text-[11px] font-medium text-[var(--ink)]">
                {t('profile.currentPassword')}
              </label>
              <input
                id="account-current-password"
                type="password"
                autoComplete="current-password"
                maxLength={4096}
                disabled={passwordLoading}
                required
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="account-new-password" className="block text-[11px] font-medium text-[var(--ink)]">
                {t('profile.newPassword')}
              </label>
              <input
                id="account-new-password"
                type="password"
                autoComplete="new-password"
                maxLength={4096}
                disabled={passwordLoading}
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                aria-describedby="account-password-hint"
                className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              />
              <p id="account-password-hint" className="profile-field-hint">{user?.role === 'admin' ? 'Minimal 8 karakter; kosongkan jika hanya ubah username.' : 'Gunakan kata sandi baru dengan minimal 8 karakter.'}</p>
            </div>

            <div className="space-y-1">
              <label htmlFor="account-confirm-password" className="block text-[11px] font-medium text-[var(--ink)]">
                {t('profile.confirmPassword')}
              </label>
              <input
                id="account-confirm-password"
                type="password"
                autoComplete="new-password"
                maxLength={4096}
                disabled={passwordLoading}
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              />
            </div>

            <button
              type="submit"
              disabled={passwordLoading}
              className="py-1.5 px-3 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] text-xs font-semibold flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
            >
              {passwordLoading && <RefreshCw className="w-3 h-3 animate-spin" />}
              <span>{t('common.save')}</span>
            </button>
          </form>
        </section>
      </div>

      {/* 2FA Card */}
      <section className="profile-section profile-two-factor" aria-labelledby="profile-two-factor-title">
        <div className="flex items-center justify-between border-b border-[var(--line)] pb-3">
          <div className="flex items-center gap-2">
            <Shield className="w-4 h-4 text-[var(--primary)]" />
            <h2 id="profile-two-factor-title" className="font-semibold text-xs text-[var(--ink)]">
              {t('profile.twoFactor')}
            </h2>
          </div>
          <StatusPill
            label={statusLoading ? 'Memuat status' : statusError ? 'Belum terverifikasi' : totpEnabled ? 'Aktif' : 'Belum aktif'}
            variant={!statusLoading && !statusError && totpEnabled ? 'success' : 'neutral'}
          />
        </div>

        {statusError ? <div role="alert" className="profile-status-error"><p>{statusError}</p><button type="button" className="access-button" onClick={() => void load2FAStatus()}><RefreshCw size={15} aria-hidden="true" />Coba lagi</button></div> : <p role={statusLoading ? 'status' : undefined} className="text-xs text-[var(--ink-muted)]">
          {statusLoading ? 'Memeriksa perlindungan akun Anda…' :
          totpEnabled
            ? `${t('profile.twoFactorEnabled')} (${remainingCodes} kode pemulihan tersisa).`
            : t('profile.twoFactorDisabled')}
        </p>}
        {setupError && !isSettingUp2FA && <p role="alert" className="text-sm text-[var(--danger)]">{setupError}</p>}

        {!statusLoading && !statusError && !totpEnabled && !isSettingUp2FA && (
          <button
            onClick={handleStartSetup2FA}
            disabled={setupLoading}
            className="py-1.5 px-4 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] text-xs font-semibold flex items-center gap-2 cursor-pointer"
          >
            {setupLoading ? (
              <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Smartphone className="w-3.5 h-3.5" />
            )}
            <span>{t('profile.setupTwoFactor')}</span>
          </button>
        )}

        {!statusLoading && !statusError && totpEnabled && !isDisabling2FA && (
          <button
            onClick={() => setIsDisabling2FA(true)}
            className="py-1.5 px-3 rounded-[var(--radius-sm)] border border-[var(--danger)] text-[var(--danger)] hover:bg-[var(--danger)]/10 text-xs font-medium cursor-pointer"
          >
            {t('profile.disableTwoFactor')}
          </button>
        )}

        {/* 2FA Disable Form */}
        {isDisabling2FA && (
          <form
            onSubmit={handleDisable2FA}
            className="p-4 border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--raised)] space-y-3"
          >
            <h3 className="font-semibold text-xs text-[var(--danger)]">
              Konfirmasi Nonaktifkan 2FA
            </h3>
            <p className="text-xs text-[var(--ink-muted)]">
              Masukkan 6 digit kode dari aplikasi autentikator atau kode pemulihan untuk mengonfirmasi penonaktifan.
            </p>
            {disableError && (
              <div role="alert" className="p-2 rounded bg-[var(--danger)]/10 text-[var(--danger)] text-xs">
                {disableError}
              </div>
            )}
            <div className="max-w-xs">
              <label htmlFor="account-disable-code" className="block mb-2 text-xs font-medium">Kode autentikasi atau pemulihan</label>
              <input
                id="account-disable-code"
                type="text"
                autoComplete="one-time-code"
                required
                autoFocus
                value={disableCode}
                onChange={(e) => setDisableCode(e.target.value)}
                placeholder="123456"
                className="w-full bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs font-mono text-[var(--ink)]"
              />
            </div>
            <div className="flex items-center gap-2">
              <button
                type="submit"
                disabled={disableLoading}
                className="py-1.5 px-3 rounded-[var(--radius-sm)] bg-[var(--danger)] text-white text-xs font-semibold cursor-pointer"
              >
                {disableLoading ? 'Memproses...' : 'Konfirmasi Nonaktifkan'}
              </button>
              <button
                type="button"
                onClick={() => {
                  setIsDisabling2FA(false);
                  setDisableError(null);
                }}
                className="py-1.5 px-3 text-xs text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                Batal
              </button>
            </div>
          </form>
        )}

        {/* 2FA Setup Flow */}
        {isSettingUp2FA && !recoveryCodes && (
          <div className="p-4 border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--raised)] space-y-4">
            <h3 className="font-semibold text-xs text-[var(--ink)]">
              Setup Autentikasi Dua Faktor
            </h3>
            <p className="text-xs text-[var(--ink-muted)]">
              {t('profile.scanQr')}
            </p>

            {setupError && (
              <div role="alert" className="p-2 rounded bg-[var(--danger)]/10 text-[var(--danger)] text-xs">
                {setupError}
              </div>
            )}

            <div className="flex flex-col sm:flex-row items-center gap-6 p-4 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)]">
              {qrBase64 && (
                <div className="p-2 bg-white rounded-[var(--radius-sm)] border border-[var(--line)]">
                  <img
                    src={`data:image/png;base64,${qrBase64}`}
                    alt="2FA QR Code"
                    className="w-36 h-36"
                  />
                </div>
              )}

              <div className="space-y-2 flex-1 text-xs">
                <span className="text-[var(--ink-muted)] block">
                  {t('profile.manualKey')}
                </span>
                <div className="flex items-center gap-2">
                  <code className="p-2 font-mono text-xs bg-[var(--raised)] border border-[var(--line)] rounded select-all break-all text-[var(--ink)]">
                    {secretKey}
                  </code>
                  <button
                    type="button"
                    onClick={() => {
                      if (secretKey) {
                        navigator.clipboard.writeText(secretKey);
                        setCopiedKey(true);
                        setTimeout(() => setCopiedKey(false), 2000);
                      }
                    }}
                    className="p-1.5 rounded border border-[var(--line)] hover:bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)]"
                    title="Salin secret"
                  >
                    {copiedKey ? (
                      <Check className="w-3.5 h-3.5 text-[var(--success)]" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                  </button>
                </div>
              </div>
            </div>

            <form onSubmit={handleVerifyAndEnable2FA} className="space-y-3 pt-2">
              <label htmlFor="account-verify-code" className="block text-xs font-medium text-[var(--ink)]">
                Masukkan 6 digit kode dari aplikasi:
              </label>
              <div className="profile-code-entry">
                <input
                  id="account-verify-code"
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  pattern="[0-9]{6}"
                  required
                  maxLength={6}
                  value={verifyCode}
                  onChange={(e) => setVerifyCode(e.target.value)}
                  placeholder="000000"
                  className="w-full bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] px-3 py-1.5 text-xs font-mono tracking-widest text-[var(--ink)]"
                />
                <button
                  type="submit"
                  disabled={setupLoading || verifyCode.length < 6}
                  className="py-1.5 px-4 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white text-xs font-semibold disabled:opacity-50 cursor-pointer whitespace-nowrap"
                >
                  {setupLoading ? 'Memeriksa...' : t('profile.verifyAndEnable')}
                </button>
              </div>
            </form>
          </div>
        )}

        {/* Recovery Codes Display Modal / View */}
        {recoveryCodes && (
          <div className="p-5 border-2 border-[var(--accent)]/50 rounded-[var(--radius-md)] bg-[var(--surface)] space-y-4">
            <div className="flex items-center gap-2 text-[var(--accent)]">
              <ShieldAlert className="w-5 h-5" />
              <h3 className="font-bold text-sm text-[var(--ink)]">
                {t('profile.recoveryCodesTitle')}
              </h3>
            </div>
            <p className="text-xs text-[var(--ink-muted)]">
              {t('profile.recoveryCodesDesc')}
            </p>

            <div className="profile-recovery-codes grid grid-cols-2 sm:grid-cols-4 gap-2.5 p-4 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] font-mono text-xs font-bold text-center text-[var(--ink)]">
              {recoveryCodes.map((code, idx) => (
                <div key={idx} className="p-1.5 bg-[var(--surface)] rounded border border-[var(--line)]">
                  {code}
                </div>
              ))}
            </div>

            <div className="flex flex-wrap items-center gap-3 pt-2">
              <button
                type="button"
                onClick={handleCopyCodes}
                className="py-1.5 px-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-xs text-[var(--ink)] hover:bg-[var(--surface)] flex items-center gap-1.5 cursor-pointer"
              >
                {copiedCodes ? <Check className="w-3.5 h-3.5 text-[var(--success)]" /> : <Copy className="w-3.5 h-3.5" />}
                <span>{copiedCodes ? 'Tersalin!' : t('profile.copyCodes')}</span>
              </button>

              <button
                type="button"
                onClick={handleDownloadCodes}
                className="py-1.5 px-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-xs text-[var(--ink)] hover:bg-[var(--surface)] flex items-center gap-1.5 cursor-pointer"
              >
                <Download className="w-3.5 h-3.5" />
                <span>{t('profile.downloadCodes')}</span>
              </button>

              <button
                type="button"
                onClick={() => {
                  setRecoveryCodes(null);
                  setIsSettingUp2FA(false);
                }}
                className="ml-auto py-1.5 px-4 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white text-xs font-semibold cursor-pointer"
              >
                {t('profile.done')}
              </button>
            </div>
          </div>
        )}
      </section>

      {/* SSH Keys Management */}
      <div className="profile-ssh">
        <SshKeyManager />
      </div>
    </div>
  );
};
