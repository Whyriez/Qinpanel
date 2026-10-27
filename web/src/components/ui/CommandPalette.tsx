import { ModalOverlay } from './ModalOverlay';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Command } from 'cmdk';
import { useTranslation } from 'react-i18next';
import {
  ArrowDown, ArrowUp, BarChart3, Check, CornerDownLeft, Cpu, FolderTree,
  HardDrive, Globe, Mail, Languages, Layers, LayoutDashboard, ListTodo, LogOut, Monitor,
  Moon, Search, Settings, ShieldCheck, Sun, Terminal, UserCheck, Users, X,
} from 'lucide-react';
import { ThemeMode } from '../../types/api';
import { useAuth } from '../../context/AuthContext';
import { isAdminOnlyRoute } from '../../lib/routeAccess';
import { filterMenuCommands } from './commandFilter';

const navigation = [
  { id: 'dashboard', icon: LayoutDashboard },
  { id: 'sites', icon: Globe },
  { id: 'email', icon: Mail },
  { id: 'files', icon: FolderTree },
  { id: 'storage', icon: HardDrive },
  { id: 'terminal', icon: Terminal },
  { id: 'connectivity', icon: Globe },
  { id: 'analytics', icon: BarChart3 },
  { id: 'jobs', icon: ListTodo },
  { id: 'profile', icon: UserCheck },
  { id: 'tenants', icon: Users },
  { id: 'plans', icon: Layers },
  { id: 'runtime', icon: Cpu },
  { id: 'audit', icon: ShieldCheck },
  { id: 'settings', icon: Settings },
];
const themes = [
  { id: 'light', icon: Sun },
  { id: 'dark', icon: Moon },
  { id: 'system', icon: Monitor },
] as const;

export interface CommandPaletteProps {
  isOpen?: boolean;
  onClose?: () => void;
  onOpen?: () => void;
  onNavigate: (nav: string) => void;
  activeNav?: string;
  theme: ThemeMode;
  onThemeChange: (theme: ThemeMode) => void;
}

