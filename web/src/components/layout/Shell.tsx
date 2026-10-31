import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LayoutDashboard,
  Globe,
  Mail,
  Users,
  BarChart3,
  Cpu,
  ListTodo,
  ShieldCheck,
  UserCheck,
  Settings,
  Search,
  Sun,
  Moon,
  Monitor,
  Menu,
  LogOut,
  Layers,
  FolderTree,
  HardDrive,
  Terminal,
  PanelLeftClose,
  PanelLeftOpen,
  X,
} from "lucide-react";
import { ThemeMode } from "../../types/api";
import { useAuth } from "../../context/AuthContext";
import { JobTray } from "./JobTray";
import { CommandPalette } from "../ui/CommandPalette";
import { ImpersonationBanner } from "./ImpersonationBanner";
import { NotificationBell } from "../notifications/NotificationBell";
import { RouteLink, pagePath } from "../../lib/router";
import { Brand } from "../ui/Brand";
import { useActiveJobCounts } from "./useActiveJobCounts";

export interface ShellProps {
  children: React.ReactNode;
  activeNav?: string;
  onNavigate?: (nav: string) => void;
  theme: ThemeMode;
  onThemeChange: (theme: ThemeMode) => void;
}

export const Shell: React.FC<ShellProps> = ({
  children,
  activeNav = "dashboard",
  onNavigate = () => {},
  theme,
  onThemeChange,
}) => {
  const { t, i18n } = useTranslation();
  const { user, logout } = useAuth();
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const sidebarRef = useRef<HTMLElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const admin = user?.role === "admin";
  const navLabel = (id: string) => t(id === 'jobs' && !admin ? 'nav.activity' : id === 'connectivity' && !admin ? 'sites.tabs.connections' : `nav.${id}`, { defaultValue: t('notFound.title') });
  const activeJobs = useActiveJobCounts(admin ? user.id : null);
  const hasActiveJobs = Boolean(activeJobs && activeJobs.total > 0);
  const jobCountDescription = activeJobs
    ? i18n.language.startsWith('en')
      ? `${activeJobs.total} active tasks: ${activeJobs.queued} queued, ${activeJobs.running} running`
      : `${activeJobs.total} tugas aktif: ${activeJobs.queued} antre, ${activeJobs.running} berjalan`
    : '';
  useEffect(() => {
    const page = pageRef.current;
    const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    if (!page?.animate || preference.matches) return;
    const animation = page.animate(
      [{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }],
      { duration: 200, easing: 'cubic-bezier(.2,.8,.2,1)' },
    );
    const cancelForReducedMotion = () => { if (preference.matches) animation.cancel(); };
    preference.addEventListener('change', cancelForReducedMotion);
    return () => {
      animation.cancel();
      preference.removeEventListener('change', cancelForReducedMotion);
    };
  }, [activeNav]);
  const groups = [
    {
      label: t("design.hosting"),
      items: [
        { id: "dashboard", icon: LayoutDashboard },
        { id: "sites", icon: Globe },
        { id: "email", icon: Mail },
        { id: "files", icon: FolderTree },
        { id: "storage", icon: HardDrive },
        { id: "terminal", icon: Terminal },
        { id: "analytics", icon: BarChart3 },
      ],
    },
    {
      label: t("design.management"),
      items: [
        ...(admin
          ? [
              { id: "tenants", icon: Users },
              { id: "plans", icon: Layers },
              { id: "runtime", icon: Cpu },
            ]
          : []),
        { id: "jobs", icon: ListTodo },
        ...(admin
          ? [
              { id: "audit", icon: ShieldCheck },
              { id: "settings", icon: Settings },
            ]
          : []),
      ],
    },
  ];
  const navigate = (id: string) => {
    onNavigate(id);
    setMobileOpen(false);
  };
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMobileOpen(false);
    };
    window.addEventListener("keydown", close);
    const media = window.matchMedia("(min-width: 768px)");
    const resize = () => {
      if (media.matches) setMobileOpen(false);
    };
    media.addEventListener("change", resize);
    return () => {
      window.removeEventListener("keydown", close);
      media.removeEventListener("change", resize);
    };
  }, []);
  useEffect(() => {
    if (!mobileOpen) return;
    const controls = () =>
      Array.from(
        sidebarRef.current?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), a[href]",
        ) || [],
      ).filter(
        (button) =>
          button.getClientRects().length > 0 &&
          window.getComputedStyle(button).visibility === "visible",
      );
    let focusFrame: number;
    const focusWhenVisible = () => {
      const first = controls()[0];
      if (first) first.focus();
      else focusFrame = window.requestAnimationFrame(focusWhenVisible);
    };
    focusFrame = window.requestAnimationFrame(focusWhenVisible);
    const trapFocus = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const buttons = controls();
      const first = buttons[0];
      const last = buttons.at(-1);
      if (!sidebarRef.current?.contains(document.activeElement)) {
        event.preventDefault();
        (event.shiftKey ? last : first)?.focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    window.addEventListener("keydown", trapFocus);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      window.removeEventListener("keydown", trapFocus);
      menuButtonRef.current?.focus();
    };
  }, [mobileOpen]);
  const cycleTheme = () =>
    onThemeChange(
      theme === "light" ? "dark" : theme === "dark" ? "system" : "light",
    );
  const ThemeIcon = theme === "light" ? Sun : theme === "dark" ? Moon : Monitor;
  const initial = user?.username.slice(0, 2).toUpperCase() || "N";

  return (
    <div
      className={`panel-shell ${collapsed ? "is-collapsed" : ""} ${mobileOpen ? "is-mobile-open" : ""}`}
    >
      <a className="skip-link" href="#panel-main">
        {t("design.skipContent")}
      </a>
      {mobileOpen && (
        <button
          className="sidebar-backdrop"
          aria-label={t("design.closeMenu")}
          onClick={() => setMobileOpen(false)}
        />
      )}
      <aside
        ref={sidebarRef}
        role={mobileOpen ? "dialog" : undefined}
        aria-modal={mobileOpen || undefined}
        className="panel-sidebar"
        id="panel-navigation"
        aria-label={t("design.mainNavigation")}
      >
        <div className="sidebar-brand">
          <Brand />
          <button
            className="mobile-close icon-button"
            aria-label={t("design.closeMenu")}
            onClick={() => setMobileOpen(false)}
          >
            <X size={19} />
          </button>
        </div>
        <nav
          className="sidebar-navigation"
          aria-label={t("design.mainNavigation")}
        >
          {groups.map((group) => (
            <div className="nav-group" key={group.label}>
              <p className="nav-group-label">{group.label}</p>
              {group.items.map(({ id, icon: Icon }) => (
                <RouteLink
                  href={pagePath(id)}
                  key={id}
                  className={`nav-link ${id === activeNav ? "is-active" : ""} ${id === 'jobs' && hasActiveJobs ? 'has-job-count' : ''}`}
                  aria-current={id === activeNav ? "page" : undefined}
                  aria-label={navLabel(id)}
                  aria-describedby={id === 'jobs' && hasActiveJobs ? 'sidebar-active-jobs' : undefined}
                  title={id === 'jobs' && hasActiveJobs ? jobCountDescription : navLabel(id)}
                  onClick={() => navigate(id)}
                >
                  <Icon size={18} strokeWidth={1.7} />
                  <span className="nav-label">{navLabel(id)}</span>
                  {id === 'jobs' && hasActiveJobs && activeJobs && <>
                    <span className="nav-job-count" aria-hidden="true">{activeJobs.total > 99 ? '99+' : activeJobs.total}</span>
                    <span id="sidebar-active-jobs" className="sr-only">{jobCountDescription}</span>
                  </>}
                </RouteLink>
              ))}
            </div>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <button
            className={`nav-link ${activeNav === "profile" ? "is-active" : ""}`}
            onClick={() => navigate("profile")}
            aria-label={t("nav.profile")}
            title={t("nav.profile")}
          >
            <UserCheck size={18} strokeWidth={1.7} />
            <span className="nav-label">{t("nav.profile")}</span>
          </button>
          <div className="sidebar-preferences">
            <button
              className="icon-button"
              onClick={cycleTheme}
              aria-label={t("design.changeTheme")}
              title={`${t("design.changeTheme")}: ${theme}`}
            >
              <ThemeIcon size={17} />
            </button>
            <button
              className="language-button"
              onClick={() =>
                i18n.changeLanguage(
                  i18n.language.startsWith("id") ? "en" : "id",
                )
              }
              aria-label={t("design.changeLanguage")}
            >
              {i18n.language.startsWith("id") ? "ID" : "EN"}
            </button>
            <button
              className="desktop-collapse icon-button"
              onClick={() => setCollapsed(!collapsed)}
              aria-label={
                collapsed ? t("design.expandNav") : t("design.collapseNav")
              }
              title={
                collapsed ? t("design.expandNav") : t("design.collapseNav")
              }
            >
              {collapsed ? (
                <PanelLeftOpen size={17} />
              ) : (
                <PanelLeftClose size={17} />
              )}
            </button>
          </div>
        </div>
      </aside>
      <div className="panel-workspace" inert={mobileOpen || undefined}>
        {user?.impersonated_by && (
          <ImpersonationBanner
            impersonatedBy={user.impersonated_by}
            username={user.username}
          />
        )}
        <header className="panel-topbar">
          <button
            className="mobile-menu icon-button"
            ref={menuButtonRef}
            aria-label={t("design.openMenu")}
            aria-expanded={mobileOpen}
            aria-controls="panel-navigation"
            onClick={() => setMobileOpen(!mobileOpen)}
          >
            <Menu size={20} />
          </button>
          <div className="workspace-breadcrumb">
            <span>Qinpanel</span>
            <span aria-hidden="true">/</span>
            <strong>{navLabel(activeNav)}</strong>
          </div>
          <button
            className="workspace-search"
            onClick={() => setPaletteOpen(true)}
            aria-label={t("common.search")}
          >
            <Search size={16} />
            <span>{t("common.search")}</span>
            <kbd>Ctrl K</kbd>
          </button>
          <div className="topbar-actions">
            <NotificationBell
              userRole={user?.role}
              onNavigateUrl={(url) => {
                const segment = url.replace(/^\//, "").split("/")[0];
                if (segment) navigate(segment);
              }}
            />
            <button
              className="account-button"
              onClick={() => navigate("profile")}
              aria-label={t("nav.profile")}
            >
              <span className="account-avatar">{initial}</span>
              <span className="account-name">{user?.username}</span>
            </button>
            <button
              className="icon-button logout-button"
              onClick={logout}
              aria-label={t("auth.logout")}
              title={t("auth.logout")}
            >
              <LogOut size={17} />
            </button>
          </div>
        </header>
        <main className="panel-content" id="panel-main" tabIndex={-1}>
          <div className="panel-page" ref={pageRef}>{children}</div>
        </main>
        <nav
          className="mobile-navigation"
          aria-label={t("design.quickNavigation")}
        >
          {groups[0].items
            .filter((item) => ["dashboard", "sites", "files", "storage"].includes(item.id))
            .map(({ id, icon: Icon }) => (
              <RouteLink
                href={pagePath(id)}
                key={id}
                className={activeNav === id ? "is-active" : ""}
                aria-current={activeNav === id ? "page" : undefined}
                onClick={() => navigate(id)}
              >
                <Icon size={19} />
                <span>{navLabel(id)}</span>
              </RouteLink>
            ))}
          <button
            onClick={() => setMobileOpen(true)}
            aria-label={t("design.openMenu")}
          >
            <Menu size={19} />
            <span>{t("design.more")}</span>
          </button>
        </nav>
        <JobTray onNavigateToJobs={() => navigate("jobs")} />
        <CommandPalette
          activeNav={activeNav}
          isOpen={paletteOpen}
          onOpen={() => setPaletteOpen(true)}
          onClose={() => setPaletteOpen(false)}
          onNavigate={navigate}
          theme={theme}
          onThemeChange={onThemeChange}
        />
      </div>
    </div>
  );
};
