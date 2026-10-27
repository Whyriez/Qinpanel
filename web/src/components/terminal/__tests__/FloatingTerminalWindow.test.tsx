import { describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FloatingTerminalWindow } from '../FloatingTerminalWindow';
import { clampTerminalBounds, initialTerminalBounds } from '../floatingBounds';

describe('File terminal floating window', () => {
  it('renders a nonmodal window with drag and window controls', () => {
    const html = renderToString(<FloatingTerminalWindow title="Terminal berkas" cwd="sites/example/public" minimized={false} onMinimizedChange={vi.fn()} onClose={vi.fn()}><span>Session output</span></FloatingTerminalWindow>);
    expect(html).toContain('aria-modal="false"');
    expect(html).toContain('data-window-state="floating"');
    for (const label of ['Geser terminal', 'Atur posisi terminal', 'Minimalkan terminal', 'Maksimalkan terminal', 'Tutup terminal berkas']) expect(html).toContain(label);
    expect(html).toContain('sites/example/public');
  });

  it('keeps terminal content mounted when minimized', () => {
    const html = renderToString(<FloatingTerminalWindow title="Terminal berkas" minimized onMinimizedChange={vi.fn()} onClose={vi.fn()}><span>Session output remains mounted</span></FloatingTerminalWindow>);
    expect(html).toContain('data-window-state="minimized"');
    expect(html).toContain('Pulihkan terminal');
    expect(html).toContain('display:none');
    expect(html).toContain('Session output remains mounted');
  });

  it.each([{ width: 1440, height: 900 }, { width: 390, height: 844 }, { width: 320, height: 240 }])('keeps initial and dragged bounds inside viewport $width x $height', viewport => {
    const initial = initialTerminalBounds(viewport);
    for (const candidate of [initial, { ...initial, x: -500, y: -500 }, { ...initial, x: 10000, y: 10000 }]) {
      const result = clampTerminalBounds(candidate, viewport);
      expect(result.x).toBeGreaterThanOrEqual(12);
      expect(result.y).toBeGreaterThanOrEqual(12);
      expect(result.x + result.width).toBeLessThanOrEqual(viewport.width - 12);
      expect(result.y + result.height).toBeLessThanOrEqual(viewport.height - 12);
    }
    expect(initial.height).toBeLessThan(viewport.height);
  });

  it('fits oversized desktop bounds into a smaller viewport', () => {
    const result = clampTerminalBounds(initialTerminalBounds({ width: 1440, height: 900 }), { width: 390, height: 360 });
    expect(result.x + result.width).toBeLessThanOrEqual(378);
    expect(result.y + result.height).toBeLessThanOrEqual(348);
  });
});
