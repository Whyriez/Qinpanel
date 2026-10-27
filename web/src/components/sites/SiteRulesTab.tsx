import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import {
  Sliders,
  ArrowRight,
  Shield,
  Globe,
  Lock,
  AlertTriangle,
  Plus,
  Trash2,
  CheckCircle2,
  AlertCircle,
  X,
  Loader2,
  Code2,
  RefreshCw,
  FileText,
  Check,
  Power,
  Filter,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { createSiteRuleId } from '../../lib/siteRuleIds';
import {
  fetchSiteRules,
  updateSiteRules,
  hashBasicAuthPassword,
  validateNginxSnippet,
} from '../../api/client';
import {
  Site,
  SiteRulesConfig,
  StructuredRedirect,
  CustomHeader,
  CorsPreset,
  BasicAuthUser,
  IpAccessRule,
  IpAction,
  CustomErrorPage,
  RedirectMatchType,
  HotlinkAction,
} from '../../types/api';

export interface SiteRulesTabProps {
  site: Site;
  userRole?: string;
  onSiteUpdated?: () => void;
  initialRules?: SiteRulesConfig;
}

type RuleSection =
  | 'redirects'
  | 'headers'
  | 'cors'
  | 'hotlink'
  | 'auth'
  | 'ip'
  | 'maintenance'
  | 'errors'
  | 'snippet';

export const DEFAULT_RULES: SiteRulesConfig = {
  redirects: [],
  custom_headers: [],
  security_headers: {
    hsts_enabled: false,
    hsts_subdomains: false,
    hsts_preload: false,
    x_frame_options: 'SAMEORIGIN',
    x_content_type_options: true,
    referrer_policy: 'strict-origin-when-cross-origin',
    permissions_policy: null,
    csp: null,
  },
  cors: {
    enabled: false,
    preset: 'disabled',
    allowed_origins: [],
    allow_credentials: true,
    allowed_methods: ['GET', 'POST', 'PUT', 'DELETE', 'OPTIONS'],
    allowed_headers: ['Content-Type', 'Authorization', 'X-Requested-With'],
    exposed_headers: [],
    max_age_secs: 86400,
  },
  hotlink_protection: {
    enabled: false,
    allowed_domains: [],
    file_extensions: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'ico', 'mp4'],
    allow_empty_referer: true,
    action: 'deny',
    redirect_url: null,
  },
  basic_auth: {
    enabled: false,
    path: '/',
    realm: 'Area Terproteksi',
    users: [],
  },
  ip_access: {
    enabled: false,
    scope: 'global',
    default_action: 'allow',
    path: null,
    rules: [],
  },
  maintenance: {
    enabled: false,
    bypass_ips: ['127.0.0.1'],
    retry_after_secs: 300,
    custom_message: 'Situs sedang dalam pemeliharaan terjadwal. Silakan kembali beberapa saat lagi.',
  },
  custom_error_pages: [],
  nginx_snippet: {
    enabled: false,
    content: '',
  },
};

