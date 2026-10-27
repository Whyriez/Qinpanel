import { panelWebSocketUrl } from '../../lib/panelUrl';
import React, { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { SearchAddon } from '@xterm/addon-search';
import '@xterm/xterm/css/xterm.css';
import { TerminalConnectionStatus, TerminalSession } from '../../types/terminal';
import { Search, X, ChevronUp, ChevronDown } from 'lucide-react';

interface TerminalInstanceProps {
  session: TerminalSession;
  isActive: boolean;
  onStatusChange?: (status: TerminalConnectionStatus) => void;
  onExit?: (code?: number) => void;
}

export const TerminalInstance: React.FC<TerminalInstanceProps> = ({
  session,
  isActive,
  onStatusChange,
  onExit,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

  const [status, setStatus] = useState<TerminalConnectionStatus>('connecting');
  const [showSearch, setShowSearch] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');

  // Notify parent on status update
  const updateStatus = (newStatus: TerminalConnectionStatus) => {
    setStatus(newStatus);
    onStatusChange?.(newStatus);
  };

  useEffect(() => {
    if (!containerRef.current) return;

    // Detect dark mode
    const isDark = document.documentElement.classList.contains('dark');

    // Qinpanel design tokens for terminal
    const termTheme = isDark
      ? {
          background: '#09191F',
          foreground: '#E4EEF0',
          cursor: '#3FC1C9',
          cursorAccent: '#09191F',
          selectionBackground: 'rgba(63, 193, 201, 0.3)',
          black: '#0F252D',
          red: '#EF6B6B',
          green: '#4CC58A',
          yellow: '#F0B455',
          blue: '#6FA3EE',
          magenta: '#C084FC',
          cyan: '#3FC1C9',
          white: '#E4EEF0',
          brightBlack: '#51656E',
          brightRed: '#F87171',
          brightGreen: '#34D399',
          brightYellow: '#FBBF24',
          brightBlue: '#60A5FA',
          brightMagenta: '#E879F9',
          brightCyan: '#22D3EE',
          brightWhite: '#FFFFFF',
        }
      : {
          background: '#FAFCFC',
          foreground: '#0C1E26',
          cursor: '#0B7A85',
          cursorAccent: '#FAFCFC',
          selectionBackground: 'rgba(11, 122, 133, 0.25)',
          black: '#0C1E26',
          red: '#C23B3B',
          green: '#2F9E62',
          yellow: '#D9962B',
          blue: '#3B76C9',
          magenta: '#9333EA',
          cyan: '#0B7A85',
          white: '#EDF1F2',
          brightBlack: '#8CA5AD',
          brightRed: '#DC2626',
          brightGreen: '#16A34A',
          brightYellow: '#D97706',
          brightBlue: '#2563EB',
          brightMagenta: '#C026D3',
          brightCyan: '#0891B2',
          brightWhite: '#FFFFFF',
        };

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: 'block',
      fontFamily: "'JetBrains Mono', monospace",
      fontSize: 13,
      lineHeight: 1.35,
      letterSpacing: 0,
      theme: termTheme,
      allowTransparency: true,
      scrollback: 5000,
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();
    const searchAddon = new SearchAddon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.loadAddon(searchAddon);

    term.open(containerRef.current);

    termRef.current = term;
    fitAddonRef.current = fitAddon;
    searchAddonRef.current = searchAddon;

    try {
      fitAddon.fit();
    } catch {
      // Ignore initial fit error if not rendered yet
    }

    // Connect WebSocket
    const wsUrl = panelWebSocketUrl(`/ws/terminal/${encodeURIComponent(session.session_id)}`);
    updateStatus('connecting');

    const ws = new WebSocket(wsUrl);
    ws.binaryType = 'arraybuffer';
    wsRef.current = ws;

    ws.onopen = () => {
      updateStatus('connected');
      try {
        fitAddon.fit();
        const { cols, rows } = term;
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      } catch {
        // ignore fit error
      }
    };

    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        // Raw binary stream from PTY
        const uint8 = new Uint8Array(event.data);
        term.write(uint8);
      } else if (typeof event.data === 'string') {
        try {
          const parsed = JSON.parse(event.data);
          if (parsed.type === 'connected') {
            updateStatus('connected');
          } else if (parsed.type === 'closed') {
            updateStatus('disconnected');
            term.write('\r\n\x1b[33m[Proses terminal telah selesai]\x1b[0m\r\n');
            onExit?.(parsed.exit_code);
          } else if (parsed.type === 'pong') {
            // Heartbeat response OK
          } else {
            term.write(event.data);
          }
        } catch {
          // Plain string message
          term.write(event.data);
        }
      }
    };

    ws.onerror = () => {
      updateStatus('disconnected');
    };

    ws.onclose = () => {
      updateStatus('disconnected');
    };

    // Forward terminal keyboard input to WebSocket
    const onDataDisposable = term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    });

    // Forward window dimension changes
    const onResizeDisposable = term.onResize(({ cols, rows }) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    // Setup periodic ping keep-alive
    const pingInterval = setInterval(() => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'ping' }));
      }
    }, 20000);

    // Resize observer to keep xterm fitted to container
    const resizeObserver = new ResizeObserver(() => {
      if (containerRef.current && containerRef.current.clientHeight > 0) {
        try {
          fitAddon.fit();
        } catch {
          // ignore
        }
      }
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      clearInterval(pingInterval);
      resizeObserver.disconnect();
      onDataDisposable.dispose();
      onResizeDisposable.dispose();
      ws.close();
      term.dispose();
      termRef.current = null;
      wsRef.current = null;
      fitAddonRef.current = null;
      searchAddonRef.current = null;
    };
  }, [session.session_id]);

  // Refit when tab becomes active
  useEffect(() => {
    if (isActive && fitAddonRef.current && termRef.current) {
      setTimeout(() => {
        try {
          fitAddonRef.current?.fit();
          termRef.current?.focus();
        } catch {
          // ignore
        }
      }, 50);
    }
  }, [isActive]);

  const handleReconnect = () => {
    if (!termRef.current) return;
    updateStatus('reconnecting');
    const wsUrl = panelWebSocketUrl(`/ws/terminal/${encodeURIComponent(session.session_id)}`);
    const ws = new WebSocket(wsUrl);
    ws.binaryType = 'arraybuffer';
    wsRef.current = ws;

    ws.onopen = () => {
      updateStatus('connected');
      if (fitAddonRef.current && termRef.current) {
        fitAddonRef.current.fit();
        const { cols, rows } = termRef.current;
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    };

    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        termRef.current?.write(new Uint8Array(event.data));
      } else if (typeof event.data === 'string') {
        termRef.current?.write(event.data);
      }
    };

    ws.onerror = () => updateStatus('disconnected');
    ws.onclose = () => updateStatus('disconnected');
  };

  const handleSearchNext = () => {
    if (searchQuery && searchAddonRef.current) {
      searchAddonRef.current.findNext(searchQuery, { incremental: true });
    }
  };

  const handleSearchPrev = () => {
    if (searchQuery && searchAddonRef.current) {
      searchAddonRef.current.findPrevious(searchQuery);
    }
  };

  return (
    <div
      className={`relative h-full w-full flex flex-col bg-[var(--surface)] ${
        isActive ? 'block' : 'hidden'
      }`}
    >
      {/* Floating Search Bar */}
      {showSearch && (
        <div className="absolute top-2 right-4 z-20 flex items-center gap-1.5 px-3 py-1.5 rounded-[var(--radius-md)] bg-[var(--surface-hover)] border border-[var(--line)] shadow-lg text-xs">
          <Search className="h-3.5 w-3.5 text-[var(--ink-muted)]" />
          <input
            type="text"
            placeholder="Cari teks di terminal..."
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              searchAddonRef.current?.findNext(e.target.value, { incremental: true });
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                if (e.shiftKey) handleSearchPrev();
                else handleSearchNext();
              } else if (e.key === 'Escape') {
                setShowSearch(false);
                termRef.current?.focus();
              }
            }}
            autoFocus
            className="w-48 bg-transparent text-[var(--ink)] placeholder-[var(--ink-muted)] outline-none text-xs"
          />
          <button
            type="button"
            onClick={handleSearchPrev}
            className="p-1 hover:bg-[var(--surface)] rounded text-[var(--ink-muted)] hover:text-[var(--ink)]"
            title="Sebelumnya (Shift+Enter)"
          >
            <ChevronUp className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={handleSearchNext}
            className="p-1 hover:bg-[var(--surface)] rounded text-[var(--ink-muted)] hover:text-[var(--ink)]"
            title="Berikutnya (Enter)"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            onClick={() => {
              setShowSearch(false);
              termRef.current?.focus();
            }}
            className="p-1 hover:bg-[var(--surface)] rounded text-[var(--ink-muted)] hover:text-[var(--ink)]"
            title="Tutup (Esc)"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {/* Disconnect Warning Banner */}
      {status === 'disconnected' && (
        <div className="px-3 py-1.5 bg-amber-500/10 border-b border-amber-500/20 text-amber-600 dark:text-amber-400 text-xs flex items-center justify-between">
          <span>Koneksi terminal terputus. Sesi PTY mungkin telah selesai atau koneksi jaringan goyang.</span>
          <button
            type="button"
            onClick={handleReconnect}
            className="px-2 py-0.5 rounded bg-amber-500/20 hover:bg-amber-500/30 text-xs font-medium cursor-pointer"
          >
            Sambung Ulang
          </button>
        </div>
      )}

      {/* Terminal Container */}
      <div
        ref={containerRef}
        className="flex-1 w-full overflow-hidden p-2 font-mono"
        onClick={() => termRef.current?.focus()}
      />
    </div>
  );
};
