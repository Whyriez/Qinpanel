import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect } from 'react';
import {
  X,
  Globe,
  ArrowRight,
  ArrowLeft,
  Loader2,
  CheckCircle2,
  AlertCircle,
  FileCode2,
  Server,
  Terminal,
  ShieldCheck,
  Network,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { TenantListItem, Site, CreateSiteRequest } from '../../types/api';
import { fetchTenants, fetchTenantDetail, createSite, fetchPhpVersions, fetchNodeVersions, getPythonVersions } from '../../api/client';
import { RouteLink } from '../../lib/router';
import { useAuth } from '../../context/AuthContext';

export interface SiteWizardDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: (site: Site) => void;
}

export const SiteWizardDialog: React.FC<SiteWizardDialogProps> = ({
  isOpen,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const { user } = useAuth();

  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const [siteType, setSiteType] = useState<'static' | 'php' | 'node' | 'python' | 'proxy'>('static');
  const [domain, setDomain] = useState('');
  const [spaFallback, setSpaFallback] = useState(false);
  const [forceHttps, setForceHttps] = useState(true);
  const [selectedTenantId, setSelectedTenantId] = useState<number | null>(null);
  const [phpVersions, setPhpVersions] = useState<string[]>([]);
  const [phpLoading, setPhpLoading] = useState(false);
  const [phpError, setPhpError] = useState('');
  const [phpVersion, setPhpVersion] = useState('');
  const [phpPreset, setPhpPreset] = useState<'generic'|'wordpress'|'laravel'|'custom'>('generic');
  const [phpEntry, setPhpEntry] = useState('index.php');

  // Node.js specific fields (Phase 7 - P7-T2 / P7-T6)
  const [nodeVersion, setNodeVersion] = useState<string>('');
  const [nodeFramework, setNodeFramework] = useState<string>('express');
  const [nodeEntrypoint, setNodeEntrypoint] = useState<string>('server.js');
  const [nodeEntrypointType, setNodeEntrypointType] = useState<string>('script');

  // Python specific fields (Phase 7 - P7-T3)
  const [pythonVersion, setPythonVersion] = useState<string>('');
  const [applicationRuntime, setApplicationRuntime] = useState<{
    kind: 'node' | 'python'; versions: string[]; loading: boolean; error: string;
  } | null>(null);
  const [runtimeReload, setRuntimeReload] = useState(0);
  const [pythonFramework, setPythonFramework] = useState<string>('django');
  const [pythonServer, setPythonServer] = useState<string>('gunicorn_wsgi');
  const [pythonEntrypointType, setPythonEntrypointType] = useState<string>('wsgi');
  const [pythonEntrypoint, setPythonEntrypoint] = useState<string>('myproject.wsgi:application');
  const [pythonWorkers, setPythonWorkers] = useState<number>(3);

  // Reverse Proxy specific fields (Phase 7 - P7-T4)
  const [proxyUpstream, setProxyUpstream] = useState<string>('http://127.0.0.1:3000');
  const [proxyPreserveHost, setProxyPreserveHost] = useState<boolean>(true);
  const [proxyWebsocket, setProxyWebsocket] = useState<boolean>(true);

  const [tenants, setTenants] = useState<TenantListItem[]>([]);
  const [isLoadingTenants, setIsLoadingTenants] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      setStep(1);
      setSiteType('static');
      setDomain('');
      setNodeVersion(''); setPythonVersion(''); setApplicationRuntime(null);
      setNodeFramework('express');
      setNodeEntrypoint('server.js');
      setNodeEntrypointType('script');
      setProxyUpstream('http://127.0.0.1:3000');
      setProxyPreserveHost(true);
      setProxyWebsocket(true);
      setSpaFallback(false);
      setForceHttps(true);
      setErrorMessage(null);

      // Load tenants for tenant selection
      setIsLoadingTenants(true);
      setSelectedTenantId(null);
      setPhpPreset('generic'); setPhpEntry('index.php');
      (user?.role === 'admin' ? fetchTenants() : fetchTenantDetail(user!.id).then(result => [result.tenant]))
        .then((data) => {
          data = data.filter(tenant => !!tenant.linux_username && !!tenant.linux_uid && tenant.status === 'active');
          setTenants(data);
          // If current logged-in user is a regular user or matches tenant, auto-select
          if (user?.role !== 'admin' && data.length > 0) {
            setSelectedTenantId(data[0].id);
          }
        })
        .catch((err) => {
          setErrorMessage('Gagal memuat daftar tenant: ' + (err.message || 'Error'));
        })
        .finally(() => {
          setIsLoadingTenants(false);
        });
    }
  }, [isOpen, user]);

  useEffect(() => {
    if (!isOpen || siteType !== 'php') return;
    let current = true;
    setPhpLoading(true); setPhpError('');
    fetchPhpVersions().then(result => {
      if (!current) return;
      const versions = result.versions.filter(v => v.installed && v.fpm_binary_exists).map(v=>v.version);
      setPhpVersions(versions); setPhpVersion(version => versions.includes(version) ? version : versions.at(-1) || '');
    }).catch(err => { if(current){setPhpVersions([]);setPhpVersion('');setPhpError(err.message || 'Gagal memuat runtime PHP.');} }).finally(()=>{if(current)setPhpLoading(false);});
    return () => { current=false; };
  }, [isOpen, siteType]);

  useEffect(() => {
    if (!isOpen || (siteType !== 'node' && siteType !== 'python')) return;
    const kind = siteType;
    let current = true;
    setApplicationRuntime({ kind, versions: [], loading: true, error: '' });
    const request = kind === 'node'
      ? fetchNodeVersions().then(result => result.versions.filter(item => item.is_installed).map(item => item.version))
      : getPythonVersions().then(result => result.filter(item => item.is_installed).map(item => item.version));
    request.then(versions => {
      if (!current) return;
      const installed = [...new Set(versions)];
      const selectVersion = kind === 'node' ? setNodeVersion : setPythonVersion;
      selectVersion(previous => installed.includes(previous) ? previous : installed[0] || '');
      setApplicationRuntime({ kind, versions: installed, loading: false, error: '' });
    }).catch((error: unknown) => {
      if (!current) return;
      (kind === 'node' ? setNodeVersion : setPythonVersion)('');
      setApplicationRuntime({ kind, versions: [], loading: false,
        error: error instanceof Error ? error.message : 'Gagal memuat runtime terpasang.' });
    });
    return () => { current = false; };
  }, [isOpen, siteType, runtimeReload]);

  const isApplication = siteType === 'node' || siteType === 'python';
  const applicationVersions = applicationRuntime?.kind === siteType ? applicationRuntime.versions : [];
  const applicationLoading = applicationRuntime?.kind !== siteType || applicationRuntime.loading;
  const applicationError = applicationRuntime?.kind === siteType ? applicationRuntime.error : '';
  const applicationName = siteType === 'node' ? 'Node.js' : 'Python';
  const applicationReady = !applicationLoading && !applicationError &&
    applicationVersions.includes(siteType === 'node' ? nodeVersion : pythonVersion);

  if (!isOpen) return null;

  const selectedTenant = tenants.find((t) => t.id === selectedTenantId);
  const tenantSlug = selectedTenant?.linux_username || '<tenant>';
  const docrootPreview = `/home/${tenantSlug}/sites/${domain.trim() || '<domain>'}/${siteType === 'php' && phpPreset === 'laravel' ? 'app/public' : 'public'}`;

  const handleNodeFrameworkChange = (preset: string) => {
    setNodeFramework(preset);
    if (preset === 'express') {
      setNodeEntrypointType('script');
      setNodeEntrypoint('server.js');
    } else if (preset === 'nextjs') {
      setNodeEntrypointType('script');
      setNodeEntrypoint('server.js');
    } else {
      setNodeEntrypointType('script');
      setNodeEntrypoint('index.js');
    }
  };

  const handlePythonFrameworkChange = (preset: string) => {
    setPythonFramework(preset);
    if (preset === 'django') {
      setPythonServer('gunicorn_wsgi');
      setPythonEntrypointType('wsgi');
      setPythonEntrypoint('myproject.wsgi:application');
    } else if (preset === 'flask') {
      setPythonServer('gunicorn_wsgi');
      setPythonEntrypointType('wsgi');
      setPythonEntrypoint('app:app');
    } else if (preset === 'fastapi') {
      setPythonServer('uvicorn_asgi');
      setPythonEntrypointType('asgi');
      setPythonEntrypoint('main:app');
    } else {
      setPythonServer('gunicorn_wsgi');
      setPythonEntrypointType('wsgi');
      setPythonEntrypoint('app:app');
    }
  };

  const handleNextStep1 = () => {
    if (isApplication && !applicationReady) {
      setErrorMessage(applicationError || `Pilih runtime ${applicationName} yang sudah terpasang.`); return;
    }
    if (siteType === 'php' && (phpLoading || !phpVersion)) {
      setErrorMessage(phpError || 'Belum ada runtime PHP yang terpasang. Buka Runtime Manager terlebih dahulu.'); return;
    }
    setErrorMessage(null);
    setStep(2);
  };

  const handleNextStep2 = (e: React.FormEvent) => {
    e.preventDefault();
    if (isApplication && !applicationReady) {
      setErrorMessage(applicationError || `Pilih runtime ${applicationName} yang sudah terpasang.`); return;
    }
    const cleanDomain = domain.trim().toLowerCase();
    if (!cleanDomain) {
      setErrorMessage('Nama domain wajib diisi.');
      return;
    }
    // Simple frontend sanity check for domain name format
    if (!cleanDomain.includes('.') || cleanDomain.startsWith('.') || cleanDomain.endsWith('.')) {
      setErrorMessage('Format domain tidak valid (contoh: example.com atau sub.domain.id).');
      return;
    }
    if (siteType === 'proxy' && !proxyUpstream.trim()) {
      setErrorMessage('URL target upstream wajib diisi.');
      return;
    }
    if (siteType === 'php' && (!phpVersions.includes(phpVersion) || (phpPreset === 'custom' && !/^[a-zA-Z0-9_.-]+\.php$/.test(phpEntry)))) {
      setErrorMessage('Pilih runtime PHP terpasang dan front-controller PHP yang valid.'); return;
    }
    setErrorMessage(null);
    setStep(3);
  };

  const handleNextStep3 = () => {
    if (!selectedTenantId) {
      setErrorMessage('Pilih tenant pemilik situs untuk melanjutkan.');
      return;
    }
    setErrorMessage(null);
    setStep(4);
  };

  const handleFinalSubmit = async () => {
    if (isApplication && !applicationReady) {
      setErrorMessage(applicationError || `Pilih runtime ${applicationName} yang sudah terpasang.`); return;
    }
    setIsSubmitting(true);
    setErrorMessage(null);

    const payload: CreateSiteRequest = {
      domain: domain.trim().toLowerCase(),
      user_id: selectedTenantId || undefined,
      spa_fallback: siteType === 'static' ? spaFallback : false,
      force_https: forceHttps,
      ...(siteType === 'php' ? {kind:'php',runtime:'php',php_version:phpVersion,preset:phpPreset,custom_entry_file:phpPreset==='custom' ? phpEntry : undefined} : siteType === 'node'
        ? {
            kind: 'nodejs',
            runtime: 'nodejs',
            node_version: nodeVersion,
            version: nodeVersion,
            framework: nodeFramework,
            entrypoint_value: nodeEntrypoint.trim() || undefined,
            entrypoint_type: nodeEntrypointType,
          }
        : siteType === 'python'
        ? {
            kind: 'python',
            runtime: 'python',
            python_version: pythonVersion,
            version: pythonVersion,
            framework: pythonFramework,
            server_type: pythonServer,
            entrypoint_value: pythonEntrypoint.trim() || undefined,
            entrypoint_type: pythonEntrypointType,
            workers: pythonWorkers,
          }
        : siteType === 'proxy'
        ? {
            kind: 'proxy',
            runtime: 'proxy',
            upstream_url: proxyUpstream.trim(),
            preserve_host: proxyPreserveHost,
            websocket_support: proxyWebsocket,
          }
        : {}),
    };

    try {
      const created = await createSite(payload);
      onSuccess(created);
      onClose();
    } catch (err: unknown) {
      const e = err as Error;
      setErrorMessage(e.message || 'Gagal membuat situs. Silakan periksa konfigurasi domain.');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150"
      onDismiss={onClose}
    >
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] w-full max-w-2xl overflow-hidden flex flex-col max-h-[90vh] dialog-panel">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--line)] bg-[var(--raised)] dialog-header">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-[var(--radius-md)] bg-[var(--primary)]/10 text-[var(--primary)] flex items-center justify-center">
              <Globe className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-base font-semibold text-[var(--ink)]">{t('sites.wizard.title')}</h2>
              <p className="text-xs text-[var(--ink-muted)]">
                Langkah {step} dari 4 &bull; {step === 1 && t('sites.wizard.step1Title')}
                {step === 2 && t('sites.wizard.step2Title')}
                {step === 3 && t('sites.wizard.step3Title')}
                {step === 4 && t('sites.wizard.step4Title')}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Step Progress Indicators */}
        <div className="grid grid-cols-4 border-b border-[var(--line)] bg-[var(--bg)] text-xs font-medium">
          <div
            className={`py-2 px-3 text-center border-b-2 transition-colors ${
              step >= 1
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)]'
            }`}
          >
            1. Tipe
          </div>
          <div
            className={`py-2 px-3 text-center border-b-2 transition-colors ${
              step >= 2
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)]'
            }`}
          >
            2. Domain
          </div>
          <div
            className={`py-2 px-3 text-center border-b-2 transition-colors ${
              step >= 3
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)]'
            }`}
          >
            3. Pemilik
          </div>
          <div
            className={`py-2 px-3 text-center border-b-2 transition-colors ${
              step >= 4
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)]'
            }`}
          >
            4. Tinjau
          </div>
        </div>

        {/* Content Body */}
        <div key={step} className="p-6 overflow-y-auto flex-1 dialog-body dialog-step">
          {errorMessage && (
            <div role="alert" className="mb-5 p-3 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs flex items-start gap-2.5">
              <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
              <span>{errorMessage}</span>
            </div>
          )}

          {/* STEP 1: Pilih Tipe Situs */}
          {step === 1 && (
            <div className="space-y-4">
              <div className="text-xs text-[var(--ink-muted)] mb-3">{t('sites.wizard.step1Desc')}</div>

              <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
                {/* Static Site (Active) */}
                <div
                  onClick={() => setSiteType('static')}
                  className={`cursor-pointer p-4 rounded-[var(--radius-md)] border transition-all ${
                    siteType === 'static'
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 shadow-xs'
                      : 'border-[var(--line)] bg-[var(--raised)] hover:border-[var(--ink-muted)]'
                  }`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <FileCode2 className="w-5 h-5 text-[var(--primary)]" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">
                    {t('sites.wizard.staticTitle')}
                  </h3>
                  <p className="text-xs text-[var(--ink-muted)]">{t('sites.wizard.staticDesc')}</p>
                </div>

                <button
                  type="button"
                  aria-pressed={siteType === 'php'}
                  onClick={() => setSiteType('php')}
                  className={`p-4 text-left rounded-[var(--radius-md)] border ${siteType === 'php' ? 'border-[var(--primary)]' : 'border-[var(--line)]'} bg-[var(--raised)]`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <Server className="w-5 h-5 text-[var(--ink-muted)]" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">{t('sites.wizard.phpTitle')}</h3>
                  <p className="text-xs text-[var(--ink-muted)]">
                    Generic, WordPress, Laravel, atau custom front-controller.
                  </p>
                </button>

                {/* Node.js Application (Available in Phase 7) */}
                <button
                  type="button"
                  aria-pressed={siteType === 'node'}
                  onClick={() => setSiteType('node')}
                  className={`cursor-pointer p-4 text-left rounded-[var(--radius-md)] border transition-all ${
                    siteType === 'node'
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 shadow-xs'
                      : 'border-[var(--line)] bg-[var(--raised)] hover:border-[var(--ink-muted)]'
                  }`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <Terminal className="w-5 h-5 text-[var(--primary)]" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">{t('sites.wizard.nodeTitle')}</h3>
                  <p className="text-xs text-[var(--ink-muted)]">{t('sites.wizard.nodeDesc')}</p>
                </button>

                {/* Python Application (Available in Phase 7 - P7-T3) */}
                <button
                  type="button"
                  aria-pressed={siteType === 'python'}
                  onClick={() => setSiteType('python')}
                  className={`cursor-pointer p-4 text-left rounded-[var(--radius-md)] border transition-all ${
                    siteType === 'python'
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 shadow-xs'
                      : 'border-[var(--line)] bg-[var(--raised)] hover:border-[var(--ink-muted)]'
                  }`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <Terminal className="w-5 h-5 text-[var(--primary)]" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">Aplikasi Python (uv + venv)</h3>
                  <p className="text-xs text-[var(--ink-muted)]">
                    WSGI/ASGI (Django, Flask, FastAPI) via Unix domain socket &amp; isolasi tenant slice.
                  </p>
                </button>

                {/* Reverse Proxy (Available in Phase 7 - P7-T4) */}
                <div
                  onClick={() => setSiteType('proxy')}
                  className={`cursor-pointer p-4 rounded-[var(--radius-md)] border transition-all ${
                    siteType === 'proxy'
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 shadow-xs'
                      : 'border-[var(--line)] bg-[var(--raised)] hover:border-[var(--ink-muted)]'
                  }`}
                >
                  <div className="flex items-center justify-between mb-2">
                    <Network className="w-5 h-5 text-[var(--primary)]" />
                  </div>
                  <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">Reverse Proxy (Nginx)</h3>
                  <p className="text-xs text-[var(--ink-muted)]">
                    Proxy ke backend server (HTTP/HTTPS/Unix socket) dengan proteksi SSRF.
                  </p>
                </div>
              </div>
            </div>
          )}

          {/* STEP 2: Domain & Routing */}
          {isApplication && step <= 2 && (applicationLoading || applicationError || applicationVersions.length === 0) && (
            <div role={applicationError ? 'alert' : 'status'} className="p-3 border border-[var(--line)] rounded text-sm">
              {applicationLoading ? `Memuat runtime ${applicationName}…` : applicationError ||
                `Belum ada runtime ${applicationName} yang terpasang.`}
              {!applicationLoading && (applicationError || applicationVersions.length === 0) && (
                <div className="mt-2 flex flex-wrap gap-3">
                  <button type="button" onClick={() => setRuntimeReload(value => value + 1)} className="underline">Coba lagi</button>
                  {user?.role === 'admin' ? (
                    <RouteLink href="/runtime" onClick={onClose}>Buka Runtime Manager</RouteLink>
                  ) : `Hubungi administrator untuk memasang ${applicationName}.`}
                </div>
              )}
            </div>
          )}
          {siteType === 'php' && step <= 2 && (phpLoading || phpError || phpVersions.length === 0) && (
            <div role={phpError ? 'alert' : 'status'} className="p-3 border border-[var(--line)] rounded text-sm">
              {phpLoading
                ? 'Memuat runtime PHP…'
                : phpError ||
                  'Belum ada runtime PHP yang terpasang.'}
              {!phpLoading && phpVersions.length === 0 && (
                <div className="mt-2">
                  {user?.role === 'admin' ? (
                    <RouteLink href="/runtime" onClick={onClose}>
                      Buka Runtime Manager
                    </RouteLink>
                  ) : (
                    'Hubungi administrator untuk memasang PHP.'
                  )}
                </div>
              )}
            </div>
          )}
          {step === 2 && (
            <form id="step2-form" onSubmit={handleNextStep2} className="space-y-4">
              {siteType === 'php' && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <label>
                    Versi PHP
                    <select
                      aria-label="Versi PHP"
                      value={phpVersion}
                      onChange={(e) => setPhpVersion(e.target.value)}
                      className="block w-full p-2 border border-[var(--line)] bg-[var(--surface)]"
                    >
                      {phpVersions.map((v) => (
                        <option key={v} value={v}>
                          PHP {v}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Preset PHP
                    <select
                      aria-label="Preset PHP"
                      value={phpPreset}
                      onChange={(e) => setPhpPreset(e.target.value as typeof phpPreset)}
                      className="block w-full p-2 border border-[var(--line)] bg-[var(--surface)]"
                    >
                      <option value="generic">Generic PHP</option>
                      <option value="wordpress">WordPress</option>
                      <option value="laravel">Laravel</option>
                      <option value="custom">Custom front-controller</option>
                    </select>
                  </label>
                  {phpPreset === 'custom' && (
                    <label>
                      Front-controller
                      <input
                        aria-label="Front-controller"
                        value={phpEntry}
                        onChange={(e) => setPhpEntry(e.target.value)}
                        required
                        className="block w-full p-2 border border-[var(--line)] bg-[var(--surface)]"
                      />
                    </label>
                  )}
                </div>
              )}
              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1.5">
                  {t('sites.wizard.domainLabel')} <span className="text-[var(--danger)]">*</span>
                </label>
                <div className="relative">
                  <Globe className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                  <input
                    type="text"
                    required
                    placeholder="situsku.com atau demo.perusahaan.id"
                    value={domain}
                    onChange={(e) => setDomain(e.target.value.toLowerCase())}
                    className="w-full pl-9 pr-3 py-2 text-xs font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] focus:ring-1 focus:ring-[var(--primary)]"
                    autoFocus
                  />
                </div>
                <p className="text-[11px] text-[var(--ink-muted)] mt-1.5">{t('sites.wizard.domainHelp')}</p>
              </div>

              {/* Node.js Specific Options */}
              {siteType === 'node' && (
                <div className="pt-2 space-y-3 border-t border-[var(--line)]">
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">Versi Node.js</label>
                      <select
                        aria-label="Versi Node.js"
                        disabled={!applicationReady}
                        value={nodeVersion}
                        onChange={(e) => setNodeVersion(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        {applicationVersions.length === 0 && <option value="">Pilih versi terpasang</option>}
                        {applicationVersions.map(version => <option key={version} value={version}>Node.js {version}</option>)}
                      </select>
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                        Preset Framework
                      </label>
                      <select
                        value={nodeFramework}
                        onChange={(e) => handleNodeFrameworkChange(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        <option value="express">Express (REST API / Web)</option>
                        <option value="nextjs">Next.js (Standalone SSR)</option>
                        <option value="generic">Generic (Kustom Script/npm)</option>
                      </select>
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                        Tipe Entrypoint
                      </label>
                      <select
                        value={nodeEntrypointType}
                        onChange={(e) => setNodeEntrypointType(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        <option value="script">File Script (server.js/app.js)</option>
                        <option value="npm_start">npm start</option>
                        <option value="npm_custom">npm run &lt;custom&gt;</option>
                      </select>
                    </div>
                  </div>

                  {nodeEntrypointType !== 'npm_start' && (
                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                        {nodeEntrypointType === 'script' ? 'Nama File Script' : 'Nama Script npm'}
                      </label>
                      <input
                        type="text"
                        value={nodeEntrypoint}
                        onChange={(e) => setNodeEntrypoint(e.target.value)}
                        placeholder={nodeEntrypointType === 'script' ? 'server.js atau app.js' : 'start:prod'}
                        className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      />
                    </div>
                  )}

                  <div className="p-2.5 rounded-[var(--radius-sm)] bg-[var(--info)]/10 text-[var(--info)] text-[11px]">
                    Port lokal akan dialokasikan otomatis secara unik dalam rentang 20000 - 29999 dan diproxy oleh
                    Nginx.
                  </div>
                </div>
              )}

              {/* Python Specific Options (Phase 7 - P7-T3) */}
              {siteType === 'python' && (
                <div className="pt-2 space-y-3 border-t border-[var(--line)]">
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                        Versi Python (uv)
                      </label>
                      <select
                        aria-label="Versi Python"
                        disabled={!applicationReady}
                        value={pythonVersion}
                        onChange={(e) => setPythonVersion(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        {applicationVersions.length === 0 && <option value="">Pilih versi terpasang</option>}
                        {applicationVersions.map(version => <option key={version} value={version}>Python {version}</option>)}
                      </select>
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                        Preset Framework
                      </label>
                      <select
                        value={pythonFramework}
                        onChange={(e) => handlePythonFrameworkChange(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        <option value="django">Django (WSGI + static/media)</option>
                        <option value="flask">Flask (WSGI + static)</option>
                        <option value="fastapi">FastAPI (ASGI)</option>
                        <option value="generic">Generic (Kustom WSGI/ASGI)</option>
                      </select>
                    </div>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">Tipe Server</label>
                      <select
                        value={pythonServer}
                        onChange={(e) => setPythonServer(e.target.value)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        <option value="gunicorn_wsgi">Gunicorn (WSGI)</option>
                        <option value="uvicorn_asgi">Uvicorn (ASGI)</option>
                        <option value="gunicorn_uvicorn_worker">Gunicorn + Uvicorn Worker</option>
                      </select>
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-[var(--ink)] mb-1">Jumlah Worker</label>
                      <input
                        type="number"
                        min={1}
                        max={16}
                        value={pythonWorkers}
                        onChange={(e) => setPythonWorkers(parseInt(e.target.value, 10) || 1)}
                        className="w-full text-xs px-3 py-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      />
                    </div>
                  </div>

                  <div>
                    <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                      Entrypoint Modul &amp; Callable
                    </label>
                    <input
                      type="text"
                      value={pythonEntrypoint}
                      onChange={(e) => setPythonEntrypoint(e.target.value)}
                      placeholder="contoh: wsgi:application atau app:app atau main:app"
                      className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                    />
                  </div>

                  <div className="p-2.5 rounded-[var(--radius-sm)] bg-[var(--info)]/10 text-[var(--info)] text-[11px]">
                    Situs akan terhubung ke backend via Unix domain socket di /run/panel/python/ dan diproxy oleh
                    Nginx.
                  </div>
                </div>
              )}

              {/* Reverse Proxy Specific Options (Phase 7 - P7-T4) */}
              {siteType === 'proxy' && (
                <div className="pt-2 space-y-3 border-t border-[var(--line)]">
                  <div>
                    <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                      URL Target Upstream <span className="text-[var(--danger)]">*</span>
                    </label>
                    <input
                      type="text"
                      required
                      value={proxyUpstream}
                      onChange={(e) => setProxyUpstream(e.target.value)}
                      placeholder="http://127.0.0.1:3000 atau unix:/run/app.sock"
                      className="w-full text-xs px-3 py-2 font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                    />
                    <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                      Protokol yang didukung: http://, https://, atau unix:/path/ke/socket.sock
                    </p>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1">
                    <label className="flex items-center gap-2 p-2.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={proxyPreserveHost}
                        onChange={(e) => setProxyPreserveHost(e.target.checked)}
                        className="rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)]"
                      />
                      <span className="text-xs text-[var(--ink)]">Preserve Host ($host)</span>
                    </label>

                    <label className="flex items-center gap-2 p-2.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={proxyWebsocket}
                        onChange={(e) => setProxyWebsocket(e.target.checked)}
                        className="rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)]"
                      />
                      <span className="text-xs text-[var(--ink)]">Dukungan WebSocket</span>
                    </label>
                  </div>

                  <div className="p-2.5 rounded-[var(--radius-sm)] bg-[var(--info)]/10 text-[var(--info)] text-[11px]">
                    Proteksi SSRF aktif: Hanya alamat loopback (127.0.0.1, ::1, localhost, unix:) yang diizinkan
                    secara default. Akses IP LAN privat dan cloud metadata diblokir otomatis.
                  </div>
                </div>
              )}

              <div className="pt-2 space-y-3 border-t border-[var(--line)]">
                {siteType === 'static' && (
                  <label className="flex items-start gap-3 p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer hover:bg-[var(--surface)] transition-colors">
                    <input
                      type="checkbox"
                      checked={spaFallback}
                      onChange={(e) => setSpaFallback(e.target.checked)}
                      className="mt-0.5 rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                    />
                    <div>
                      <span className="text-xs font-semibold text-[var(--ink)] block">
                        {t('sites.wizard.spaFallbackLabel')}
                      </span>
                      <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                        {t('sites.wizard.spaFallbackHelp')}
                      </span>
                    </div>
                  </label>
                )}

                <label className="flex items-start gap-3 p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer hover:bg-[var(--surface)] transition-colors">
                  <input
                    type="checkbox"
                    checked={forceHttps}
                    onChange={(e) => setForceHttps(e.target.checked)}
                    className="mt-0.5 rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                  <div>
                    <span className="text-xs font-semibold text-[var(--ink)] block">
                      {t('sites.wizard.forceHttpsLabel')}
                    </span>
                    <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                      {t('sites.wizard.forceHttpsHelp')}
                    </span>
                  </div>
                </label>
              </div>
            </form>
          )}

          {/* STEP 3: Pemilik Tenant */}
          {step === 3 && (
            <div className="space-y-4">
              <div className="text-xs text-[var(--ink-muted)] mb-2">{t('sites.wizard.step3Desc')}</div>

              {isLoadingTenants ? (
                <div className="p-8 text-center text-xs text-[var(--ink-muted)] flex items-center justify-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin text-[var(--primary)]" />
                  Memuat daftar tenant...
                </div>
              ) : tenants.length === 0 ? (
                <div className="p-4 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs">
                  Belum ada tenant aktif. Anda harus membuat minimal 1 tenant terlebih dahulu di halaman Tenant.
                </div>
              ) : (
                <div className="space-y-2 max-h-60 overflow-y-auto pr-1">
                  {tenants.map((tItem) => {
                    const isSelected = tItem.id === selectedTenantId;
                    return (
                      <button
                        type="button"
                        aria-pressed={isSelected}
                        key={tItem.id}
                        onClick={() => setSelectedTenantId(tItem.id)}
                        className={`cursor-pointer p-3 rounded-[var(--radius-md)] border transition-all flex items-center justify-between ${
                          isSelected
                            ? 'border-[var(--primary)] bg-[var(--primary)]/5 ring-1 ring-[var(--primary)]'
                            : 'border-[var(--line)] bg-[var(--raised)] hover:bg-[var(--surface)]'
                        }`}
                      >
                        <div className="flex items-center gap-3">
                          <div
                            className={`w-7 h-7 rounded-full flex items-center justify-center text-xs font-bold ${
                              isSelected
                                ? 'bg-[var(--primary)] text-white'
                                : 'bg-[var(--line)] text-[var(--ink-muted)]'
                            }`}
                          >
                            {tItem.username.slice(0, 2).toUpperCase()}
                          </div>
                          <div>
                            <div className="text-xs font-semibold text-[var(--ink)]">{tItem.username}</div>
                            <div className="text-[11px] font-mono text-[var(--ink-muted)]">
                              uid: {tItem.linux_uid || '-'} &bull; linux: {tItem.linux_username}
                            </div>
                          </div>
                        </div>

                        {isSelected && <CheckCircle2 className="w-4 h-4 text-[var(--primary)] shrink-0" />}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* STEP 4: Tinjau & Konfirmasi */}
          {step === 4 && (
            <div className="space-y-4">
              <div className="text-xs text-[var(--ink-muted)] mb-3">{t('sites.wizard.step4Desc')}</div>

              <div className="rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] divide-y divide-[var(--line)] text-xs">
                <div className="flex items-center justify-between p-3">
                  <span className="text-[var(--ink-muted)]">{t('sites.wizard.reviewDomain')}</span>
                  <span className="font-mono font-semibold text-[var(--ink)]">{domain}</span>
                </div>
                <div className="flex items-center justify-between p-3">
                  <span className="text-[var(--ink-muted)]">{t('sites.wizard.reviewType')}</span>
                  <span className="font-semibold text-[var(--primary)]">
                    {siteType === 'proxy'
                      ? 'Reverse Proxy (Nginx)'
                      : siteType === 'node'
                        ? `Node.js (${nodeFramework.toUpperCase()} • v${nodeVersion})`
                        : siteType === 'python'
                          ? `Python Application (${pythonFramework.toUpperCase()} • v${pythonVersion})`
                          : siteType === 'php'
                            ? 'PHP (Multi-Versi)'
                            : 'Situs Statis (HTML/CSS/JS)'}
                  </span>
                </div>
                {siteType === 'proxy' && (
                  <div className="flex items-center justify-between p-3">
                    <span className="text-[var(--ink-muted)]">Target Upstream:</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">{proxyUpstream}</span>
                  </div>
                )}
                {siteType === 'node' && (
                  <div className="flex items-center justify-between p-3">
                    <span className="text-[var(--ink-muted)]">Entrypoint:</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">
                      {nodeEntrypointType === 'npm_start'
                        ? 'npm start'
                        : `${nodeEntrypointType}: ${nodeEntrypoint}`}
                    </span>
                  </div>
                )}
                {siteType === 'python' && (
                  <div className="flex items-center justify-between p-3">
                    <span className="text-[var(--ink-muted)]">Server &amp; Entrypoint:</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">
                      {pythonServer} &bull; {pythonEntrypoint}
                    </span>
                  </div>
                )}
                <div className="flex items-center justify-between p-3">
                  <span className="text-[var(--ink-muted)]">{t('sites.wizard.reviewTenant')}</span>
                  <span className="font-semibold text-[var(--ink)]">
                    {selectedTenant?.username} ({tenantSlug})
                  </span>
                </div>
                <div className="p-3">
                  <span className="text-[var(--ink-muted)] block mb-1">{t('sites.wizard.reviewDocroot')}</span>
                  <span className="font-mono text-[11px] bg-[var(--bg)] px-2 py-1 rounded block text-[var(--ink)] overflow-x-auto">
                    {docrootPreview}
                  </span>
                </div>
                <div className="flex items-center justify-between p-3">
                  <span className="text-[var(--ink-muted)]">Opsi Routing:</span>
                  <span className="text-[11px] text-[var(--ink)]">
                    {spaFallback ? 'SPA Fallback aktif' : 'SPA standar'} &bull;{' '}
                    {forceHttps ? 'Paksa HTTPS aktif' : 'HTTP/HTTPS fleksibel'}
                  </span>
                </div>
              </div>

              <div className="p-3 rounded-[var(--radius-md)] bg-[var(--info)]/10 border border-[var(--info)]/20 text-xs text-[var(--info)] flex items-start gap-2">
                <ShieldCheck className="w-4 h-4 shrink-0 mt-0.5" />
                <span>
                  Situs akan dibuat dengan vhost Nginx terisolasi, sertifikat self-signed otomatis sebagai
                  fallback, dan izin POSIX ACL www-data yang aman.
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-between px-6 py-4 border-t border-[var(--line)] bg-[var(--raised)] dialog-footer">
          {step > 1 ? (
            <button
              type="button"
              onClick={() => {
                setErrorMessage(null);
                setStep((prev) => (prev - 1) as 1 | 2 | 3 | 4);
              }}
              className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface)] transition-colors flex items-center gap-1.5"
            >
              <ArrowLeft className="w-3.5 h-3.5" />
              Kembali
            </button>
          ) : (
            <button
              type="button"
              onClick={onClose}
              className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
            >
              Batal
            </button>
          )}

          {step === 1 && (
            <button
              type="button"
              onClick={handleNextStep1}
              disabled={isApplication && !applicationReady}
              className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center gap-1.5"
            >
              Lanjutkan
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
          )}

          {step === 2 && (
            <button
              type="submit"
              form="step2-form"
              disabled={isApplication && !applicationReady}
              className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center gap-1.5"
            >
              Lanjutkan
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
          )}

          {step === 3 && (
            <button
              type="button"
              disabled={isLoadingTenants || tenants.length === 0}
              onClick={handleNextStep3}
              className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
            >
              Lanjutkan
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
          )}

          {step === 4 && (
            <button
              type="button"
              disabled={isSubmitting || (isApplication && !applicationReady)}
              onClick={handleFinalSubmit}
              className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
            >
              {isSubmitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              {isSubmitting ? 'Menerapkan ke Nginx...' : t('sites.wizard.submitButton')}
            </button>
          )}
        </div>
      </div>
    </ModalOverlay>
  );
};
