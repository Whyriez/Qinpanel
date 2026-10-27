import { useEffect, useId, useRef, useState, type HTMLAttributes } from 'react';
import { createPortal } from 'react-dom';

interface ModalOverlayProps extends HTMLAttributes<HTMLDivElement> {
  onDismiss?: () => void;
  dismissDisabled?: boolean;
  variant?: 'dialog' | 'sheet';
}

const openModals: HTMLElement[] = [];
let restoreBackground: (() => void) | undefined;
const focusableSelector =
  'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [contenteditable="true"], [tabindex]:not([tabindex="-1"])';

function controlsWithin(element: HTMLElement | null) {
  return Array.from(element?.querySelectorAll<HTMLElement>(focusableSelector) ?? []).filter(
    (control) => control.tabIndex >= 0 && control.getClientRects().length > 0 &&
      getComputedStyle(control).visibility !== 'hidden' && !control.closest('[inert]'),
  );
}

/** Portals keep dialogs clear of page clipping and transformed ancestors. */
export function ModalOverlay({
  children,
  className = '',
  onDismiss,
  dismissDisabled = false,
  variant = 'dialog',
  onKeyDown,
  onClick,
  ...props
}: ModalOverlayProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [opener] = useState(() => (typeof document === 'undefined' ? null : document.activeElement));

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const viewport = window.visualViewport;
    const syncViewport = () => {
      if (!viewport) return;
      element.style.setProperty('--dialog-viewport-height', `${viewport.height}px`);
      element.style.setProperty('--dialog-viewport-top', `${viewport.offsetTop}px`);
    };
    syncViewport();
    viewport?.addEventListener('resize', syncViewport);
    viewport?.addEventListener('scroll', syncViewport);
    if (openModals.length === 0) {
      const root = document.getElementById('root');
      const wasInert = root?.inert ?? false;
      const bodyOverflow = document.body.style.overflow;
      const htmlOverflow = document.documentElement.style.overflow;
      if (root) root.inert = true;
      document.body.style.overflow = 'hidden';
      document.documentElement.style.overflow = 'hidden';
      restoreBackground = () => {
        if (root) root.inert = wasInert;
        document.body.style.overflow = bodyOverflow;
        document.documentElement.style.overflow = htmlOverflow;
      };
    }
    const previous = openModals.at(-1);
    if (previous) previous.inert = true;
    openModals.push(element);
    const heading = element.querySelector<HTMLElement>('h1, h2, h3, [data-dialog-title]');
    if (!element.hasAttribute('aria-labelledby') && !element.hasAttribute('aria-label') && heading) {
      if (!heading.id) heading.id = titleId;
      element.setAttribute('aria-labelledby', heading.id);
    }
    const frame = requestAnimationFrame(() => {
      if (!element.contains(document.activeElement)) {
        const target = controlsWithin(element)[0];
        (target ?? element).focus({ preventScroll: true });
      }
    });
    return () => {
      cancelAnimationFrame(frame);
      viewport?.removeEventListener('resize', syncViewport);
      viewport?.removeEventListener('scroll', syncViewport);
      const index = openModals.indexOf(element);
      if (index >= 0) openModals.splice(index, 1);
      const top = openModals.at(-1);
      if (top) top.inert = false;
      if (openModals.length === 0) {
        restoreBackground?.();
        restoreBackground = undefined;
      }
      if (opener instanceof HTMLElement && opener.isConnected && !opener.closest('[inert]')) {
        opener.focus({ preventScroll: true });
      }
    };
  }, [opener, titleId]);

  const content = (
    <div
      {...props}
      ref={ref}
      className={`dialog-overlay ${className}`}
      data-variant={variant}
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      onClick={(event) => {
        if (event.target === event.currentTarget && dismissDisabled) return;
        onClick?.(event);
      }}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (event.defaultPrevented || openModals.at(-1) !== ref.current) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          if (!dismissDisabled) onDismiss?.();
        }
        if (event.key !== 'Tab') return;
        const controls = controlsWithin(ref.current);
        const first = controls[0];
        const last = controls.at(-1);
        if (!first) {
          event.preventDefault();
          ref.current?.focus();
        } else if (
          event.shiftKey &&
          (document.activeElement === first || document.activeElement === ref.current)
        ) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }}
    >
      {children}
    </div>
  );
  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
