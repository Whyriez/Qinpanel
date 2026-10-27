export interface WindowBounds { x: number; y: number; width: number; height: number }
export interface ViewportSize { width: number; height: number }
export const WINDOW_MARGIN = 12;

export function clampTerminalBounds(bounds: WindowBounds, viewport: ViewportSize): WindowBounds {
  const width = Math.min(bounds.width, Math.max(1, viewport.width - WINDOW_MARGIN * 2));
  const height = Math.min(bounds.height, Math.max(1, viewport.height - WINDOW_MARGIN * 2));
  return {
    width, height,
    x: Math.max(WINDOW_MARGIN, Math.min(bounds.x, viewport.width - width - WINDOW_MARGIN)),
    y: Math.max(WINDOW_MARGIN, Math.min(bounds.y, viewport.height - height - WINDOW_MARGIN)),
  };
}

export function initialTerminalBounds(viewport: ViewportSize): WindowBounds {
  const width = Math.min(840, viewport.width - WINDOW_MARGIN * 2);
  const height = Math.min(440, Math.round(viewport.height * 0.62));
  return clampTerminalBounds({ x: viewport.width - width - 24, y: viewport.height - height - 72, width, height }, viewport);
}
