import React, { useState, useEffect, useCallback } from 'react';
import { TerminalTabItem, TerminalSession, TerminalConnectionStatus } from '../../types/terminal';
import { createTerminalSession, closeTerminalSession } from '../../api/client';
import { TerminalTabs } from './TerminalTabs';
import { TerminalInstance } from './TerminalInstance';
import { toast } from 'sonner';
import { TerminalAccessNotice, useTerminalAccess } from './TerminalAccess';
import { FloatingTerminalWindow } from './FloatingTerminalWindow';

interface TerminalDrawerProps {
  tenant?: string;
  isOpen: boolean;
  onClose: () => void;
  initialCwd?: string;
  title?: string;
  openRequest?: number;
}

export const TerminalDrawer: React.FC<TerminalDrawerProps> = ({
  isOpen,
  onClose,
  initialCwd,
  tenant,
  title = 'Terminal berkas',
  openRequest,
}) => {
  const { state: access, refresh: refreshAccess, canOpenTerminal } = useTerminalAccess(tenant, isOpen);
  const initialized = React.useRef(false);
  const open = React.useRef(isOpen);
  open.current = isOpen;
  const liveSessions = React.useRef(new Set<string>());
  const mounted = React.useRef(true);
  useEffect(() => {
    mounted.current = true;
    const sessions = liveSessions.current;
    return () => {
      mounted.current = false;
      sessions.forEach(id => { void closeTerminalSession(id).catch(() => {}); });
      sessions.clear();
    };
  }, []);

  const [tabs, setTabs] = useState<TerminalTabItem[]>([]);
  const [activeTabId, setActiveTabId] = useState<string>('');
  const [isMinimized, setIsMinimized] = useState(false);
  const [isSpawning, setIsSpawning] = useState(false);
  const [sessionError, setSessionError] = useState('');
  useEffect(() => { if (isOpen) setIsMinimized(false); }, [isOpen, openRequest]);

  // Helper to spawn a new session tab
  const spawnTab = useCallback(
    async (cwd?: string) => {
      if (isSpawning || !canOpenTerminal) return;
      setIsSpawning(true);
      setSessionError('');

      const targetCwd = cwd || initialCwd || '';
      const fallbackTitle = targetCwd
        ? targetCwd.split('/').filter(Boolean).pop() || 'Terminal'
        : `Terminal ${tabs.length + 1}`;

      const tempId = `tab-${Date.now()}`;
      const newTab: TerminalTabItem = {
        id: tempId,
        title: fallbackTitle,
        cwd: targetCwd,
        status: 'connecting',
      };

      setTabs((prev) => [...prev, newTab]);
      setActiveTabId(tempId);

      try {
        const session: TerminalSession = await createTerminalSession({
          tenant,
          cwd: targetCwd || undefined,
          cols: 80,
          rows: 24,
        });

        if (!mounted.current || !open.current) { await closeTerminalSession(session.session_id); return; }
        liveSessions.current.add(session.session_id);
        setTabs((prev) =>
          prev.map((t) =>
            t.id === tempId
              ? {
                  ...t,
                  session,
                  title: session.cwd.split('/').filter(Boolean).pop() || fallbackTitle,
                  status: 'connecting',
                }
              : t
          )
        );
      } catch (err: unknown) {
        if (!mounted.current || !open.current) return;
        const errorMsg = err instanceof Error ? err.message : 'Gagal membuka sesi terminal';
        toast.error(errorMsg);
        setSessionError(errorMsg);
        if ((err as { status?: number })?.status === 403) refreshAccess();
        // Remove temporary failed tab
        setTabs((prev) => prev.filter((t) => t.id !== tempId));
      } finally {
        if (mounted.current) setIsSpawning(false);
      }
    },
    [initialCwd, isSpawning, tabs.length, tenant, canOpenTerminal, refreshAccess]
  );

  // Spawn initial tab when drawer opens with no tabs
  useEffect(() => {
    if (!isOpen) {
      initialized.current = false;
      liveSessions.current.forEach(id => { void closeTerminalSession(id).catch(() => {}); });
      liveSessions.current.clear();
      setTabs(previous => previous.length ? [] : previous);
    }
    if (isOpen && canOpenTerminal && !initialized.current && tabs.length === 0 && !isSpawning) {
      initialized.current = true;
      spawnTab(initialCwd);
    }
  }, [isOpen, tabs.length, initialCwd, spawnTab, isSpawning, canOpenTerminal]);

  const handleCloseTab = async (tabId: string) => {
    const tabToClose = tabs.find((t) => t.id === tabId);
    if (tabToClose?.session) {
      try {
        await closeTerminalSession(tabToClose.session.session_id);
        liveSessions.current.delete(tabToClose.session.session_id);
      } catch {
        // ignore close error
      }
    }

    const nextTabs = tabs.filter((t) => t.id !== tabId);
    setTabs(nextTabs);

    if (activeTabId === tabId) {
      if (nextTabs.length > 0) {
        setActiveTabId(nextTabs[nextTabs.length - 1].id);
      } else {
        onClose();
      }
    }
  };

  const handleStatusChange = (tabId: string, status: TerminalConnectionStatus) => {
    setTabs((prev) =>
      prev.map((t) => (t.id === tabId ? { ...t, status } : t))
    );
  };

  if (!isOpen) return null;

  return (
    <FloatingTerminalWindow
      title={title}
      cwd={tabs.find(tab => tab.id === activeTabId)?.cwd || initialCwd || '~'}
      minimized={isMinimized}
      onMinimizedChange={setIsMinimized}
      onClose={onClose}
    >
      {/* Tab bar header */}
      <TerminalTabs
        tabs={tabs}
        activeTabId={activeTabId}
        onSelectTab={setActiveTabId}
        onNewTab={() => spawnTab()}
        newTabDisabled={!canOpenTerminal || isSpawning}
        onCloseTab={handleCloseTab}
        fillWidth
      />

      {/* Terminal Viewports Container */}
      <div className="min-h-0 flex-1 w-full relative overflow-hidden bg-[var(--surface)]">
        {!canOpenTerminal ? <TerminalAccessNotice state={access} onRefresh={refreshAccess} /> : tabs.length === 0 ? <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-sm">
          <p role={sessionError ? 'alert' : undefined}>{sessionError || 'Tidak ada sesi terminal aktif saat ini.'}</p>
          <button type="button" onClick={() => spawnTab()} disabled={isSpawning} className="rounded border border-[var(--line)] px-3 py-2">Buka sesi terminal</button>
        </div> : tabs.map((tab) => {
          if (!tab.session) {
            return (
              <div
                key={tab.id}
                className={`h-full w-full flex flex-col items-center justify-center text-xs text-[var(--ink-muted)] ${
                  tab.id === activeTabId ? 'flex' : 'hidden'
                }`}
              >
                <div className="animate-spin h-5 w-5 border-2 border-[var(--primary)] border-t-transparent rounded-full mb-2" />
                <span>Memulai sesi shell tenant...</span>
              </div>
            );
          }

          return (
            <TerminalInstance
              key={tab.session.session_id}
              session={tab.session}
              isActive={!isMinimized && tab.id === activeTabId}
              onStatusChange={(st) => handleStatusChange(tab.id, st)}
            />
          );
        })}
      </div>
    </FloatingTerminalWindow>
  );
};
