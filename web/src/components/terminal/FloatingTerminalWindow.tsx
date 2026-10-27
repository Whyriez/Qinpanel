import { useEffect, useId, useRef, useState, type ReactNode, type PointerEvent, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Maximize2, Minimize2, Minus, Move, RotateCcw, Terminal, X } from 'lucide-react';
import { clampTerminalBounds, initialTerminalBounds, WINDOW_MARGIN, type ViewportSize } from './floatingBounds';

function viewportSize(): ViewportSize {
  return typeof window === 'undefined' ? { width: 1280, height: 800 } : { width: window.innerWidth, height: window.innerHeight };
}

interface FloatingTerminalWindowProps {
  title: string;
  cwd?: string;
  minimized: boolean;
  onMinimizedChange: (minimized: boolean) => void;
  onClose: () => void;
  children: ReactNode;
}

/** Nonmodal window, used only by the file manager's terminal drawer. */
export function FloatingTerminalWindow({ title, cwd, minimized, onMinimizedChange, onClose, children }: FloatingTerminalWindowProps) {
  const [viewport, setViewport] = useState(viewportSize);
  const [bounds, setBounds] = useState(() => initialTerminalBounds(viewportSize()));
  const [maximized, setMaximized] = useState(false);
  const [positionControls, setPositionControls] = useState(false);
  const drag = useRef<{ pointer: number; offsetX: number; offsetY: number } | null>(null);
  const titleId = useId();
  const helpId = useId();
  const moveButton = useRef<HTMLButtonElement>(null);
  const titleButton = useRef<HTMLButtonElement>(null);
  const firstPositionButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const resize = () => {
      const next = viewportSize();
      setViewport(next);
      setBounds(previous => clampTerminalBounds(previous, next));
    };
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, []);
  useEffect(() => { if (positionControls) firstPositionButton.current?.focus(); }, [positionControls]);
  useEffect(() => { if (minimized) titleButton.current?.focus(); }, [minimized]);

  const move = (x: number, y: number) => setBounds(previous => clampTerminalBounds({ ...previous, x: previous.x + x, y: previous.y + y }, viewport));
  const startDrag = (event: PointerEvent<HTMLButtonElement>) => {
    if (minimized || maximized || event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, offsetX: event.clientX - bounds.x, offsetY: event.clientY - bounds.y };
    setPositionControls(false);
  };
  const moveDrag = (event: PointerEvent<HTMLButtonElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    setBounds(previous => clampTerminalBounds({ ...previous, x: event.clientX - current.offsetX, y: event.clientY - current.offsetY }, viewport));
  };
  const stopDrag = () => { drag.current = null; };
  const moveWithKeys = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (minimized || maximized) return;
    const amount = event.shiftKey ? 40 : 10;
    const offsets: Record<string, [number, number]> = { ArrowLeft: [-amount, 0], ArrowRight: [amount, 0], ArrowUp: [0, -amount], ArrowDown: [0, amount] };
    if (offsets[event.key]) { event.preventDefault(); event.stopPropagation(); move(...offsets[event.key]); }
  };
  const toggleMaximize = () => {
    stopDrag();
    setPositionControls(false);
    onMinimizedChange(false);
    setMaximized(previous => minimized ? true : !previous);
  };
  const shown = minimized
    ? { x: Math.max(WINDOW_MARGIN, viewport.width - 344), y: Math.max(WINDOW_MARGIN, viewport.height - 108), width: Math.min(320, viewport.width - WINDOW_MARGIN * 2), height: 44 }
    : maximized
      ? { x: WINDOW_MARGIN, y: WINDOW_MARGIN, width: viewport.width - WINDOW_MARGIN * 2, height: viewport.height - WINDOW_MARGIN * 2 }
      : bounds;
  const controlClass = 'flex h-8 w-8 shrink-0 items-center justify-center rounded text-[var(--ink-muted)] hover:bg-[var(--raised)] hover:text-[var(--ink)] focus-visible:outline-2 focus-visible:outline-[var(--primary)]';
  const frame = (
    <section
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      data-files-terminal="true"
      data-window-state={minimized ? 'minimized' : maximized ? 'maximized' : 'floating'}
      className="fixed z-40 flex flex-col overflow-hidden rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] shadow-[var(--shadow-floating)]"
      style={{ left: shown.x, top: shown.y, width: shown.width, height: shown.height }}
    >
      <header className="flex h-11 shrink-0 items-center gap-1 border-b border-[var(--line)] px-2">
        <button
          ref={titleButton}
          type="button"
          aria-label={minimized ? 'Pulihkan terminal' : 'Geser terminal'}
          aria-describedby={minimized ? undefined : helpId}
          disabled={maximized && !minimized}
          title={minimized ? 'Pulihkan terminal' : 'Tarik untuk menggeser; gunakan tombol panah untuk mengatur posisi'}
          className={`flex min-w-0 flex-1 items-center gap-2 rounded px-2 py-1 text-left focus-visible:outline-2 focus-visible:outline-[var(--primary)] ${minimized ? 'cursor-pointer' : maximized ? '' : 'cursor-grab active:cursor-grabbing'}`}
          style={{ touchAction: 'none' }}
          onPointerDown={startDrag}
          onPointerMove={moveDrag}
          onPointerUp={stopDrag}
          onPointerCancel={stopDrag}
          onLostPointerCapture={stopDrag}
          onKeyDown={moveWithKeys}
          onClick={() => { if (minimized) onMinimizedChange(false); }}
        >
          <Terminal className="h-4 w-4 shrink-0 text-[var(--primary)]" />
          <span className="min-w-0 truncate text-xs font-medium" id={titleId} title={cwd ? `${title} · ${cwd}` : title}>{title}{cwd && <span className="ml-2 font-mono font-normal text-[var(--ink-muted)]">{cwd}</span>}</span>
        </button>
        {!minimized && !maximized && <button ref={moveButton} type="button" aria-label="Atur posisi terminal" title="Atur posisi terminal" aria-expanded={positionControls} onClick={() => setPositionControls(previous => !previous)} className={controlClass}><Move className="h-3.5 w-3.5" /></button>}
        {!minimized && <button type="button" aria-label="Minimalkan terminal" title="Minimalkan terminal" className={controlClass} onClick={() => { stopDrag(); setPositionControls(false); onMinimizedChange(true); }}><Minus className="h-4 w-4" /></button>}
        <button type="button" aria-label={maximized && !minimized ? 'Pulihkan ukuran terminal' : 'Maksimalkan terminal'} title={maximized && !minimized ? 'Pulihkan ukuran terminal' : 'Maksimalkan terminal'} className={controlClass} onClick={toggleMaximize}>{maximized && !minimized ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}</button>
        <button type="button" aria-label="Tutup terminal berkas" title="Tutup terminal berkas" onClick={onClose} className={`${controlClass} hover:text-[var(--danger)]`}><X className="h-4 w-4" /></button>
      </header>
      <span id={helpId} className="sr-only">Geser dengan tombol panah, atau Shift dan panah untuk langkah lebih besar. Tombol Atur posisi terminal menyediakan kontrol klik.</span>
      <div hidden={minimized} className="min-h-0 flex-1 flex-col" style={{ display: minimized ? 'none' : 'flex' }}>
        {positionControls && <div role="toolbar" aria-label="Posisi terminal" className="flex shrink-0 items-center gap-1 border-b border-[var(--line)] px-3 py-1" onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setPositionControls(false); moveButton.current?.focus(); } }}>
          <span className="mr-auto text-xs text-[var(--ink-muted)]">Posisi jendela</span>
          <button ref={firstPositionButton} type="button" aria-label="Geser ke kiri" className={controlClass} onClick={() => move(-40, 0)}><ArrowLeft className="h-4 w-4" /></button>
          <button type="button" aria-label="Geser ke atas" className={controlClass} onClick={() => move(0, -40)}><ArrowUp className="h-4 w-4" /></button>
          <button type="button" aria-label="Geser ke bawah" className={controlClass} onClick={() => move(0, 40)}><ArrowDown className="h-4 w-4" /></button>
          <button type="button" aria-label="Geser ke kanan" className={controlClass} onClick={() => move(40, 0)}><ArrowRight className="h-4 w-4" /></button>
          <button type="button" aria-label="Atur ulang posisi" className={controlClass} onClick={() => setBounds(initialTerminalBounds(viewport))}><RotateCcw className="h-3.5 w-3.5" /></button>
        </div>}
        {children}
      </div>
    </section>
  );
  return typeof document === 'undefined' ? frame : createPortal(frame, document.body);
}
