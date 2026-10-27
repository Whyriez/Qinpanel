import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { ConnectionDialog } from './ConnectionDialog';
import type { StorageConnection } from '../../api/storage';
vi.mock('../ui/ModalOverlay', () => ({ ModalOverlay: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
describe('S3 credential form', () => {
  it('labels write-only password fields and does not claim saved provider credentials', () => {
    const html = renderToString(<ConnectionDialog context={{ owners: [{ id: 2, username: 'alice' }], sites: [], upload_limit_bytes: 33554432 }} english={false} onClose={vi.fn()} onSaved={vi.fn()} />);
    expect(html).toContain('Hubungkan storage S3'); expect(html).toContain('type="password"'); expect(html).toContain('autoComplete="new-password"'); expect(html).toContain('Key dienkripsi'); expect(html).toContain('Supabase S3'); expect(html).not.toContain('enc:v1:');
  });

  it('provides named form groups, a dialog description and an optional session token', () => {
    const html = renderToString(<ConnectionDialog context={{ owners: [{ id: 2, username: 'alice' }], sites: [], upload_limit_bytes: 33554432 }} english onClose={vi.fn()} onSaved={vi.fn()} />);
    for (const label of ['Connection details', 'Bucket destination', 'Access credentials']) expect(html).toContain(`<legend>${label}</legend>`);
    expect(html).toContain('Connect an existing bucket for your tenant or project.');
    expect(html).toContain('<summary>Session token (optional)</summary>');
    expect(html).toContain('Saving a connection does not create a bucket.');
  });

  it('keeps editing credentials empty and clarifies that compatible storage must already exist', () => {
    const connection: StorageConnection = { id: 'fixture', owner_id: 2, site_id: null, label: 'Project media', provider: 'compatible', endpoint: 'https://objects.example.test', region: 'us-east-1', addressing: 'path', bucket: 'project-media', revision: 1, verified_at: null, has_credentials: true, created_at: 1, updated_at: 1 };
    const html = renderToString(<ConnectionDialog connection={connection} context={{ owners: [{ id: 2, username: 'alice' }], sites: [], upload_limit_bytes: 33554432 }} english onClose={vi.fn()} onSaved={vi.fn()} />);
    const credentials = html.match(/<input[^>]+type="password"[^>]*>/g) ?? [];
    expect(credentials).toHaveLength(3);
    for (const credential of credentials) { expect(credential).toContain('value=""'); expect(credential).not.toContain('required'); }
    expect(html).toContain('Leave all key fields empty to keep credentials');
    expect(html).toContain('Use an S3 endpoint that is already running.');
  });
});
