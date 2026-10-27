import { describe, it, expect, vi, beforeEach } from 'vitest';
import { request } from './client';
import { changeMailbox, createMailbox, emailMessage, sendEmail, previewEmailTemplate, createEmailRequestId } from './email';
vi.mock('./client', () => ({ request: vi.fn().mockResolvedValue({}) }));
describe('email contracts', () => {
  beforeEach(() => vi.clearAllMocks());
  it('generates cryptographic UUIDs without requiring secure-context randomUUID', () => {
    const id = createEmailRequestId();
    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(createEmailRequestId()).not.toBe(id);
  });
  it('sends a domain id and never sends arbitrary storage paths', async () => {
    await createMailbox({ domain_id: 12, local_part: 'admin', quota_mb: 1024 });
    expect(request).toHaveBeenCalledWith('/api/v1/email/mailboxes', expect.objectContaining({ method: 'POST', body: JSON.stringify({ domain_id: 12, local_part: 'admin', quota_mb: 1024 }) }));
  });
  it('scopes message mutations to mailbox, folder, uid and uid validity', async () => {
    await emailMessage('box/one', 'trash', 'inbox', 42, 17);
    expect(request).toHaveBeenCalledWith('/api/v1/email/mailboxes/box%2Fone/messages', { method: 'POST', body: JSON.stringify({ action: 'trash', folder: 'inbox', uid: 42, uid_validity: 17 }) });
  });
  it('preserves the draft idempotency id across transport retries', async () => {
    const draft = { request_id: 'fixture-uuid', to: 'to@example.com', subject: 'Hello', body: 'Text' };
    await sendEmail('box', draft); await sendEmail('box', draft);
    expect(vi.mocked(request).mock.calls[0]).toEqual(vi.mocked(request).mock.calls[1]);
  });
  it('accepts generated and custom password resets', async () => {
    await changeMailbox('box', { action: 'reset_password' });
    expect(request).toHaveBeenLastCalledWith('/api/v1/email/mailboxes/box', { method: 'POST', body: '{"action":"reset_password"}' });
  });
  it('previews placeholders in one pass as plain text', () => {
    const result = previewEmailTemplate({ panel_name: '{title}<script>', subject: '{panel_name} {title}', body: '{message}\n{action_url}' });
    expect(result.subject).toBe('{title}<script> Sertifikat segera kedaluwarsa');
    expect(result.body).toContain('/sites/1?tab=ssl');
  });
});
