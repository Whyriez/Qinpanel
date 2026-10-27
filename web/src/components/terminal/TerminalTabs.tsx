import React from 'react';
import { TerminalTabItem } from '../../types/terminal';
import { Plus, X, Terminal, Search, RotateCw, Trash2, Maximize2, Minimize2 } from 'lucide-react';

interface TerminalTabsProps {
  fillWidth?: boolean;
  newTabDisabled?: boolean;
  tabs: TerminalTabItem[];
  activeTabId: string;
  onSelectTab: (id: string) => void;
  onNewTab: () => void;
  onCloseTab: (id: string) => void;
  onClearActiveTab?: () => void;
  onSearchToggle?: () => void;
  onReconnectActiveTab?: () => void;
  isMaximized?: boolean;
  onToggleMaximize?: () => void;
  onCloseAll?: () => void;
}

export const TerminalTabs: React.FC<TerminalTabsProps> = ({
  tabs,
  activeTabId,
  onSelectTab,
  onNewTab,
  onCloseTab,
  onClearActiveTab,
  onSearchToggle,
  onReconnectActiveTab,
  isMaximized,
  onToggleMaximize,
  onCloseAll,
  newTabDisabled = false,
  fillWidth = false,
}) => {
  return (
    <div className="flex items-center justify-between border-b border-[var(--line)] bg-[var(--surface-hover)] px-2 py-1 select-none text-xs">
      {/* Tab List */}
      <div className={`flex items-center gap-1 overflow-x-auto no-scrollbar ${fillWidth ? 'min-w-0 flex-1' : 'max-w-[calc(100%-180px)]'}`}>
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const statusDotColor =
            tab.status === 'connected'
              ? 'bg-emerald-500'
              : tab.status === 'connecting' || tab.status === 'reconnecting'
              ? 'bg-amber-500 animate-pulse'
              : 'bg-rose-500';

          return (
            <div
              key={tab.id}
              onClick={() => onSelectTab(tab.id)}
              className={`group flex items-center gap-1.5 px-3 py-1.5 rounded-t-[var(--radius-md)] cursor-pointer text-xs font-mono transition-all border-b-2 ${
                isActive
                  ? 'bg-[var(--surface)] text-[var(--ink)] font-semibold border-[var(--primary)] shadow-sm'
                  : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]/50 border-transparent'
              }`}
            >
              {/* Status indicator dot */}
              <span
                className={`h-2 w-2 rounded-full ${statusDotColor}`}
                title={`Status: ${tab.status}`}
              />
              <Terminal className="h-3 w-3 opacity-70" />
              <span className="truncate max-w-[130px]">{tab.title}</span>
              {tabs.length > 1 && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(tab.id);
                  }}
                  className="p-0.5 rounded opacity-0 group-hover:opacity-100 hover:bg-[var(--surface-hover)] text-[var(--ink-muted)] hover:text-[var(--danger)] transition-opacity"
                  title="Tutup tab"
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          );
        })}

        {/* New Tab Button */}
        <button
          type="button"
          onClick={onNewTab}
          disabled={newTabDisabled}
          className="flex items-center gap-1 px-2 py-1.5 rounded-[var(--radius-md)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)] transition-colors cursor-pointer text-xs disabled:opacity-50 disabled:cursor-not-allowed"
          title="Buka tab terminal baru"
        >
          <Plus className="h-3.5 w-3.5" />
          <span className="text-[11px] font-sans">Tab Baru</span>
        </button>
      </div>

      {/* Toolbar Quick Actions */}
      <div className="flex items-center gap-1 text-[var(--ink-muted)]">
        {onSearchToggle && (
          <button
            type="button"
            onClick={onSearchToggle}
            className="p-1.5 rounded hover:bg-[var(--surface)] hover:text-[var(--ink)] transition-colors"
            title="Cari teks (Ctrl+F)"
          >
            <Search className="h-3.5 w-3.5" />
          </button>
        )}
        {onReconnectActiveTab && (
          <button
            type="button"
            onClick={onReconnectActiveTab}
            className="p-1.5 rounded hover:bg-[var(--surface)] hover:text-[var(--ink)] transition-colors"
            title="Sambung ulang sesi"
          >
            <RotateCw className="h-3.5 w-3.5" />
          </button>
        )}
        {onClearActiveTab && (
          <button
            type="button"
            onClick={onClearActiveTab}
            className="p-1.5 rounded hover:bg-[var(--surface)] hover:text-[var(--ink)] transition-colors"
            title="Bersihkan layar (Ctrl+L)"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        )}
        {onToggleMaximize && (
          <button
            type="button"
            onClick={onToggleMaximize}
            className="p-1.5 rounded hover:bg-[var(--surface)] hover:text-[var(--ink)] transition-colors"
            title={isMaximized ? 'Perkecil jendela' : 'Perbesar layar penuh'}
          >
            {isMaximized ? (
              <Minimize2 className="h-3.5 w-3.5" />
            ) : (
              <Maximize2 className="h-3.5 w-3.5" />
            )}
          </button>
        )}
        {onCloseAll && (
          <button
            type="button"
            onClick={onCloseAll}
            className="p-1.5 rounded hover:bg-[var(--danger)]/10 hover:text-[var(--danger)] transition-colors ml-1"
            title="Tutup terminal"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
    </div>
  );
};
