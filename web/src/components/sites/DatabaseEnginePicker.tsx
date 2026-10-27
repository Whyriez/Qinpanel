import { Loader2, RefreshCw, Server } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { AvailableDatabaseEngine } from '../../types/api';

export function DatabaseEnginePicker({ engines, selected, loading, error, busy = false, onSelect, onRefresh }: {
  engines: AvailableDatabaseEngine[]; selected: string; loading: boolean; error: string; busy?: boolean;
  onSelect: (engine: string) => void; onRefresh: () => void;
}) {
  const root = useRef<HTMLFieldSetElement>(null);
  const refresh = useRef<HTMLButtonElement>(null);
  const hadFocus = useRef(false);
  useEffect(() => {
    // A refreshed capability may remove the focused radio. Keep keyboard focus
    // in this surface so modal Escape/Tab handling remains available.
    if (hadFocus.current && document.activeElement === document.body && !root.current?.closest('[inert]')) {
      (refresh.current?.disabled ? root.current : refresh.current)?.focus({ preventScroll: true });
    }
  }, [engines, loading, error, busy]);
  return <fieldset ref={root} tabIndex={-1} onFocusCapture={() => { hadFocus.current = true; }} onBlurCapture={event => {
    if (event.relatedTarget instanceof Node) hadFocus.current = Boolean(root.current?.contains(event.relatedTarget));
  }} disabled={busy} className="min-w-0 space-y-3">
    <legend className="mb-2 text-xs font-semibold text-[var(--ink)]">Mesin database</legend>
    {loading ? <p role="status" className="flex items-center gap-2 text-sm text-[var(--ink-muted)]"><Loader2 size={16} className="animate-spin" />Memeriksa mesin yang tersedia…</p>
      : error ? <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>
      : engines.length === 0 ? <p role="status" className="text-sm leading-relaxed text-[var(--ink-muted)]">Belum ada mesin basis data yang siap digunakan. Minta administrator memasang atau menjalankan layanannya, lalu muat ulang pilihan mesin.</p>
      : <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {engines.map(engine => <label key={engine.engine} className={`flex min-w-0 cursor-pointer items-center gap-3 rounded-[var(--radius-md)] border p-3 text-sm ${selected === engine.engine ? 'border-[var(--primary)] bg-[var(--raised)]' : 'border-[var(--line)]'}`}>
          <input type="radio" name="database-engine" value={engine.engine} checked={selected === engine.engine} onChange={() => onSelect(engine.engine)} />
          <Server size={17} className="shrink-0 text-[var(--ink-muted)]" aria-hidden="true" />
          <span className="min-w-0 break-words"><span className="block font-medium">{engine.label}</span><span className="text-xs text-[var(--ink-muted)]">Port {engine.port}</span></span>
        </label>)}
      </div>}
    <button ref={refresh} type="button" className="inline-flex min-h-9 items-center gap-2 text-xs font-medium text-[var(--primary)] disabled:opacity-50" disabled={loading || busy} onClick={onRefresh}><RefreshCw size={14} aria-hidden="true" />Muat ulang pilihan mesin</button>
  </fieldset>;
}
