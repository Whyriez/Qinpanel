import { describe, expect, it } from 'vitest';
import { auditCategories, auditDetails, auditSearchParams, parseAuditFilters } from './auditFilters';
import id from '../i18n/locales/id.json';
import en from '../i18n/locales/en.json';

describe('Audit navigation and metadata', () => {
  it('restores category, search, status and page together from a shareable URL', () => {
    const expected = { category: 'websites', q: 'demo.test & admin', status: 'failure', page: 3 } as const;
    expect(parseAuditFilters(new URLSearchParams(auditSearchParams(expected)))).toEqual(expected);
  });
  it('normalizes invalid categories, statuses and unsafe pages', () => {
    for (const page of ['NaN', '-1', '1.5', '9007199254740992']) {
      expect(parseAuditFilters(new URLSearchParams(`category=unknown&status=broken&page=${page}`))).toEqual({ category: '', q: '', status: '', page: 1 });
    }
    expect(auditSearchParams({ category: '', q: '  ', status: '', page: 1 })).toBe('');
    expect(parseAuditFilters(new URLSearchParams({ q: 'x'.repeat(250) })).q).toHaveLength(200);
  });
  it('keeps percent, underscores and punctuation literal in the search URL', () => {
    const q = "%_ ' OR 1=1 -- <script>";
    expect(parseAuditFilters(new URLSearchParams(auditSearchParams({ category: 'other', q, status: '', page: 1 }))).q).toBe(q);
  });
  it('renders malformed historical JSON as text instead of crashing the inspector', () => {
    expect(auditDetails('{broken')).toBe('{broken');
    expect(auditDetails('{"site":1}')).toBe('{\n  "site": 1\n}');
    expect(auditDetails(null)).toBeNull();
    expect(auditDetails('')).toBeNull();
  });
  it('has labels and explanations for every category in both languages', () => {
    for (const locale of [id, en]) {
      expect(Object.keys(locale.audit.categories).sort()).toEqual([...auditCategories].sort());
      for (const category of auditCategories) {
        expect(locale.audit.categories[category]).toBeTruthy();
        expect(locale.audit.descriptions[category]).toBeTruthy();
      }
    }
  });
});