export const SiteRulesTab: React.FC<SiteRulesTabProps> = ({ site, userRole, onSiteUpdated, initialRules }) => {
  const { t } = useTranslation();
  const isAdmin = userRole === 'admin';

  const [rules, setRules] = useState<SiteRulesConfig>(initialRules || DEFAULT_RULES);
  const [activeSection, setActiveSection] = useState<RuleSection>('redirects');
  const [isLoading, setIsLoading] = useState(!initialRules);
  const [isSaving, setIsSaving] = useState(false);
  const [hasLoadedRules, setHasLoadedRules] = useState(Boolean(initialRules));
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null);

  // Snippet validation state
  const [snippetTesting, setSnippetTesting] = useState(false);
  const [snippetValidation, setSnippetValidation] = useState<{
    valid: boolean;
    message?: string;
  } | null>(null);

  // Modals / forms
  const [showAddRedirectModal, setShowAddRedirectModal] = useState(false);
  const [redirSource, setRedirSource] = useState('');
  const [redirTarget, setRedirTarget] = useState('');
  const [redirCode, setRedirCode] = useState<number>(301);
  const [redirType, setRedirType] = useState<RedirectMatchType>('prefix');

  const [showAddHeaderModal, setShowAddHeaderModal] = useState(false);
  const [hdrName, setHdrName] = useState('');
  const [hdrValue, setHdrValue] = useState('');
  const [hdrAlways, setHdrAlways] = useState(true);

  const [showAddAuthUserModal, setShowAddAuthUserModal] = useState(false);
  const [authUsername, setAuthUsername] = useState('');
  const [authPassword, setAuthPassword] = useState('');
  const [isHashingPassword, setIsHashingPassword] = useState(false);

  const [showAddIpRuleModal, setShowAddIpRuleModal] = useState(false);
  const [ipCidr, setIpCidr] = useState('');
  const [ipAction, setIpAction] = useState<IpAction>('deny');
  const [ipDesc, setIpDesc] = useState('');

  const [showAddErrorPageModal, setShowAddErrorPageModal] = useState(false);
  const [errorCode, setErrorCode] = useState<number>(404);
  const [errorPath, setErrorPath] = useState('/errors/404.html');

  // Load initial rules
  const loadRules = useCallback(async () => {
    setIsLoading(true);
    setHasLoadedRules(false);
    setFeedback(null);
    try {
      const data = await fetchSiteRules(site.id);
      setRules({
        ...DEFAULT_RULES,
        ...data,
        security_headers: {
          ...DEFAULT_RULES.security_headers,
          ...(data.security_headers || {}),
        },
        cors: {
          ...DEFAULT_RULES.cors,
          ...(data.cors || {}),
        },
        hotlink_protection: {
          ...DEFAULT_RULES.hotlink_protection,
          ...(data.hotlink_protection || {}),
        },
        basic_auth: {
          ...DEFAULT_RULES.basic_auth,
          ...(data.basic_auth || {}),
        },
        ip_access: {
          ...DEFAULT_RULES.ip_access,
          ...(data.ip_access || {}),
        },
        maintenance: {
          ...DEFAULT_RULES.maintenance,
          ...(data.maintenance || {}),
        },
        nginx_snippet: {
          ...DEFAULT_RULES.nginx_snippet,
          ...(data.nginx_snippet || {}),
        },
      });
      setHasLoadedRules(true);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setFeedback({ type: 'error', message: `Gagal memuat aturan situs: ${msg}` });
    } finally {
      setIsLoading(false);
    }
  }, [site.id]);

  useEffect(() => {
    loadRules();
  }, [loadRules]);

  // Save rules
  const handleSave = async () => {
    if (!hasLoadedRules || isLoading || isSaving) return;
    setIsSaving(true);
    setFeedback(null);
    try {
      const updated = await updateSiteRules(site.id, rules);
      setRules(updated);
      setFeedback({
        type: 'success',
        message: t('sites.rules.saved', 'Aturan situs berhasil diterapkan ke Nginx secara atomik.'),
      });
      if (onSiteUpdated) {
        onSiteUpdated();
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setFeedback({
        type: 'error',
        message: `Gagal menyimpan aturan situs: ${msg}`,
      });
    } finally {
      setIsSaving(false);
    }
  };

  // Add redirect
  const handleAddRedirect = (e: React.FormEvent) => {
    e.preventDefault();
    if (!redirSource.trim() || !redirTarget.trim()) return;
    const newRedir: StructuredRedirect = {
      id: createSiteRuleId('redirect'),
      source: redirSource.trim(),
      target: redirTarget.trim(),
      status_code: redirCode,
      match_type: redirType,
      enabled: true,
    };
    setRules((prev) => ({
      ...prev,
      redirects: [...prev.redirects, newRedir],
    }));
    setRedirSource('');
    setRedirTarget('');
    setShowAddRedirectModal(false);
  };

  const handleRemoveRedirect = (index: number) => {
    setRules((prev) => ({
      ...prev,
      redirects: prev.redirects.filter((_, i) => i !== index),
    }));
  };

  // Add custom header
  const handleAddHeader = (e: React.FormEvent) => {
    e.preventDefault();
    if (!hdrName.trim() || !hdrValue.trim()) return;
    const newHdr: CustomHeader = {
      id: createSiteRuleId('header'),
      enabled: true,
      name: hdrName.trim(),
      value: hdrValue.trim(),
      always: hdrAlways,
    };
    setRules((prev) => ({
      ...prev,
      custom_headers: [...prev.custom_headers, newHdr],
    }));
    setHdrName('');
    setHdrValue('');
    setShowAddHeaderModal(false);
  };

  const handleRemoveHeader = (index: number) => {
    setRules((prev) => ({
      ...prev,
      custom_headers: prev.custom_headers.filter((_, i) => i !== index),
    }));
  };

  // Add basic auth user
  const handleAddAuthUser = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!authUsername.trim() || !authPassword) return;
    setIsHashingPassword(true);
    try {
      const resp = await hashBasicAuthPassword(site.id, authPassword);
      const newUser: BasicAuthUser = {
        username: authUsername.trim(),
        password_hash: resp.hash,
      };
      setRules((prev) => ({
        ...prev,
        basic_auth: {
          ...prev.basic_auth,
          users: [...prev.basic_auth.users, newUser],
        },
      }));
      setAuthUsername('');
      setAuthPassword('');
      setShowAddAuthUserModal(false);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      alert(`Gagal menghasilkan hash password: ${msg}`);
    } finally {
      setIsHashingPassword(false);
    }
  };

  const handleRemoveAuthUser = (index: number) => {
    setRules((prev) => ({
      ...prev,
      basic_auth: {
        ...prev.basic_auth,
        users: prev.basic_auth.users.filter((_, i) => i !== index),
      },
    }));
  };

  // Add IP rule
  const handleAddIpRule = (e: React.FormEvent) => {
    e.preventDefault();
    if (!ipCidr.trim()) return;
    const newRule: IpAccessRule = {
      id: createSiteRuleId('ip'),
      cidr_or_ip: ipCidr.trim(),
      action: ipAction,
      description: ipDesc.trim() || null,
    };
    setRules((prev) => ({
      ...prev,
      ip_access: {
        ...prev.ip_access,
        rules: [...prev.ip_access.rules, newRule],
      },
    }));
    setIpCidr('');
    setIpDesc('');
    setShowAddIpRuleModal(false);
  };

  const handleRemoveIpRule = (index: number) => {
    setRules((prev) => ({
      ...prev,
      ip_access: {
        ...prev.ip_access,
        rules: prev.ip_access.rules.filter((_, i) => i !== index),
      },
    }));
  };

  // Add error page
  const handleAddErrorPage = (e: React.FormEvent) => {
    e.preventDefault();
    if (!errorPath.trim()) return;
    const newErr: CustomErrorPage = {
      status_code: errorCode,
      target: errorPath.trim(),
    };
    setRules((prev) => ({
      ...prev,
      custom_error_pages: [
        ...prev.custom_error_pages.filter((ep) => ep.status_code !== errorCode),
        newErr,
      ],
    }));
    setShowAddErrorPageModal(false);
  };

  const handleRemoveErrorPage = (statusCode: number) => {
    setRules((prev) => ({
      ...prev,
      custom_error_pages: prev.custom_error_pages.filter((ep) => ep.status_code !== statusCode),
    }));
  };

  // Validate Snippet
  const handleValidateSnippet = async () => {
    setSnippetTesting(true);
    setSnippetValidation(null);
    try {
      const resp = await validateNginxSnippet(site.id, rules.nginx_snippet.content);
      if (resp.valid) {
        setSnippetValidation({
          valid: true,
          message: resp.message || 'Snippet Nginx valid dan memenuhi kriteria keamanan.',
        });
      } else {
        setSnippetValidation({
          valid: false,
          message: resp.error || 'Snippet melanggar batasan direktif keamanan.',
        });
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setSnippetValidation({
        valid: false,
        message: `Validasi gagal: ${msg}`,
      });
    } finally {
      setSnippetTesting(false);
    }
  };

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center py-20 text-[var(--ink-muted)]">
        <Loader2 className="w-8 h-8 animate-spin text-[var(--primary)] mb-3" />
        <p className="text-sm">Memuat aturan situs...</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Top Banner & Header */}
      <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-[var(--ink)] flex items-center gap-2">
            <Sliders className="w-5 h-5 text-[var(--primary)]" />
            {t('sites.rules.title', 'Aturan Situs & Keamanan Web')}
          </h2>
          <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-2xl">
            {t(
              'sites.rules.subtitle',
              'Konfigurasi redirect terstruktur, header keamanan, CORS, proteksi hotlink, basic auth, kontrol IP, mode maintenance, halaman error, dan snippet Nginx.'
            )}
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <button
            onClick={loadRules}
            disabled={isSaving}
            className="px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] hover:bg-[var(--card-hover)] transition-colors flex items-center gap-1.5"
            title="Muat ulang pengaturan dari server"
          >
            <RefreshCw className="w-3.5 h-3.5" />
            Segarkan
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving || isLoading || !hasLoadedRules}
            className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity flex items-center gap-2 shadow-sm disabled:opacity-50"
          >
            {isSaving ? (
              <>
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                {t('sites.rules.saving', 'Menerapkan ke Nginx...')}
              </>
            ) : (
              <>
                <Check className="w-3.5 h-3.5" />
                {t('sites.rules.save', 'Simpan Perubahan Aturan')}
              </>
            )}
          </button>
        </div>
      </div>

      {/* Global Feedback Alert */}
      {feedback && (
        <div
          className={`p-3.5 rounded-[var(--radius-md)] text-xs flex items-center justify-between border ${
            feedback.type === 'success'
              ? 'bg-[var(--success)]/10 border-[var(--success)]/20 text-[var(--success)]'
              : 'bg-[var(--danger)]/10 border-[var(--danger)]/20 text-[var(--danger)]'
          }`}
        >
          <div className="flex items-center gap-2">
            {feedback.type === 'success' ? (
              <CheckCircle2 className="w-4 h-4 shrink-0" />
            ) : (
              <AlertCircle className="w-4 h-4 shrink-0" />
            )}
            <span>{feedback.message}</span>
          </div>
          <button onClick={() => setFeedback(null)} className="hover:opacity-75">
            <X className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Section Navigation Pills */}
      <div className="flex items-center gap-1.5 border-b border-[var(--line)] pb-2 overflow-x-auto text-xs font-medium">
        {[
          { id: 'redirects', label: t('sites.rules.tabRedirects', 'Redirect'), icon: ArrowRight },
          { id: 'headers', label: t('sites.rules.tabHeaders', 'Header & Keamanan'), icon: Shield },
          { id: 'cors', label: t('sites.rules.tabCors', 'CORS'), icon: Globe },
          { id: 'hotlink', label: t('sites.rules.tabHotlink', 'Proteksi Hotlink'), icon: AlertTriangle },
          { id: 'auth', label: t('sites.rules.tabAuth', 'Basic Auth'), icon: Lock },
          { id: 'ip', label: t('sites.rules.tabIp', 'Akses IP'), icon: Filter },
          { id: 'maintenance', label: t('sites.rules.tabMaintenance', 'Maintenance'), icon: Power },
          { id: 'errors', label: t('sites.rules.tabErrors', 'Halaman Error'), icon: FileText },
          { id: 'snippet', label: t('sites.rules.tabSnippet', 'Snippet Nginx'), icon: Code2 },
        ].map((sec) => {
          const Icon = sec.icon;
          const isActive = activeSection === sec.id;
          return (
            <button
              key={sec.id}
              onClick={() => setActiveSection(sec.id as RuleSection)}
              className={`px-3 py-1.5 rounded-[var(--radius-md)] transition-colors flex items-center gap-1.5 shrink-0 ${
                isActive
                  ? 'bg-[var(--primary)] text-white font-semibold shadow-sm'
                  : 'bg-[var(--card)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--card-hover)]'
              }`}
            >
              <Icon className="w-3.5 h-3.5" />
              {sec.label}
            </button>
          );
        })}
      </div>

      {/* SECTION 1: REDIRECTS */}
      {activeSection === 'redirects' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)]">
                Redirect Terstruktur (301 / 302)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Arahkan path lama atau URL pola regex ke lokasi target baru dengan kode status yang tepat.
              </p>
            </div>
            <button
              onClick={() => setShowAddRedirectModal(true)}
              className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity flex items-center gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" />
              Tambah Redirect
            </button>
          </div>

          {rules.redirects.length === 0 ? (
            <div className="py-8 text-center border border-dashed border-[var(--line)] rounded-[var(--radius-md)] text-[var(--ink-muted)] text-xs">
              Belum ada aturan redirect yang dibuat. Klik tombol di atas untuk menambahkan redirect.
            </div>
          ) : (
            <div className="overflow-x-auto border border-[var(--line)] rounded-[var(--radius-md)]">
              <table className="w-full text-xs text-left">
                <thead className="bg-[var(--bg)] text-[var(--ink-muted)] border-b border-[var(--line)]">
                  <tr>
                    <th className="px-3 py-2">Tipe</th>
                    <th className="px-3 py-2">Path Asal</th>
                    <th className="px-3 py-2">Tujuan</th>
                    <th className="px-3 py-2">Status</th>
                    <th className="px-3 py-2 text-right">Aksi</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)]">
                  {rules.redirects.map((r, idx) => (
                    <tr key={r.id} className="hover:bg-[var(--card-hover)] transition-colors">
                      <td className="px-3 py-2 font-mono uppercase text-[var(--primary)] font-semibold">
                        {r.match_type}
                      </td>
                      <td className="px-3 py-2 font-mono font-medium text-[var(--ink)]">
                        {r.source}
                      </td>
                      <td className="px-3 py-2 font-mono text-[var(--ink)]">{r.target}</td>
                      <td className="px-3 py-2">
                        <span
                          className={`px-2 py-0.5 rounded text-[10px] font-bold ${
                            r.status_code === 301
                              ? 'bg-[var(--primary)]/10 text-[var(--primary)]'
                              : 'bg-[var(--warning)]/10 text-[var(--warning)]'
                          }`}
                        >
                          {r.status_code}
                        </span>
                        <label className="mt-2 flex items-center gap-1.5 text-[var(--ink-muted)]">
                          <input
                            type="checkbox"
                            aria-label={`Aktifkan redirect ${r.source}`}
                            checked={r.enabled}
                            onChange={(e) => setRules((prev) => ({
                              ...prev,
                              redirects: prev.redirects.map((rule) => rule.id === r.id
                                ? { ...rule, enabled: e.target.checked } : rule),
                            }))}
                          />
                          Aktif
                        </label>
                      </td>
                      <td className="px-3 py-2 text-right">
                        <button
                          onClick={() => handleRemoveRedirect(idx)}
                          className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10 transition-colors"
                          title="Hapus redirect"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* SECTION 2: HEADERS & SECURITY */}
      {activeSection === 'headers' && (
        <div className="space-y-6">
          {/* Security Headers Card */}
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Shield className="w-4 h-4 text-[var(--primary)]" />
                Header Keamanan Bawaan (Security Headers)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Terapkan best-practice keamanan browser standar industri untuk melindungi pengunjung situs.
              </p>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
              {/* HSTS */}
              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] space-y-2">
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-[var(--ink)]">HTTP Strict Transport Security (HSTS)</span>
                  <input
                    type="checkbox"
                    checked={rules.security_headers.hsts_enabled}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        security_headers: {
                          ...prev.security_headers,
                          hsts_enabled: e.target.checked,
                        },
                      }))
                    }
                    className="rounded text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                </div>
                {rules.security_headers.hsts_enabled && (
                  <div className="space-y-2 pt-1 border-t border-[var(--line)]">
                    <p className="text-[var(--ink-muted)]">Max-age: 31536000 detik (1 tahun).</p>
                    <label className="flex items-center gap-2 cursor-pointer text-[var(--ink-muted)]">
                      <input
                        type="checkbox"
                        checked={rules.security_headers.hsts_subdomains}
                        onChange={(e) =>
                          setRules((prev) => ({
                            ...prev,
                            security_headers: {
                              ...prev.security_headers,
                              hsts_subdomains: e.target.checked,
                            },
                          }))
                        }
                        className="rounded text-[var(--primary)]"
                      />
                      Sertakan Subdomain (includeSubDomains)
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer text-[var(--ink-muted)]">
                      <input
                        type="checkbox"
                        checked={rules.security_headers.hsts_preload}
                        onChange={(e) =>
                          setRules((prev) => ({
                            ...prev,
                            security_headers: {
                              ...prev.security_headers,
                              hsts_preload: e.target.checked,
                            },
                          }))
                        }
                        className="rounded text-[var(--primary)]"
                      />
                      Izinkan Preload HSTS (preload)
                    </label>
                  </div>
                )}
              </div>

              {/* X-Frame-Options */}
              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] space-y-2">
                <span className="font-semibold text-[var(--ink)] block">X-Frame-Options (Proteksi Clickjacking)</span>
                <select
                  value={rules.security_headers.x_frame_options || ''}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      security_headers: {
                        ...prev.security_headers,
                        x_frame_options: e.target.value || null,
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--card)] text-[var(--ink)] text-xs"
                >
                  <option value="DENY">DENY (Tolak semua iframe)</option>
                  <option value="SAMEORIGIN">SAMEORIGIN (Hanya domain yang sama)</option>
                  <option value="">Nonaktifkan</option>
                </select>
              </div>

              {/* X-Content-Type-Options */}
              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] flex items-center justify-between">
                <div>
                  <span className="font-semibold text-[var(--ink)] block">X-Content-Type-Options: nosniff</span>
                  <span className="text-[var(--ink-muted)] text-[11px]">
                    Cegah MIME-sniffing browser
                  </span>
                </div>
                <input
                  type="checkbox"
                  checked={rules.security_headers.x_content_type_options}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      security_headers: {
                        ...prev.security_headers,
                        x_content_type_options: e.target.checked,
                      },
                    }))
                  }
                  className="rounded text-[var(--primary)]"
                />
              </div>

              {/* Referrer-Policy */}
              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] space-y-1.5">
                <span className="font-semibold text-[var(--ink)] block">Referrer-Policy</span>
                <select
                  value={rules.security_headers.referrer_policy || ''}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      security_headers: {
                        ...prev.security_headers,
                        referrer_policy: e.target.value || null,
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--card)] text-[var(--ink)] text-xs"
                >
                  <option value="strict-origin-when-cross-origin">strict-origin-when-cross-origin (Disarankan)</option>
                  <option value="no-referrer">no-referrer</option>
                  <option value="same-origin">same-origin</option>
                  <option value="strict-origin">strict-origin</option>
                  <option value="origin-when-cross-origin">origin-when-cross-origin</option>
                  <option value="">Nonaktifkan</option>
                </select>
              </div>
            </div>

            {/* Permissions Policy & CSP */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs pt-2">
              <div className="space-y-1">
                <label className="font-semibold text-[var(--ink)]">Permissions-Policy:</label>
                <input
                  type="text"
                  placeholder="camera=(), microphone=(), geolocation=()"
                  value={rules.security_headers.permissions_policy || ''}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      security_headers: {
                        ...prev.security_headers,
                        permissions_policy: e.target.value.trim() ? e.target.value : null,
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                />
              </div>
              <div className="space-y-1">
                <label className="font-semibold text-[var(--ink)]">Content-Security-Policy (CSP):</label>
                <input
                  type="text"
                  placeholder="default-src 'self'; script-src 'self' 'unsafe-inline'"
                  value={rules.security_headers.csp || ''}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      security_headers: {
                        ...prev.security_headers,
                        csp: e.target.value.trim() ? e.target.value : null,
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                />
              </div>
            </div>
          </div>

          {/* Custom Key-Value Headers Card */}
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
            <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <h3 className="text-sm font-semibold text-[var(--ink)]">Header Respons Kustom (add_header)</h3>
                <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                  Tambahkan header HTTP kustom ke setiap respons web server Nginx.
                </p>
              </div>
              <button
                onClick={() => setShowAddHeaderModal(true)}
                className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity flex items-center gap-1.5"
              >
                <Plus className="w-3.5 h-3.5" />
                Tambah Header
              </button>
            </div>

            {rules.custom_headers.length === 0 ? (
              <div className="py-6 text-center border border-dashed border-[var(--line)] rounded-[var(--radius-md)] text-[var(--ink-muted)] text-xs">
                Belum ada header respons kustom.
              </div>
            ) : (
              <div className="overflow-x-auto border border-[var(--line)] rounded-[var(--radius-md)]">
                <table className="w-full text-xs text-left">
                  <thead className="bg-[var(--bg)] text-[var(--ink-muted)] border-b border-[var(--line)]">
                    <tr>
                      <th className="px-3 py-2">Nama Header</th>
                      <th className="px-3 py-2">Nilai Header</th>
                      <th className="px-3 py-2">Flag Always</th>
                      <th className="px-3 py-2 text-right">Aksi</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-[var(--line)] font-mono">
                    {rules.custom_headers.map((h, idx) => (
                      <tr key={h.id} className="hover:bg-[var(--card-hover)] transition-colors">
                        <td className="px-3 py-2 font-semibold text-[var(--ink)]">{h.name}</td>
                        <td className="px-3 py-2 text-[var(--ink-muted)]">{h.value}</td>
                        <td className="px-3 py-2">
                          {h.always ? (
                            <span className="px-1.5 py-0.5 rounded text-[10px] bg-[var(--success)]/10 text-[var(--success)] font-sans font-medium">
                              always
                            </span>
                          ) : (
                            <span className="text-[var(--ink-muted)] font-sans">-</span>
                          )}
                        </td>
                        <td className="px-3 py-2 text-right font-sans">
                          <input
                            type="checkbox"
                            aria-label={`Aktifkan header ${h.name}`}
                            checked={h.enabled}
                            onChange={(e) => setRules((prev) => ({
                              ...prev,
                              custom_headers: prev.custom_headers.map((header) => header.id === h.id
                                ? { ...header, enabled: e.target.checked } : header),
                            }))}
                          />
                          <button
                            onClick={() => handleRemoveHeader(idx)}
                            aria-label={`Hapus header ${h.name}`}
                            className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10 transition-colors"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      )}

      {/* SECTION 3: CORS PRESET */}
      {activeSection === 'cors' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div>
            <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
              <Globe className="w-4 h-4 text-[var(--primary)]" />
              Cross-Origin Resource Sharing (CORS)
            </h3>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              Atur izin akses lintas domain untuk API atau resource web dengan preflight OPTIONS 204 otomatis.
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-4 gap-3 text-xs pt-1">
            {[
              { id: 'disabled', label: 'Dinonaktifkan', desc: 'Tidak menambahkan header CORS' },
              { id: 'allow_all', label: 'Izinkan Semua (*)', desc: 'Access-Control-Allow-Origin: *' },
              { id: 'same_origin', label: 'Same-Origin', desc: 'Hanya domain dan protokol yang sama' },
              { id: 'custom', label: 'Domain Kustom', desc: 'Whitelist domain tertentu' },
            ].map((p) => {
              const isSelected = rules.cors.preset === p.id;
              return (
                <div
                  key={p.id}
                  onClick={() =>
                    setRules((prev) => ({
                      ...prev,
                      cors: { ...prev.cors, preset: p.id as CorsPreset, enabled: p.id !== 'disabled' },
                    }))
                  }
                  className={`p-3.5 rounded-[var(--radius-md)] border cursor-pointer transition-all ${
                    isSelected
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)] font-semibold shadow-xs'
                      : 'border-[var(--line)] bg-[var(--bg)] text-[var(--ink-muted)] hover:border-[var(--primary)]/50'
                  }`}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span>{p.label}</span>
                    <input
                      type="radio"
                      name="cors_preset"
                      checked={isSelected}
                      onChange={() => {}}
                      className="text-[var(--primary)]"
                    />
                  </div>
                  <p className="text-[11px] font-normal text-[var(--ink-muted)]">{p.desc}</p>
                </div>
              );
            })}
          </div>

          {rules.cors.preset === 'custom' && (
            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] space-y-3 text-xs">
              <div>
                <label className="font-semibold text-[var(--ink)] block mb-1">
                  Origin yang Diizinkan (pisahkan dengan koma atau baris baru):
                </label>
                <textarea
                  rows={2}
                  placeholder="https://app.example.com, https://admin.example.com"
                  value={rules.cors.allowed_origins.join(', ')}
                  onChange={(e) => {
                    const list = e.target.value
                      .split(/[,\n]/)
                      .map((s) => s.trim())
                      .filter(Boolean);
                    setRules((prev) => ({
                      ...prev,
                      cors: { ...prev.cors, allowed_origins: list },
                    }));
                  }}
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--card)] text-[var(--ink)] font-mono text-xs"
                />
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <label className="flex items-center gap-2 cursor-pointer text-[var(--ink)]">
                  <input
                    type="checkbox"
                    checked={rules.cors.allow_credentials}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        cors: { ...prev.cors, allow_credentials: e.target.checked },
                      }))
                    }
                    className="rounded text-[var(--primary)]"
                  />
                  Izinkan Kredensial (Access-Control-Allow-Credentials: true)
                </label>
                <div className="flex items-center gap-2">
                  <span className="text-[var(--ink-muted)]">Max-Age Preflight:</span>
                  <input
                    type="number"
                    value={rules.cors.max_age_secs}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        cors: { ...prev.cors, max_age_secs: Number(e.target.value) },
                      }))
                    }
                    className="w-24 px-2 py-1 rounded border border-[var(--line)] bg-[var(--card)] text-[var(--ink)] font-mono text-xs"
                  />
                  <span className="text-[var(--ink-muted)]">detik</span>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* SECTION 4: HOTLINK PROTECTION */}
      {activeSection === 'hotlink' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <AlertTriangle className="w-4 h-4 text-[var(--primary)]" />
                Perlindungan Hotlink (Anti-Leech)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Cegah situs eksternal mencuri bandwidth dengan langsung menautkan gambar atau media Anda.
              </p>
            </div>
            <label className="flex items-center gap-2 cursor-pointer text-xs font-semibold text-[var(--ink)]">
              <input
                type="checkbox"
                checked={rules.hotlink_protection.enabled}
                onChange={(e) =>
                  setRules((prev) => ({
                    ...prev,
                    hotlink_protection: { ...prev.hotlink_protection, enabled: e.target.checked },
                  }))
                }
                className="rounded text-[var(--primary)] w-4 h-4"
              />
              Aktifkan Proteksi
            </label>
          </div>

          {rules.hotlink_protection.enabled && (
            <div className="space-y-4 pt-2 border-t border-[var(--line)] text-xs">
              <div>
                <label className="font-semibold text-[var(--ink)] block mb-1">
                  Domain Referrer yang Diizinkan (whitelist):
                </label>
                <textarea
                  rows={2}
                  placeholder="cdn.example.com, partner.org"
                  value={rules.hotlink_protection.allowed_domains.join(', ')}
                  onChange={(e) => {
                    const list = e.target.value
                      .split(/[,\n]/)
                      .map((s) => s.trim())
                      .filter(Boolean);
                    setRules((prev) => ({
                      ...prev,
                      hotlink_protection: { ...prev.hotlink_protection, allowed_domains: list },
                    }));
                  }}
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                />
              </div>

              <div>
                <label className="font-semibold text-[var(--ink)] block mb-1">
                  Ekstensi File yang Dilindungi:
                </label>
                <input
                  type="text"
                  placeholder="jpg, png, webp, gif, mp4"
                  value={rules.hotlink_protection.file_extensions.join(', ')}
                  onChange={(e) => {
                    const list = e.target.value
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean);
                    setRules((prev) => ({
                      ...prev,
                      hotlink_protection: { ...prev.hotlink_protection, file_extensions: list },
                    }));
                  }}
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                />
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <label className="flex items-center gap-2 cursor-pointer text-[var(--ink)]">
                  <input
                    type="checkbox"
                    checked={rules.hotlink_protection.allow_empty_referer}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        hotlink_protection: { ...prev.hotlink_protection, allow_empty_referer: e.target.checked },
                      }))
                    }
                    className="rounded text-[var(--primary)]"
                  />
                  Izinkan Referrer Kosong (Permintaan langsung browser / bookmark)
                </label>

                <div className="flex items-center gap-2">
                  <span className="font-semibold text-[var(--ink)]">Tindakan:</span>
                  <select
                    value={rules.hotlink_protection.action}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        hotlink_protection: { ...prev.hotlink_protection, action: e.target.value as HotlinkAction },
                      }))
                    }
                    className="px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] text-xs"
                  >
                    <option value="deny">Tolak dengan 403 Forbidden</option>
                    <option value="redirect">Redirect ke Gambar Peringatan</option>
                  </select>
                </div>
              </div>

              {rules.hotlink_protection.action === 'redirect' && (
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">
                    URL Gambar Peringatan / Placeholder:
                  </label>
                  <input
                    type="text"
                    placeholder="https://example.com/images/no-hotlink.png"
                    value={rules.hotlink_protection.redirect_url || ''}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        hotlink_protection: { ...prev.hotlink_protection, redirect_url: e.target.value },
                      }))
                    }
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                  />
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* SECTION 5: BASIC AUTH */}
      {activeSection === 'auth' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Lock className="w-4 h-4 text-[var(--primary)]" />
                Autentikasi Dasar HTTP (Basic Auth - bcrypt)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Kunci seluruh situs atau lokasi path tertentu dengan popup login browser dan hash bcrypt.
              </p>
            </div>
            <label className="flex items-center gap-2 cursor-pointer text-xs font-semibold text-[var(--ink)]">
              <input
                type="checkbox"
                checked={rules.basic_auth.enabled}
                onChange={(e) =>
                  setRules((prev) => ({
                    ...prev,
                    basic_auth: { ...prev.basic_auth, enabled: e.target.checked },
                  }))
                }
                className="rounded text-[var(--primary)] w-4 h-4"
              />
              Aktifkan Basic Auth
            </label>
          </div>

          {rules.basic_auth.enabled && (
            <div className="space-y-4 pt-2 border-t border-[var(--line)] text-xs">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Path Terproteksi:</label>
                  <input
                    type="text"
                    placeholder="/ atau /admin"
                    value={rules.basic_auth.path}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        basic_auth: { ...prev.basic_auth, path: e.target.value },
                      }))
                    }
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                  />
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Nama Realm (Judul Dialog):</label>
                  <input
                    type="text"
                    placeholder="Area Terproteksi"
                    value={rules.basic_auth.realm}
                    onChange={(e) =>
                      setRules((prev) => ({
                        ...prev,
                        basic_auth: { ...prev.basic_auth, realm: e.target.value },
                      }))
                    }
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] text-xs"
                  />
                </div>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-[var(--ink)]">Daftar Pengguna (Credentials)</span>
                  <button
                    onClick={() => setShowAddAuthUserModal(true)}
                    className="px-2.5 py-1 text-xs font-semibold rounded bg-[var(--primary)] text-white hover:opacity-90 transition-opacity flex items-center gap-1"
                  >
                    <Plus className="w-3.5 h-3.5" />
                    Tambah Pengguna
                  </button>
                </div>

                {rules.basic_auth.users.length === 0 ? (
                  <div className="p-4 text-center border border-dashed border-[var(--line)] rounded text-[var(--ink-muted)]">
                    Belum ada pengguna. Tambahkan minimal satu pengguna agar login dapat berfungsi.
                  </div>
                ) : (
                  <div className="overflow-x-auto border border-[var(--line)] rounded">
                    <table className="w-full text-xs text-left">
                      <thead className="bg-[var(--bg)] text-[var(--ink-muted)] border-b border-[var(--line)]">
                        <tr>
                          <th className="px-3 py-2">Username</th>
                          <th className="px-3 py-2">Hash Password (bcrypt)</th>
                          <th className="px-3 py-2 text-right">Aksi</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-[var(--line)] font-mono">
                        {rules.basic_auth.users.map((u, idx) => (
                          <tr key={idx} className="hover:bg-[var(--card-hover)]">
                            <td className="px-3 py-2 font-semibold text-[var(--ink)]">{u.username}</td>
                            <td className="px-3 py-2 text-[var(--ink-muted)] truncate max-w-xs">
                              {u.password_hash}
                            </td>
                            <td className="px-3 py-2 text-right font-sans">
                              <button
                                onClick={() => handleRemoveAuthUser(idx)}
                                className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10"
                              >
                                <Trash2 className="w-3.5 h-3.5" />
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      )}

      {/* SECTION 6: IP ACCESS */}
      {activeSection === 'ip' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Filter className="w-4 h-4 text-[var(--primary)]" />
                Kontrol Akses IP (Allow / Deny)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Batasi akses berdasarkan alamat IP atau subnet CIDR (Mode Whitelist atau Blacklist).
              </p>
            </div>
            <button
              onClick={() => setShowAddIpRuleModal(true)}
              className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 flex items-center gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" />
              Tambah Aturan IP
            </button>
          </div>

          <label className="flex items-center gap-2 text-xs text-[var(--ink)]">
            <input
              type="checkbox"
              checked={rules.ip_access.enabled}
              onChange={(e) => setRules((prev) => ({
                ...prev,
                ip_access: { ...prev.ip_access, enabled: e.target.checked },
              }))}
            />
            Aktifkan kontrol akses IP
          </label>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
            <div>
              <label className="font-semibold text-[var(--ink)] block mb-1">
                Tindakan Default (Jika tidak cocok dengan aturan):
              </label>
              <select
                value={rules.ip_access.default_action}
                onChange={(e) =>
                  setRules((prev) => ({
                    ...prev,
                    ip_access: {
                      ...prev.ip_access,
                      default_action: e.target.value as IpAction,
                    },
                  }))
                }
                className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] text-xs"
              >
                <option value="allow">Izinkan Semua (Mode Blacklist - Hanya blokir IP terdaftar)</option>
                <option value="deny">Tolak Semua (Mode Whitelist - Hanya izinkan IP terdaftar)</option>
              </select>
            </div>

            <div>
              <label className="font-semibold text-[var(--ink)] block mb-1">
                Path Spesifik (Opsional, kosongkan untuk seluruh situs):
              </label>
              <input
                type="text"
                placeholder="misal: /wp-admin atau /api/secret"
                value={rules.ip_access.path || ''}
                onChange={(e) =>
                  setRules((prev) => ({
                    ...prev,
                    ip_access: {
                      ...prev.ip_access,
                      scope: e.target.value.trim() ? 'path' : 'global',
                      path: e.target.value.trim() ? e.target.value.trim() : null,
                    },
                  }))
                }
                className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
              />
            </div>
          </div>

          {rules.ip_access.rules.length === 0 ? (
            <div className="py-6 text-center border border-dashed border-[var(--line)] rounded text-[var(--ink-muted)] text-xs">
              Belum ada aturan IP khusus yang didaftarkan.
            </div>
          ) : (
            <div className="overflow-x-auto border border-[var(--line)] rounded text-xs">
              <table className="w-full text-left">
                <thead className="bg-[var(--bg)] text-[var(--ink-muted)] border-b border-[var(--line)]">
                  <tr>
                    <th className="px-3 py-2">Tindakan</th>
                    <th className="px-3 py-2">Alamat IP / CIDR</th>
                    <th className="px-3 py-2">Keterangan</th>
                    <th className="px-3 py-2 text-right">Aksi</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)]">
                  {rules.ip_access.rules.map((rule, idx) => (
                    <tr key={rule.id} className="hover:bg-[var(--card-hover)]">
                      <td className="px-3 py-2">
                        <span
                          className={`px-2 py-0.5 rounded text-[10px] font-bold uppercase ${
                            rule.action === 'allow'
                              ? 'bg-[var(--success)]/10 text-[var(--success)]'
                              : 'bg-[var(--danger)]/10 text-[var(--danger)]'
                          }`}
                        >
                          {rule.action}
                        </span>
                      </td>
                      <td className="px-3 py-2 font-mono font-semibold text-[var(--ink)]">
                        {rule.cidr_or_ip}
                      </td>
                      <td className="px-3 py-2 text-[var(--ink-muted)]">
                        {rule.description || '-'}
                      </td>
                      <td className="px-3 py-2 text-right">
                        <button
                          onClick={() => handleRemoveIpRule(idx)}
                          className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* SECTION 7: MAINTENANCE MODE */}
      {activeSection === 'maintenance' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Power className="w-4 h-4 text-[var(--warning)]" />
                Mode Pemeliharaan (Maintenance Mode - 503)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Kunci akses publik dengan respons HTTP 503 dan Retry-After, dengan pengecualian IP khusus.
              </p>
            </div>
            <label className="flex items-center gap-2 cursor-pointer text-xs font-semibold text-[var(--ink)]">
              <input
                type="checkbox"
                checked={rules.maintenance.enabled}
                onChange={(e) =>
                  setRules((prev) => ({
                    ...prev,
                    maintenance: { ...prev.maintenance, enabled: e.target.checked },
                  }))
                }
                className="rounded text-[var(--warning)] w-4 h-4"
              />
              Aktifkan Maintenance
            </label>
          </div>

          {rules.maintenance.enabled && (
            <div className="p-3 bg-[var(--warning)]/10 border border-[var(--warning)]/30 rounded-[var(--radius-md)] text-xs text-[var(--warning)] flex items-center gap-2 font-medium">
              <AlertTriangle className="w-4 h-4 shrink-0" />
              Situs ini saat ini akan merespons 503 Service Unavailable untuk semua pengunjung selain IP bypass.
            </div>
          )}

          <div className="space-y-3 pt-2 text-xs">
            <div>
              <label className="font-semibold text-[var(--ink)] block mb-1">
                Daftar IP yang Dikecualikan (Bypass IPs):
              </label>
              <input
                type="text"
                placeholder="127.0.0.1, 192.168.1.100, 203.0.113.50"
                value={rules.maintenance.bypass_ips.join(', ')}
                onChange={(e) => {
                  const list = e.target.value
                    .split(',')
                    .map((s) => s.trim())
                    .filter(Boolean);
                  setRules((prev) => ({
                    ...prev,
                    maintenance: { ...prev.maintenance, bypass_ips: list },
                  }));
                }}
                className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
              />
              <span className="text-[11px] text-[var(--ink-muted)] mt-0.5 block">
                IP pengembang / administrator yang tetap dapat membuka dan menguji situs saat maintenance.
              </span>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="font-semibold text-[var(--ink)] block mb-1">
                  Header Retry-After (Detik):
                </label>
                <input
                  type="number"
                  value={rules.maintenance.retry_after_secs}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      maintenance: {
                        ...prev.maintenance,
                        retry_after_secs: Number(e.target.value),
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs"
                />
              </div>

              <div>
                <label className="font-semibold text-[var(--ink)] block mb-1">
                  Pesan Pemeliharaan Kustom:
                </label>
                <input
                  type="text"
                  value={rules.maintenance.custom_message || ''}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      maintenance: {
                        ...prev.maintenance,
                        custom_message: e.target.value,
                      },
                    }))
                  }
                  className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] text-xs"
                />
              </div>
            </div>
          </div>
        </div>
      )}

      {/* SECTION 8: ERROR PAGES */}
      {activeSection === 'errors' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <FileText className="w-4 h-4 text-[var(--primary)]" />
                Halaman Error Kustom (Custom Error Pages)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Arahkan kode status error HTTP (404, 500, dll.) ke halaman HTML kustom milik Anda.
              </p>
            </div>
            <button
              onClick={() => setShowAddErrorPageModal(true)}
              className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 flex items-center gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" />
              Tambah Error Page
            </button>
          </div>

          <p className="text-xs leading-relaxed text-[var(--ink-muted)]">
            Untuk memakai desain sendiri, unggah HTML ke docroot situs (misalnya /errors/404.html), lalu tambahkan kode 404 dan path tersebut.
            {' '}<a href="/errors/404.html" download="qinpanel-404.html" className="font-medium text-[var(--primary)] underline underline-offset-4">Unduh contoh 404 Qinpanel</a>.
          </p>

          {rules.custom_error_pages.length === 0 ? (
            <div className="py-6 text-center border border-dashed border-[var(--line)] rounded text-[var(--ink-muted)] text-xs">
              Belum ada halaman error kustom. Error 404 dari server memakai halaman bawaan Qinpanel.
            </div>
          ) : (
            <div className="overflow-x-auto border border-[var(--line)] rounded text-xs">
              <table className="w-full text-left">
                <thead className="bg-[var(--bg)] text-[var(--ink-muted)] border-b border-[var(--line)]">
                  <tr>
                    <th className="px-3 py-2">Kode Error</th>
                    <th className="px-3 py-2">Path File Target</th>
                    <th className="px-3 py-2 text-right">Aksi</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)] font-mono">
                  {rules.custom_error_pages.map((ep) => (
                    <tr key={ep.status_code} className="hover:bg-[var(--card-hover)]">
                      <td className="px-3 py-2 font-bold text-[var(--primary)]">
                        {ep.status_code}
                      </td>
                      <td className="px-3 py-2 text-[var(--ink)]">{ep.target}</td>
                      <td className="px-3 py-2 text-right font-sans">
                        <button
                          onClick={() => handleRemoveErrorPage(ep.status_code)}
                          className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* SECTION 9: NGINX SNIPPET */}
      {activeSection === 'snippet' && (
        <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-sm space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Code2 className="w-4 h-4 text-[var(--primary)]" />
                Snippet Nginx Kustom (Admin Only)
              </h3>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                Sisipkan direktif Nginx kustom langsung ke dalam blok server situs ini.
              </p>
            </div>
            {isAdmin ? (
              <label className="flex items-center gap-2 cursor-pointer text-xs font-semibold text-[var(--ink)]">
                <input
                  type="checkbox"
                  checked={rules.nginx_snippet.enabled}
                  onChange={(e) =>
                    setRules((prev) => ({
                      ...prev,
                      nginx_snippet: {
                        ...prev.nginx_snippet,
                        enabled: e.target.checked,
                      },
                    }))
                  }
                  className="rounded text-[var(--primary)] w-4 h-4"
                />
                Aktifkan Snippet
              </label>
            ) : (
              <span className="px-2 py-0.5 rounded text-[11px] bg-[var(--danger)]/10 text-[var(--danger)] font-medium">
                Khusus Administrator
              </span>
            )}
          </div>

          {!isAdmin && (
            <div className="p-3 bg-[var(--warning)]/10 border border-[var(--warning)]/30 rounded text-xs text-[var(--warning)] flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              Anda masuk sebagai tenant reguler. Untuk alasan keamanan, konfigurasi snippet Nginx mentah hanya dapat diubah oleh administrator sistem.
            </div>
          )}

          <div className="space-y-2 text-xs">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-[var(--ink)]">Direktif Nginx:</span>
              {isAdmin && (
                <button
                  onClick={handleValidateSnippet}
                  disabled={snippetTesting || !rules.nginx_snippet.content.trim()}
                  className="px-2.5 py-1 text-xs font-medium rounded border border-[var(--line)] bg-[var(--bg)] hover:bg-[var(--card-hover)] flex items-center gap-1.5 disabled:opacity-50"
                >
                  {snippetTesting ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <CheckCircle2 className="w-3.5 h-3.5 text-[var(--primary)]" />
                  )}
                  Uji Validitas Sintaks & Keamanan
                </button>
              )}
            </div>

            {snippetValidation && (
              <div
                className={`p-2.5 rounded text-xs flex items-center gap-2 ${
                  snippetValidation.valid
                    ? 'bg-[var(--success)]/10 text-[var(--success)] border border-[var(--success)]/20'
                    : 'bg-[var(--danger)]/10 text-[var(--danger)] border border-[var(--danger)]/20'
                }`}
              >
                {snippetValidation.valid ? (
                  <CheckCircle2 className="w-4 h-4 shrink-0" />
                ) : (
                  <AlertCircle className="w-4 h-4 shrink-0" />
                )}
                <span>{snippetValidation.message}</span>
              </div>
            )}

            <textarea
              rows={8}
              disabled={!isAdmin}
              placeholder="# Contoh direktif aman:&#10;client_body_buffer_size 32k;&#10;large_client_header_buffers 4 16k;"
              value={rules.nginx_snippet.content}
              onChange={(e) =>
                setRules((prev) => ({
                  ...prev,
                  nginx_snippet: {
                    ...prev.nginx_snippet,
                    content: e.target.value,
                  },
                }))
              }
              className="w-full p-3 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono text-xs focus:ring-1 focus:ring-[var(--primary)] disabled:opacity-60"
            />

            <div className="p-3 rounded bg-[var(--bg)] border border-[var(--line)] text-[11px] text-[var(--ink-muted)] space-y-1">
              <span className="font-semibold text-[var(--ink)] block">Batasan Keamanan (Strict Denylist):</span>
              <p>
                Direktif berikut dilarang demi keamanan isolasi server: <code className="text-[var(--danger)]">user</code>,{' '}
                <code className="text-[var(--danger)]">root /</code>,{' '}
                <code className="text-[var(--danger)]">alias /</code>,{' '}
                <code className="text-[var(--danger)]">load_module</code>,{' '}
                <code className="text-[var(--danger)]">include</code>, dan IP metadata cloud (SSRF).
              </p>
            </div>
          </div>
        </div>
      )}

      {/* MODAL 1: ADD REDIRECT */}
      {showAddRedirectModal && (
        <ModalOverlay
          className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
          onDismiss={() => setShowAddRedirectModal(false)}
        >
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 max-w-md w-full shadow-lg space-y-4 text-xs dialog-panel">
            <div className="flex items-center justify-between dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">Tambah Aturan Redirect</h3>
              <button onClick={() => setShowAddRedirectModal(false)} className="hover:opacity-75">
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleAddRedirect} className="dialog-form">
              <div className="dialog-body space-y-3">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Path Asal:</label>
                  <input
                    type="text"
                    required
                    placeholder="/blog/old-post"
                    value={redirSource}
                    onChange={(e) => setRedirSource(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Tujuan (URL / Path):</label>
                  <input
                    type="text"
                    required
                    placeholder="https://example.com/new-post atau /new-post"
                    value={redirTarget}
                    onChange={(e) => setRedirTarget(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="font-semibold text-[var(--ink)] block mb-1">Tipe Pencocokan:</label>
                    <select
                      value={redirType}
                      onChange={(e) => setRedirType(e.target.value as RedirectMatchType)}
                      className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                    >
                      <option value="prefix">Prefix (Awalan path)</option>
                      <option value="exact">Exact (Persis sama)</option>
                      <option value="regex">Regex (Ekspresi reguler)</option>
                    </select>
                  </div>
                  <div>
                    <label className="font-semibold text-[var(--ink)] block mb-1">Kode Status:</label>
                    <select
                      value={redirCode}
                      onChange={(e) => setRedirCode(Number(e.target.value))}
                      className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                    >
                      <option value={301}>301 (Permanen)</option>
                      <option value={302}>302 (Sementara)</option>
                    </select>
                  </div>
                </div>
              </div>
              <div className="flex justify-end gap-2 pt-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setShowAddRedirectModal(false)}
                  className="px-3 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                >
                  Batal
                </button>
                <button type="submit" className="px-3 py-1.5 rounded bg-[var(--primary)] text-white font-semibold">
                  Tambahkan
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* MODAL 2: ADD HEADER */}
      {showAddHeaderModal && (
        <ModalOverlay
          className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
          onDismiss={() => setShowAddHeaderModal(false)}
        >
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 max-w-md w-full shadow-lg space-y-4 text-xs dialog-panel">
            <div className="flex items-center justify-between dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">Tambah Header Respons</h3>
              <button onClick={() => setShowAddHeaderModal(false)} className="hover:opacity-75">
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleAddHeader} className="dialog-form">
              <div className="dialog-body space-y-3">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Nama Header:</label>
                  <input
                    type="text"
                    required
                    placeholder="X-Custom-Header"
                    value={hdrName}
                    onChange={(e) => setHdrName(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Nilai Header:</label>
                  <input
                    type="text"
                    required
                    placeholder="custom-value"
                    value={hdrValue}
                    onChange={(e) => setHdrValue(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <label className="flex items-center gap-2 cursor-pointer text-[var(--ink)] pt-1">
                  <input
                    type="checkbox"
                    checked={hdrAlways}
                    onChange={(e) => setHdrAlways(e.target.checked)}
                    className="rounded text-[var(--primary)]"
                  />
                  Sertakan pada respon error juga (flag always)
                </label>
              </div>
              <div className="flex justify-end gap-2 pt-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setShowAddHeaderModal(false)}
                  className="px-3 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                >
                  Batal
                </button>
                <button type="submit" className="px-3 py-1.5 rounded bg-[var(--primary)] text-white font-semibold">
                  Tambahkan
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* MODAL 3: ADD AUTH USER */}
      {showAddAuthUserModal && (
        <ModalOverlay
          className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
          onDismiss={() => setShowAddAuthUserModal(false)}
        >
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 max-w-md w-full shadow-lg space-y-4 text-xs dialog-panel">
            <div className="flex items-center justify-between dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">Tambah Pengguna Basic Auth</h3>
              <button onClick={() => setShowAddAuthUserModal(false)} className="hover:opacity-75">
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleAddAuthUser} className="dialog-form">
              <div className="dialog-body space-y-3">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Nama Pengguna (Username):</label>
                  <input
                    type="text"
                    required
                    placeholder="admin"
                    value={authUsername}
                    onChange={(e) => setAuthUsername(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Kata Sandi:</label>
                  <input
                    type="password"
                    required
                    placeholder="••••••••••••"
                    value={authPassword}
                    onChange={(e) => setAuthPassword(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                  />
                  <span className="text-[11px] text-[var(--ink-muted)] mt-1 block">
                    Kata sandi akan otomatis dienkripsi dengan bcrypt sebelum disimpan ke berkas htpasswd.
                  </span>
                </div>
              </div>
              <div className="flex justify-end gap-2 pt-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setShowAddAuthUserModal(false)}
                  className="px-3 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isHashingPassword}
                  className="px-3 py-1.5 rounded bg-[var(--primary)] text-white font-semibold flex items-center gap-1.5 disabled:opacity-50"
                >
                  {isHashingPassword && <Loader2 className="w-3 h-3 animate-spin" />}
                  Tambahkan & Hash
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* MODAL 4: ADD IP RULE */}
      {showAddIpRuleModal && (
        <ModalOverlay
          className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
          onDismiss={() => setShowAddIpRuleModal(false)}
        >
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 max-w-md w-full shadow-lg space-y-4 text-xs dialog-panel">
            <div className="flex items-center justify-between dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">Tambah Aturan IP</h3>
              <button onClick={() => setShowAddIpRuleModal(false)} className="hover:opacity-75">
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleAddIpRule} className="dialog-form">
              <div className="dialog-body space-y-3">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Alamat IP atau Subnet CIDR:</label>
                  <input
                    type="text"
                    required
                    placeholder="192.168.1.100 atau 10.0.0.0/24"
                    value={ipCidr}
                    onChange={(e) => setIpCidr(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Tindakan:</label>
                  <select
                    value={ipAction}
                    onChange={(e) => setIpAction(e.target.value as IpAction)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                  >
                    <option value="allow">Izinkan (Allow)</option>
                    <option value="deny">Tolak (Deny)</option>
                  </select>
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Keterangan (Opsional):</label>
                  <input
                    type="text"
                    placeholder="Kantor pusat"
                    value={ipDesc}
                    onChange={(e) => setIpDesc(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                  />
                </div>
              </div>
              <div className="flex justify-end gap-2 pt-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setShowAddIpRuleModal(false)}
                  className="px-3 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                >
                  Batal
                </button>
                <button type="submit" className="px-3 py-1.5 rounded bg-[var(--primary)] text-white font-semibold">
                  Tambahkan
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* MODAL 5: ADD ERROR PAGE */}
      {showAddErrorPageModal && (
        <ModalOverlay
          className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
          onDismiss={() => setShowAddErrorPageModal(false)}
        >
          <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 max-w-md w-full shadow-lg space-y-4 text-xs dialog-panel">
            <div className="flex items-center justify-between dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">Tambah Pemetaan Halaman Error</h3>
              <button onClick={() => setShowAddErrorPageModal(false)} className="hover:opacity-75">
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleAddErrorPage} className="dialog-form">
              <div className="dialog-body space-y-3">
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Kode Status Error:</label>
                  <select
                    value={errorCode}
                    onChange={(e) => setErrorCode(Number(e.target.value))}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                  >
                    <option value={400}>400 Bad Request</option>
                    <option value={401}>401 Unauthorized</option>
                    <option value={403}>403 Forbidden</option>
                    <option value={404}>404 Not Found</option>
                    <option value={500}>500 Internal Server Error</option>
                    <option value={502}>502 Bad Gateway</option>
                    <option value={503}>503 Service Unavailable</option>
                    <option value={504}>504 Gateway Timeout</option>
                  </select>
                </div>
                <div>
                  <label className="font-semibold text-[var(--ink)] block mb-1">Path Target HTML:</label>
                  <input
                    type="text"
                    required
                    placeholder="/custom_404.html"
                    value={errorPath}
                    onChange={(e) => setErrorPath(e.target.value)}
                    className="w-full px-2.5 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] font-mono"
                  />
                </div>
              </div>
              <div className="flex justify-end gap-2 pt-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => setShowAddErrorPageModal(false)}
                  className="px-3 py-1.5 rounded border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)]"
                >
                  Batal
                </button>
                <button type="submit" className="px-3 py-1.5 rounded bg-[var(--primary)] text-white font-semibold">
                  Tambahkan
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
