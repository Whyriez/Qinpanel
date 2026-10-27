import React, { useEffect, useState, Suspense, lazy } from 'react';
import { Shell } from './components/layout/Shell';
import { DashboardPage } from './pages/DashboardPage';
import { LoginPage } from './pages/LoginPage';
import { ToastProvider } from './components/ui/Toast';
import { ThemeMode } from './types/api';
import { AuthProvider, useAuth } from './context/AuthContext';
import './i18n';
import { navigate, pagePath, useBrowserRoute } from './lib/router';
import { isAdminOnlyRoute, postLoginDestination } from './lib/routeAccess';
import { parseSiteRoute, sitePath } from './lib/siteRoutes';
import { NotFoundPage } from './pages/NotFoundPage';
import { BrandLoading } from './components/ui/Brand';
import { useTranslation } from 'react-i18next';

const FirstRunWizardPage = lazy(() =>
  import('./pages/FirstRunWizardPage').then((m) => ({ default: m.FirstRunWizardPage }))
);
const SecurityProfilePage = lazy(() =>
  import('./pages/SecurityProfilePage').then((m) => ({ default: m.SecurityProfilePage }))
);
const AuditLogPage = lazy(() =>
  import('./pages/AuditLogPage').then((m) => ({ default: m.AuditLogPage }))
);
const EmailPage = lazy(() => import('./pages/EmailPage').then(m => ({ default: m.EmailPage })));
const JobsPage = lazy(() =>
  import('./pages/JobsPage').then((m) => ({ default: m.JobsPage }))
);
const TenantsPage = lazy(() =>
  import('./pages/TenantsPage').then((m) => ({ default: m.TenantsPage }))
);
const TenantDetailPage = lazy(() =>
  import('./pages/TenantDetailPage').then((m) => ({ default: m.TenantDetailPage }))
);
const PlansPage = lazy(() =>
  import('./pages/PlansPage').then((m) => ({ default: m.PlansPage }))
);
const SitesPage = lazy(() =>
  import('./pages/SitesPage').then((m) => ({ default: m.SitesPage }))
);
const SiteDetailRoute = lazy(() =>
  import('./pages/SiteDetailRoute').then((m) => ({ default: m.SiteDetailRoute }))
);
const RuntimePage = lazy(() =>
  import('./pages/RuntimePage').then((m) => ({ default: m.RuntimePage }))
);
const FileManagerPage = lazy(() =>
  import('./pages/FileManagerPage').then((m) => ({ default: m.FileManagerPage }))
);
const TerminalPage = lazy(() =>
  import('./pages/TerminalPage').then((m) => ({ default: m.TerminalPage }))
);
const AnalyticsPage = lazy(() =>
  import('./pages/AnalyticsPage').then((m) => ({ default: m.AnalyticsPage }))
);
const SettingsPage = lazy(() =>
  import('./pages/SettingsPage').then((m) => ({ default: m.SettingsPage }))
);
const StoragePage = lazy(() => import('./pages/StoragePage').then(m => ({ default: m.StoragePage })));
const ConnectivityPage = lazy(() => import('./pages/ConnectivityPage').then(m => ({ default: m.ConnectivityPage })));

const PageFallback: React.FC = () => {
  const { i18n } = useTranslation();
  return <BrandLoading message={i18n.language.startsWith('en') ? 'Loading page…' : 'Memuat halaman…'} />;
};

