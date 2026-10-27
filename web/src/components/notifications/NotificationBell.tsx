import React, { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Bell,
  Check,
  CheckCheck,
  Trash2,
  Settings,
  ShieldAlert,
  HardDrive,
  AlertTriangle,
  RefreshCw,
  GitPullRequest,
  ShieldCheck,
  ExternalLink,
  Sparkles,
} from 'lucide-react';
import {
  fetchNotifications,
  fetchUnreadNotificationCount,
  markNotificationRead,
  markAllNotificationsRead,
  deleteNotification,
  clearNotifications,
  evaluateNotificationAlerts,
} from '../../api/client';
import { NotificationItem, NotificationCategory, NotificationSeverity } from '../../types/api';
import { NotificationSettingsModal } from './NotificationSettingsModal';

interface NotificationBellProps {
  userRole?: string;
  onNavigateUrl?: (url: string) => void;
}

export const NotificationBell: React.FC<NotificationBellProps> = ({
  userRole = 'user',
  onNavigateUrl,
}) => {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<'all' | 'unread'>('all');
  const [unreadCount, setUnreadCount] = useState<number>(0);
  const [notifications, setNotifications] = useState<NotificationItem[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [isEvaluating, setIsEvaluating] = useState<boolean>(false);

  const containerRef = useRef<HTMLDivElement>(null);

  // Poll unread count on mount & every 30s
  useEffect(() => {
    loadUnreadCount();
    const interval = setInterval(() => {
      loadUnreadCount();
    }, 30000);
    return () => clearInterval(interval);
  }, []);

  // Fetch notifications whenever popover opens or active tab changes
  useEffect(() => {
    if (isOpen) {
      loadNotifications();
    }
  }, [isOpen, activeTab]);

  // Click outside and escape key handling
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        setIsOpen(false);
      }
    };

    if (isOpen) {
      document.addEventListener('mousedown', handleClickOutside);
      document.addEventListener('keydown', handleKeyDown);
    }
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [isOpen]);

  const loadUnreadCount = async () => {
    try {
      const res = await fetchUnreadNotificationCount();
      setUnreadCount(res.unread_count);
    } catch {
      // Ignore background fetch error
    }
  };

  const loadNotifications = async () => {
    setIsLoading(true);
    try {
      const res = await fetchNotifications({
        page: 1,
        per_page: 30,
        unread_only: activeTab === 'unread',
      });
      setNotifications(res.items);
      setUnreadCount(res.unread_count);
    } catch {
      // Ignore error
    } finally {
      setIsLoading(false);
    }
  };

  const handleToggle = () => {
    setIsOpen(!isOpen);
  };

  const handleMarkRead = async (id: string, e?: React.MouseEvent) => {
    if (e) e.stopPropagation();
    try {
      await markNotificationRead(id);
      setNotifications((prev) =>
        prev.map((n) =>
          n.id === id ? { ...n, is_read: true, read_at: new Date().toISOString() } : n
        )
      );
      setUnreadCount((prev) => Math.max(0, prev - 1));
    } catch {
      // Ignore error
    }
  };

  const handleItemClick = async (item: NotificationItem) => {
    if (!item.is_read) {
      await handleMarkRead(item.id);
    }
    if (item.action_url) {
      setIsOpen(false);
      if (onNavigateUrl) {
        onNavigateUrl(item.action_url);
      } else {
        window.location.hash = item.action_url;
      }
    }
  };

  const handleMarkAllRead = async () => {
    try {
      await markAllNotificationsRead();
      setNotifications((prev) =>
        prev.map((n) => ({ ...n, is_read: true, read_at: new Date().toISOString() }))
      );
      setUnreadCount(0);
    } catch {
      // Ignore error
    }
  };

  const handleDelete = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const itemToDelete = notifications.find((n) => n.id === id);
      await deleteNotification(id);
      setNotifications((prev) => prev.filter((n) => n.id !== id));
      if (itemToDelete && !itemToDelete.is_read) {
        setUnreadCount((prev) => Math.max(0, prev - 1));
      }
    } catch {
      // Ignore error
    }
  };

  const handleClearRead = async () => {
    try {
      await clearNotifications(true);
      setNotifications((prev) => prev.filter((n) => !n.is_read));
    } catch {
      // Ignore error
    }
  };

  const handleEvaluateNow = async () => {
    setIsEvaluating(true);
    try {
      await evaluateNotificationAlerts();
      await loadNotifications();
      await loadUnreadCount();
    } catch {
      // Ignore error
    } finally {
      setIsEvaluating(false);
    }
  };

  // Helper formatting relative time in Indonesian / locale
  const formatTimeAgo = (dateStr: string) => {
    try {
      const date = new Date(dateStr);
      const now = new Date();
      const diffSec = Math.floor((now.getTime() - date.getTime()) / 1000);

      if (diffSec < 60) return 'Baru saja';
      const diffMin = Math.floor(diffSec / 60);
      if (diffMin < 60) return `${diffMin} mnt lalu`;
      const diffHour = Math.floor(diffMin / 60);
      if (diffHour < 24) return `${diffHour} jam lalu`;
      const diffDay = Math.floor(diffHour / 24);
      if (diffDay < 30) return `${diffDay} hari lalu`;
      return date.toLocaleDateString();
    } catch {
      return dateStr;
    }
  };

  // Category Icon & Color
  const renderCategoryIcon = (category: NotificationCategory, severity: NotificationSeverity) => {
    const isCritical = severity === 'critical';

    switch (category) {
      case 'ssl_expiring':
        return (
          <div
            className={`w-7 h-7 rounded-[var(--radius-sm)] flex items-center justify-center shrink-0 ${
              isCritical
                ? 'bg-[var(--danger)]/15 text-[var(--danger)]'
                : 'bg-[var(--accent)]/15 text-[var(--accent)]'
            }`}
          >
            {isCritical ? <ShieldAlert className="w-4 h-4" /> : <ShieldCheck className="w-4 h-4" />}
          </div>
        );
      case 'quota_warning':
        return (
          <div
            className={`w-7 h-7 rounded-[var(--radius-sm)] flex items-center justify-center shrink-0 ${
              isCritical
                ? 'bg-[var(--danger)]/15 text-[var(--danger)]'
                : 'bg-[var(--accent)]/15 text-[var(--accent)]'
            }`}
          >
            <HardDrive className="w-4 h-4" />
          </div>
        );
      case 'job_failed':
      case 'backup_failed':
        return (
          <div className="w-7 h-7 rounded-[var(--radius-sm)] bg-[var(--danger)]/15 text-[var(--danger)] flex items-center justify-center shrink-0">
            <AlertTriangle className="w-4 h-4" />
          </div>
        );
      case 'deploy_failed':
        return (
          <div className="w-7 h-7 rounded-[var(--radius-sm)] bg-[var(--danger)]/15 text-[var(--danger)] flex items-center justify-center shrink-0">
            <GitPullRequest className="w-4 h-4" />
          </div>
        );
      case 'security_alert':
        return (
          <div className="w-7 h-7 rounded-[var(--radius-sm)] bg-[var(--danger)]/15 text-[var(--danger)] flex items-center justify-center shrink-0">
            <ShieldAlert className="w-4 h-4" />
          </div>
        );
      default:
        return (
          <div className="w-7 h-7 rounded-[var(--radius-sm)] bg-[var(--primary)]/15 text-[var(--primary)] flex items-center justify-center shrink-0">
            <Bell className="w-4 h-4" />
          </div>
        );
    }
  };

  const isAdmin = userRole === 'admin';

  return (
    <div ref={containerRef} className="relative inline-block">
      {/* Bell Trigger Button */}
      <button
        type="button"
        onClick={handleToggle}
        aria-label="Pusat notifikasi"
        aria-expanded={isOpen}
        className={`p-1.5 rounded-[var(--radius-sm)] transition-colors relative cursor-pointer ${
          isOpen
            ? 'bg-[var(--raised)] text-[var(--primary)]'
            : 'text-[var(--ink-muted)] hover:bg-[var(--raised)] hover:text-[var(--ink)]'
        }`}
        title={t('notifications.title')}
      >
        <Bell className="w-4 h-4" />
        {unreadCount > 0 && (
          <span
            data-testid="unread-badge"
            className="absolute -top-1 -right-1 min-w-[16px] h-4 px-1 rounded-full bg-[var(--danger)] text-white text-[10px] font-bold flex items-center justify-center shadow-xs animate-in zoom-in-50 duration-150"
          >
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>

      {/* Popover Card */}
      {isOpen && (
        <div
          data-testid="notification-popover"
          className="absolute right-0 mt-2 w-80 sm:w-96 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-2xl z-50 flex flex-col max-h-[520px] overflow-hidden animate-in fade-in slide-in-from-top-2 duration-150"
        >
          {/* Popover Header */}
          <div className="px-4 py-3 border-b border-[var(--line)] flex items-center justify-between bg-[var(--surface)]">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold text-[var(--ink)]">
                {t('notifications.title')}
              </span>
              {unreadCount > 0 && (
                <span className="px-1.5 py-0.2 rounded-full text-[10px] font-semibold bg-[var(--primary)]/10 text-[var(--primary)]">
                  {unreadCount}
                </span>
              )}
            </div>

            <div className="flex items-center gap-1">
              {unreadCount > 0 && (
                <button
                  type="button"
                  onClick={handleMarkAllRead}
                  className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--primary)] hover:bg-[var(--raised)] cursor-pointer text-xs"
                  title={t('notifications.markAllRead')}
                >
                  <CheckCheck className="w-3.5 h-3.5" />
                </button>
              )}

              {isAdmin && (
                <button
                  type="button"
                  onClick={() => {
                    setIsSettingsOpen(true);
                    setIsOpen(false);
                  }}
                  className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer"
                  title={t('notifications.settings')}
                >
                  <Settings className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          </div>

          {/* Filter Tabs */}
          <div className="flex border-b border-[var(--line)] bg-[var(--surface-sunken)]/30 px-3">
            <button
              type="button"
              onClick={() => setActiveTab('all')}
              className={`py-2 px-3 text-xs font-medium border-b-2 transition-colors cursor-pointer ${
                activeTab === 'all'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              {t('notifications.all')}
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('unread')}
              className={`py-2 px-3 text-xs font-medium border-b-2 transition-colors cursor-pointer flex items-center gap-1.5 ${
                activeTab === 'unread'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              <span>{t('notifications.unread')}</span>
              {unreadCount > 0 && (
                <span className="w-1.5 h-1.5 rounded-full bg-[var(--danger)]" />
              )}
            </button>
          </div>

          {/* Items List */}
          <div className="flex-1 overflow-y-auto divide-y divide-[var(--line)]">
            {isLoading ? (
              <div className="py-10 flex flex-col items-center justify-center gap-2 text-[var(--ink-muted)]">
                <RefreshCw className="w-5 h-5 animate-spin text-[var(--primary)]" />
                <span className="text-xs">{t('common.loading')}</span>
              </div>
            ) : notifications.length === 0 ? (
              <div className="py-10 px-4 text-center text-[var(--ink-muted)] space-y-2">
                <div className="w-10 h-10 mx-auto rounded-full bg-[var(--raised)] flex items-center justify-center text-[var(--ink-muted)]">
                  <Sparkles className="w-5 h-5 opacity-60" />
                </div>
                <p className="text-xs font-medium text-[var(--ink)]">
                  {activeTab === 'unread'
                    ? t('notifications.unreadEmpty')
                    : t('notifications.empty')}
                </p>
                <p className="text-[11px] max-w-xs mx-auto text-[var(--ink-muted)]">
                  {t('notifications.emptyDesc')}
                </p>
              </div>
            ) : (
              notifications.map((item) => (
                <div
                  key={item.id}
                  onClick={() => handleItemClick(item)}
                  data-testid={`notification-item-${item.id}`}
                  className={`p-3 flex items-start gap-3 transition-colors cursor-pointer group ${
                    item.is_read
                      ? 'bg-[var(--surface)] hover:bg-[var(--raised)]/60'
                      : 'bg-[var(--primary)]/5 hover:bg-[var(--primary)]/10'
                  }`}
                >
                  {/* Category icon */}
                  {renderCategoryIcon(item.category, item.severity)}

                  {/* Body */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-baseline justify-between gap-2">
                      <h4
                        className={`text-xs truncate ${
                          item.is_read
                            ? 'font-medium text-[var(--ink)]'
                            : 'font-semibold text-[var(--ink)]'
                        }`}
                      >
                        {item.title}
                      </h4>
                      <span className="text-[10px] font-mono text-[var(--ink-muted)] shrink-0">
                        {formatTimeAgo(item.created_at)}
                      </span>
                    </div>

                    <p className="text-[11px] text-[var(--ink-muted)] mt-0.5 line-clamp-2 leading-relaxed">
                      {item.message}
                    </p>

                    {item.action_url && (
                      <div className="flex items-center gap-1 mt-1 text-[10px] text-[var(--primary)] font-medium">
                        <span>Lihat detail</span>
                        <ExternalLink className="w-2.5 h-2.5" />
                      </div>
                    )}
                  </div>

                  {/* Quick item actions */}
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                    {!item.is_read && (
                      <button
                        type="button"
                        onClick={(e) => handleMarkRead(item.id, e)}
                        className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--success)] hover:bg-[var(--surface)] cursor-pointer"
                        title={t('notifications.markRead')}
                      >
                        <Check className="w-3.5 h-3.5" />
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={(e) => handleDelete(item.id, e)}
                      className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--danger)] hover:bg-[var(--surface)] cursor-pointer"
                      title={t('notifications.delete')}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>

          {/* Popover Footer */}
          <div className="px-3 py-2 border-t border-[var(--line)] bg-[var(--surface-sunken)]/40 flex items-center justify-between text-xs">
            <button
              type="button"
              onClick={handleClearRead}
              className="text-[11px] text-[var(--ink-muted)] hover:text-[var(--danger)] transition-colors cursor-pointer"
            >
              {t('notifications.clearRead')}
            </button>

            {isAdmin && (
              <button
                type="button"
                onClick={handleEvaluateNow}
                disabled={isEvaluating}
                className="flex items-center gap-1 text-[11px] text-[var(--primary)] hover:underline cursor-pointer disabled:opacity-50"
              >
                <RefreshCw
                  className={`w-3 h-3 ${isEvaluating ? 'animate-spin' : ''}`}
                />
                <span>
                  {isEvaluating
                    ? t('notifications.evaluating')
                    : t('notifications.evaluateNow')}
                </span>
              </button>
            )}
          </div>
        </div>
      )}

      {/* Settings Modal */}
      {isSettingsOpen && (
        <NotificationSettingsModal
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          onSaved={() => {
            loadNotifications();
            loadUnreadCount();
          }}
        />
      )}
    </div>
  );
};
