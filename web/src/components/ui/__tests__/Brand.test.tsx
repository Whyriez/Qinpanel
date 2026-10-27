import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { Brand, BrandLoading } from '../Brand';
import '../../../i18n';
const readAsset = (path: string) => readFileSync(new URL(`../../../../${path}`, import.meta.url), 'utf8').trim();
const mark = readAsset('public/brand/qinpanel-mark.svg');
const markShapes = mark.replace(/^<svg\b[^>]*>/, '').replace(/<\/svg>$/, '');

describe('Qinpanel identity', () => {
  it.each(['mark', 'wordmark-light', 'wordmark-dark'])('keeps cached %s asset URLs as aliases of the new identity', (name) => {
    expect(readAsset(`public/brand/qonpanel-${name}.svg`)).toBe(readAsset(`public/brand/qinpanel-${name}.svg`));
  });
  it('serves the identical mark as the browser favicon', () => {
    expect(readAsset('public/favicon.svg')).toBe(mark);
    expect(readAsset('index.html')).toContain('href="/favicon.svg?v=4"');
    expect(readAsset('index.html')).toContain('src="/brand/qinpanel-mark.svg?v=4"');
  });

  it.each(['light', 'dark'])('keeps the %s wordmark shapes and transparent fill identical to the favicon', (theme) => {
    const wordmark = readAsset(`public/brand/qinpanel-wordmark-${theme}.svg`);
    // Missing fill="none" inherits SVG's default black fill inside the Q.
    expect(mark.match(/^<svg\b[^>]*>/)?.[0]).toContain('fill="none"');
    expect(wordmark.match(/^<svg\b[^>]*>/)?.[0]).toContain('fill="none"');
    expect(wordmark.match(/<g\b[^>]*>(.*?)<\/g>/)?.[1]).toBe(markShapes);
  });

  it('embeds the same mark in the standalone website error page', () => {
    const errorPage = readAsset('public/errors/404.html');
    expect(errorPage.match(/<svg\b[^>]*>/)?.[0]).toContain('fill="none"');
    expect(errorPage.match(/<svg\b[^>]*>(.*?)<\/svg>/)?.[1]).toBe(markShapes);
  });

  it('shares the canonical decorative mark and accessible product name', () => {
    const html = renderToStaticMarkup(<Brand />);
    expect(html).toContain('aria-label="Qinpanel"');
    expect(html).toContain('/brand/qinpanel-mark.svg?v=4');
    expect(html).toContain('aria-hidden="true"');
    expect(html).not.toMatch(/QonPanel|QinPanel|Qpanel|Nusa|Vpanel/);
  });
  it('announces loading and reserves the full brand for initial and session loading', () => {
    for (const fullScreen of [true, false]) {
      const html = renderToStaticMarkup(<BrandLoading fullScreen={fullScreen} message="Memeriksa sesi Anda?" />);
      expect(html).toContain('role="status"');
      expect(html).toContain('aria-live="polite"');
      expect(html.includes('Qinpanel')).toBe(fullScreen);
      expect(html).toContain('aria-atomic="true"');
      expect(html).toContain('panel-loading__spinner');
      expect(html).not.toContain('role="progressbar"');
      expect(html.includes('panel-loading__placeholder')).toBe(!fullScreen);
      expect(html.includes('brand-loading--full')).toBe(fullScreen);
    }
  });
  it('shares the screen layout and loading stylesheet before and after React starts', () => {
    const html = readAsset('index.html');
    for (const className of ['panel-loading--screen', 'panel-loading__content', 'panel-loading__status', 'panel-loading__spinner']) {
      expect(html).toContain(className);
      expect(renderToStaticMarkup(<BrandLoading fullScreen message="Menyiapkan panel…" />)).toContain(className);
    }
    expect(html).toContain('/brand/boot.css?v=2');
    expect(html.indexOf('/brand/theme.js')).toBeLessThan(html.indexOf('/brand/boot.css'));
    expect(readAsset('public/brand/boot.css')).toContain('prefers-reduced-motion: reduce');
  });
  it.each([
    ['dark', false, true], ['light', true, false], ['system', true, true],
    ['system', false, false], [null, true, true], ['invalid', false, false], ['invalid', true, false],
  ])('applies saved theme %s with system dark %s before first paint', (preference, systemDark, expectedDark) => {
    const classes = new Set<string>();
    runInNewContext(readAsset('public/brand/theme.js'), {
      localStorage: { getItem: (key: string) => { expect(key).toBe('nusa_theme'); return preference; } },
      matchMedia: () => ({ matches: systemDark }),
      document: { documentElement: { classList: {
        toggle: (name: string, enabled: boolean) => { if (enabled) classes.add(name); else classes.delete(name); },
        add: (name: string) => { classes.add(name); },
      } } },
    });
    expect(classes.has('dark')).toBe(expectedDark);
    expect(classes.has('theme-ready')).toBe(true);
  });
});
