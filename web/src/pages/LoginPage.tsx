import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Shield,
  KeyRound,
  Lock,
  User as UserIcon,
  AlertCircle,
  RefreshCw,
  Globe,
  FolderTree,
  Activity,
} from "lucide-react";
import { useAuth } from "../context/AuthContext";
import { Brand } from "../components/ui/Brand";

export interface LoginPageProps {
  onOpenWizard?: () => void;
}

export const LoginPage: React.FC<LoginPageProps> = ({ onOpenWizard }) => {
  const { t } = useTranslation();
  const { login } = useAuth();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [twoFactorCode, setTwoFactorCode] = useState("");
  const [is2FAMode, setIs2FAMode] = useState(false);
  const [isRecoveryMode, setIsRecoveryMode] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setLoading(true);

    try {
      const code = is2FAMode ? twoFactorCode.trim() : undefined;
      const res = await login(username.trim(), password, code);

      if (res.requires_2fa) {
        setIs2FAMode(true);
        setTwoFactorCode("");
        setError(null);
      }
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Terjadi kesalahan saat masuk";
      setError(message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="auth-page">
      <aside className="auth-introduction">
        <Brand />
        <div className="auth-intro-copy">
          <h1>{t("design.loginHeading")}</h1>
          <p>{t("design.loginDescription")}</p>
          <div className="auth-capabilities">
            <div className="auth-capability">
              <Globe size={19} strokeWidth={1.6} />
              <span>{t("design.loginSites")}</span>
            </div>
            <div className="auth-capability">
              <FolderTree size={19} strokeWidth={1.6} />
              <span>{t("design.loginFiles")}</span>
            </div>
            <div className="auth-capability">
              <Activity size={19} strokeWidth={1.6} />
              <span>{t("design.loginMonitoring")}</span>
            </div>
          </div>
        </div>
        <footer>{t("design.hostingWorkspace")}</footer>
      </aside>
      <main className="auth-form-panel">
        <div className="auth-form-container">
          <Brand />
          <div className="mb-6">
            <h2 className="text-base font-semibold text-[var(--ink)]">
              {is2FAMode ? t("auth.twoFactorTitle") : t("auth.loginTitle")}
            </h2>
            <p className="text-xs text-[var(--ink-muted)] mt-1">
              {is2FAMode
                ? t("auth.twoFactorSubtitle")
                : t("auth.loginSubtitle")}
            </p>
          </div>

          {error && (
            <div
              role="alert"
              className="mb-5 p-3 rounded-[var(--radius-sm)] bg-[var(--danger)]/10 border border-[var(--danger)]/30 flex items-start gap-2.5 text-xs text-[var(--danger)]"
            >
              <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
              <span className="leading-tight">{error}</span>
            </div>
          )}

          <form onSubmit={handleSubmit} className="space-y-4">
            {!is2FAMode ? (
              <>
                <div className="space-y-1.5">
                  <label
                    htmlFor="login-username"
                    className="block text-xs font-medium text-[var(--ink)]"
                  >
                    {t("auth.username")}
                  </label>
                  <div className="relative">
                    <UserIcon className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                    <input
                      id="login-username"
                      autoComplete="username"
                      type="text"
                      required
                      autoFocus
                      value={username}
                      onChange={(e) => setUsername(e.target.value)}
                      placeholder="admin atau nama pengguna"
                      className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] pl-9 pr-3 py-2 text-xs text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:border-[var(--primary)] transition-colors"
                    />
                  </div>
                </div>

                <div className="space-y-1.5">
                  <label
                    htmlFor="login-password"
                    className="block text-xs font-medium text-[var(--ink)]"
                  >
                    {t("auth.password")}
                  </label>
                  <div className="relative">
                    <Lock className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                    <input
                      id="login-password"
                      autoComplete="current-password"
                      type="password"
                      required
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder="••••••••••••"
                      className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] pl-9 pr-3 py-2 text-xs text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:border-[var(--primary)] transition-colors"
                    />
                  </div>
                </div>
              </>
            ) : (
              <div className="space-y-1.5">
                <label
                  htmlFor="login-code"
                  className="block text-xs font-medium text-[var(--ink)]"
                >
                  {isRecoveryMode
                    ? t("auth.recoveryCode")
                    : t("auth.twoFactorCode")}
                </label>
                <div className="relative">
                  <Shield className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--primary)]" />
                  <input
                    id="login-code"
                    autoComplete="one-time-code"
                    inputMode={isRecoveryMode ? "text" : "numeric"}
                    type="text"
                    required
                    autoFocus
                    maxLength={isRecoveryMode ? 8 : 6}
                    value={twoFactorCode}
                    onChange={(e) => setTwoFactorCode(e.target.value)}
                    placeholder={isRecoveryMode ? "A1B2C3D4" : "123456"}
                    className="w-full bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] pl-9 pr-3 py-2 text-sm font-mono tracking-widest text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:border-[var(--primary)] transition-colors"
                  />
                </div>

                <div className="pt-1 text-right">
                  <button
                    type="button"
                    onClick={() => {
                      setIsRecoveryMode(!isRecoveryMode);
                      setTwoFactorCode("");
                      setError(null);
                    }}
                    className="text-[11px] text-[var(--primary)] hover:underline"
                  >
                    {isRecoveryMode
                      ? t("auth.useTotpCode")
                      : t("auth.useRecoveryCode")}
                  </button>
                </div>
              </div>
            )}

            <button
              type="submit"
              disabled={loading}
              className="w-full mt-2 py-2 px-4 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] text-xs font-semibold flex items-center justify-center gap-2 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {loading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : is2FAMode ? (
                <>
                  <span>{t("auth.verifyButton")}</span>
                </>
              ) : (
                <span>{t("auth.loginButton")}</span>
              )}
            </button>

            {is2FAMode && (
              <button
                type="button"
                onClick={() => {
                  setIs2FAMode(false);
                  setTwoFactorCode("");
                  setError(null);
                }}
                className="w-full text-center text-xs text-[var(--ink-muted)] hover:text-[var(--ink)] py-1"
              >
                Kembali ke login kata sandi
              </button>
            )}
          </form>

          {onOpenWizard && !is2FAMode && (
            <div className="mt-6 pt-4 border-t border-[var(--line)] text-center">
              <button
                onClick={onOpenWizard}
                className="text-xs text-[var(--primary)] hover:underline flex items-center justify-center gap-1.5 mx-auto"
              >
                <KeyRound className="w-3.5 h-3.5" />
                <span>Bantuan akses admin</span>
              </button>
            </div>
          )}
          <p className="auth-note">{t("design.loginAccessNote")}</p>
        </div>
      </main>
    </div>
  );
};