const MainApp: React.FC = () => {
  const { i18n } = useTranslation();
  const { isAuthenticated, isLoading, user } = useAuth();
  const [theme, setTheme] = useState<ThemeMode>(() => {
    return (localStorage.getItem('nusa_theme') as ThemeMode) || 'system';
  });
  const route = useBrowserRoute();
  const tenantMatch = /^\/tenants\/(\d+)(?:\/.*)?$/.exec(route.pathname);
  const siteMatch = parseSiteRoute(route.pathname);
  const activeNav = tenantMatch ? 'tenant-detail' : siteMatch ? 'site-detail' : route.pathname.slice(1);
  const selectedTenantId = tenantMatch ? Number(tenantMatch[1]) : null;
  const isWizardOpen = route.pathname === '/setup';
  const setActiveNav = (page: string) => navigate(pagePath(page));
  const loginPath = `/login?returnTo=${encodeURIComponent(route.search.get('returnTo') || '/dashboard')}`;
  useEffect(() => {
    if (isLoading) return;
    if (route.pathname === '/') { navigate('/dashboard', true); return; }
    if (!isAuthenticated && !['/login', '/setup'].includes(route.pathname)) {
      navigate(`/login?returnTo=${encodeURIComponent(route.url)}`, true);
    } else if (isAuthenticated && ['/login', '/setup'].includes(route.pathname)) {
      navigate(postLoginDestination(route.search.get('returnTo'), user?.role), true);
    }
  }, [isLoading, isAuthenticated, user?.role, route.url, route.pathname, route.search]);

  useEffect(() => {
    localStorage.setItem('nusa_theme', theme);

    const root = document.documentElement;
    const isDark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);

    if (isDark) {
      root.classList.add('dark');
    } else {
      root.classList.remove('dark');
    }

    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleSystemChange = (e: MediaQueryListEvent) => {
      if (theme === 'system') {
        if (e.matches) root.classList.add('dark');
        else root.classList.remove('dark');
      }
    };
    mediaQuery.addEventListener('change', handleSystemChange);
    return () => mediaQuery.removeEventListener('change', handleSystemChange);
  }, [theme]);

  // Loading skeleton while verifying session
  if (isLoading) {
    return <BrandLoading fullScreen message={i18n.language.startsWith('en') ? 'Checking your session…' : 'Memeriksa sesi Anda…'} />;
  }

  // First-run wizard view
  if (isWizardOpen) {
    return (
      <Suspense fallback={<PageFallback />}>
        <FirstRunWizardPage
          onFinish={() => navigate(loginPath)}
          onBackToLogin={() => navigate(loginPath)}
        />
      </Suspense>
    );
  }

  // Not authenticated: render Login Page
  if (!isAuthenticated) {
    return <LoginPage onOpenWizard={() => navigate(`/setup?${route.search.toString()}`)} />;
  }

  if (user?.role !== 'admin' && isAdminOnlyRoute(route.pathname)) {
    return <Shell activeNav={activeNav} onNavigate={setActiveNav} theme={theme} onThemeChange={setTheme}><div role="alert">Anda tidak memiliki akses ke halaman ini. <a href="/dashboard">Buka dashboard</a></div></Shell>;
  }

  // Authenticated: render Shell and active page
  return (
    <>
      <Shell
        activeNav={
          activeNav === 'tenant-detail'
            ? 'tenants'
            : activeNav === 'site-detail'
            ? 'sites'
            : activeNav
        }
        onNavigate={setActiveNav}
        theme={theme}
        onThemeChange={setTheme}
      >
        {activeNav === 'dashboard' && <DashboardPage onNavigate={setActiveNav} onSelectSite={(site) => navigate(sitePath(site))} />}
        <Suspense fallback={<PageFallback />}>
          {activeNav === 'audit' && <AuditLogPage />}
          {activeNav === 'jobs' && <JobsPage />}
          {activeNav === 'email' && <EmailPage />}
          {activeNav === 'storage' && <StoragePage />}
          {activeNav === 'profile' && <SecurityProfilePage />}
          {activeNav === 'sites' && (
            <SitesPage
              onSelectSite={(site) => {
                navigate(sitePath(site));
              }}
            />
          )}
          {activeNav === 'site-detail' && siteMatch && (
            <SiteDetailRoute
              siteKey={siteMatch.key}
              onBack={() => {
                navigate('/sites');
              }}
            />
          )}
          {(activeNav === 'tenants' || activeNav === 'users') && (
            <TenantsPage
              onSelectTenant={(id) => {
                navigate(`/tenants/${id}`);
              }}
              onNavigateToPlans={() => setActiveNav('plans')}
            />
          )}
          {activeNav === 'tenant-detail' && selectedTenantId && (
            <TenantDetailPage
              tenantId={selectedTenantId}
              onBack={() => {
                navigate('/tenants');
              }}
            />
          )}
          {activeNav === 'plans' && <PlansPage />}
          {activeNav === 'runtime' && <RuntimePage />}
          {activeNav === 'files' && <FileManagerPage />}
          {activeNav === 'terminal' && <TerminalPage />}
          {activeNav === 'analytics' && <AnalyticsPage />}
          {activeNav === 'settings' && <SettingsPage />}
          {activeNav === 'connectivity' && <ConnectivityPage />}
          {!['dashboard','audit','email','jobs','profile','sites','site-detail','tenants','tenant-detail','users','plans','runtime','files','storage','terminal','analytics','settings','connectivity','login','setup'].includes(activeNav) && <NotFoundPage />}
        </Suspense>
      </Shell>
      <ToastProvider />
    </>
  );
};

export const App: React.FC = () => {
  return (
    <AuthProvider>
      <MainApp />
    </AuthProvider>
  );
};

export default App;