export function CommandPalette({ isOpen: controlledOpen, onClose, onOpen, onNavigate, activeNav, theme, onThemeChange }: CommandPaletteProps) {
  const { t, i18n } = useTranslation();
  const { logout, user } = useAuth();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const descriptionId = useId();
  const isControlled = controlledOpen !== undefined;
  const isVisible = isControlled ? controlledOpen : open;
  const isAdmin = user?.role === 'admin';
  const nextLanguage = i18n.language.startsWith('id') ? 'en' : 'id';
  const clearSearch = () => { setSearch(''); inputRef.current?.focus(); };

  const handleClose = useCallback(() => {
    setSearch('');
    onClose?.();
    if (!isControlled) setOpen(false);
  }, [onClose, isControlled]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== 'k' || event.repeat) return;
      event.preventDefault();
      if (isVisible) handleClose();
      else if (isControlled) onOpen?.();
      else setOpen(true);
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isVisible, isControlled, handleClose, onOpen]);

  if (!isVisible) return null;

  const selectPage = (page: string) => { onNavigate(page); handleClose(); };
  const renderNavigation = (admin: boolean) => navigation
    .filter(item => isAdminOnlyRoute(`/${item.id}`) === admin)
    .map(({ id, icon: Icon }) => (
      <Command.Item
        key={id}
        value={`page-${id}`}
        keywords={[t(`nav.${id}`), t(`palette.descriptions.${id}`), id]}
        onSelect={() => selectPage(id)}
        className="command-item"
      >
        <Icon className="command-item-icon" aria-hidden="true" />
        <span className="command-item-copy">
          <span className="command-item-title">{t(`nav.${id}`)}</span>
          <span className="command-item-description">{t(`palette.descriptions.${id}`)}</span>
        </span>
        {activeNav === id && <span className="command-item-meta"><Check size={13} aria-hidden="true" />{t('palette.currentPage')}</span>}
        <CornerDownLeft className="command-item-enter" aria-hidden="true" />
      </Command.Item>
    ));

  return (
    <ModalOverlay className="command-overlay" onClick={handleClose} onDismiss={handleClose} aria-label={t('palette.title')} aria-describedby={descriptionId}>
      <div className="dialog-panel command-palette" onClick={event => event.stopPropagation()}>
        <Command label={t('palette.title')} filter={filterMenuCommands} loop className="dialog-form">
          <div className="command-caption">
            <div>
              <h2>{t('palette.title')}</h2>
              <p id={descriptionId}>{t('palette.subtitle')}</p>
            </div>
            <button type="button" className="command-control" onClick={handleClose} aria-label={t('palette.close')} title={t('palette.close')}>
              <X size={18} aria-hidden="true" />
            </button>
          </div>
          <div className="command-search-row">
            <Search size={20} aria-hidden="true" />
            <Command.Input
              ref={inputRef}
              autoFocus
              value={search}
              onValueChange={setSearch}
              placeholder={t('palette.placeholder')}
              aria-label={t('palette.placeholder')}
              autoComplete="off"
              spellCheck={false}
            />
            {search && <button type="button" className="command-control" aria-label={t('palette.clear')} onClick={clearSearch}><X size={16} aria-hidden="true" /></button>}
          </div>
          <Command.List className="command-results" label={t('palette.results')}>
            <Command.Empty className="command-empty">
              <Search size={24} aria-hidden="true" />
              <strong>{t('palette.noResults')}</strong>
              <p>{t('palette.noResultsHint', { query: search })}</p>
              <button type="button" className="command-reset" onClick={clearSearch}>{t('palette.clear')}</button>
            </Command.Empty>
            <Command.Group heading={t('palette.navigation')} className="command-group">
              {renderNavigation(false)}
            </Command.Group>
            {isAdmin && <Command.Group heading={t('palette.administration')} className="command-group">{renderNavigation(true)}</Command.Group>}
            <Command.Group heading={t('palette.preferences')} className="command-group">
              {themes.map(({ id, icon: Icon }) => (
                <Command.Item key={id} value={`theme-${id}`} keywords={[t('palette.theme', { theme: t(`common.theme.${id}`) }), id]} onSelect={() => { onThemeChange(id); handleClose(); }} className="command-item">
                  <Icon className="command-item-icon" aria-hidden="true" />
                  <span className="command-item-title">{t('palette.theme', { theme: t(`common.theme.${id}`) })}</span>
                  {theme === id && <span className="command-item-meta command-active"><Check size={14} aria-hidden="true" />{t('palette.active')}</span>}
                  <CornerDownLeft className="command-item-enter" aria-hidden="true" />
                </Command.Item>
              ))}
              <Command.Item value="language" keywords={[t('palette.language'), 'language', 'English', 'Indonesia']} onSelect={() => { void i18n.changeLanguage(nextLanguage); handleClose(); }} className="command-item">
                <Languages className="command-item-icon" aria-hidden="true" />
                <span className="command-item-title">{t('palette.language')}: {nextLanguage === 'en' ? 'English' : 'Indonesia'}</span>
                <CornerDownLeft className="command-item-enter" aria-hidden="true" />
              </Command.Item>
            </Command.Group>
            <Command.Group heading={t('palette.account')} className="command-group">
              <Command.Item value="logout" keywords={[t('auth.logout')]} onSelect={() => { handleClose(); void logout(); }} className="command-item command-item-danger">
                <LogOut className="command-item-icon" aria-hidden="true" />
                <span className="command-item-title">{t('auth.logout')}</span>
                <CornerDownLeft className="command-item-enter" aria-hidden="true" />
              </Command.Item>
            </Command.Group>
          </Command.List>
          <div className="command-footer">
            <span><kbd><ArrowUp size={12} aria-hidden="true" /><ArrowDown size={12} aria-hidden="true" /></kbd>{t('palette.move')}</span>
            <span><kbd><CornerDownLeft size={13} aria-hidden="true" /></kbd>{t('palette.select')}</span>
            <span className="command-footer-close"><kbd>Esc</kbd>{t('palette.dismiss')}</span>
          </div>
        </Command>
      </div>
    </ModalOverlay>
  );
}
