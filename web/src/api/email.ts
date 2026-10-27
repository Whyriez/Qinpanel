import { request } from './client';
import type { JobItem } from '../types/api';
export type EmailFolder = 'inbox' | 'sent' | 'trash';
export interface Mailbox { id: string; domain_id: number; address: string; quota_mb: number; enabled: boolean; state: 'pending' | 'active' | 'disabled' | 'failed'; operation: 'apply' | 'delete'; job_id: string | null; error: string | null }
export interface EmailOverview { mailboxes: Mailbox[]; domains: { id: number; name: string; site_id?: number }[]; server: { configured: boolean; server: { hostname: string; certificate_id: number } | null; services: { name: string; running: boolean }[] } | null; server_error: string | null; certificates: { id: number; hostname: string; san_domains: string[] }[] }
export interface EmailTemplate { panel_name: string; subject: string; body: string }
export interface EmailJob { job: JobItem; id?: string; address?: string; password?: string | null }
export interface EmailItem { uid: number | string; flags?: string; 'hdr.from': string; 'hdr.to': string; 'hdr.subject': string; 'date.received.unixtime'?: string; 'size.physical'?: string; 'text.utf8'?: string }
export interface EmailMessages { items: EmailItem[]; uid_validity: number; total: number; unseen: number; before: number | null }
export interface EmailDns { hostname: string; ipv4: string | null; ipv6: string | null; records: { type: string; name: string; content: string; priority?: number }[] }
const base = '/api/v1/email';
/** UUID v4 using the Web Crypto API available on HTTP installations too. */
export function createEmailRequestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = Array.from(bytes, value => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
const post = <T,>(path: string, data: unknown) => request<T>(path, { method: 'POST', body: JSON.stringify(data) });
export const fetchEmail = () => request<EmailOverview>(base);
export const installEmail = (hostname: string, certificate_id: number) => post<EmailJob>(`${base}/install`, { hostname, certificate_id });
export const createMailbox = (data: { domain_id: number; local_part: string; password?: string; quota_mb: number }) => post<EmailJob>(`${base}/mailboxes`, data);
export const changeMailbox = (id: string, action: { action: 'reset_password'; password?: string } | { action: 'set_enabled'; enabled: boolean } | { action: 'delete' | 'retry' }) => post<EmailJob>(`${base}/mailboxes/${encodeURIComponent(id)}`, action);
export const fetchEmailMessages = (id: string, folder: EmailFolder, before?: number) => request<EmailMessages>(`${base}/mailboxes/${encodeURIComponent(id)}/messages?folder=${folder}${before ? `&before=${before}` : ''}`);
export const emailMessage = (id: string, action: 'read' | 'trash', folder: EmailFolder, uid: number, uid_validity: number) => post<EmailItem>(`${base}/mailboxes/${encodeURIComponent(id)}/messages`, { action, folder, uid, uid_validity });
export const sendEmail = (id: string, data: { request_id: string; to: string; subject: string; body: string }) => post<EmailJob>(`${base}/mailboxes/${encodeURIComponent(id)}/send`, data);
export const fetchEmailDns = (id: string) => request<EmailDns>(`${base}/mailboxes/${encodeURIComponent(id)}/dns`);
export const fetchEmailTemplate = () => request<EmailTemplate>(`${base}/templates`);
export const saveEmailTemplate = (data: EmailTemplate) => request<EmailTemplate>(`${base}/templates`, { method: 'PUT', body: JSON.stringify(data) });
/** One pass only: placeholder text inside event values is never expanded. */
export function previewEmailTemplate(template: EmailTemplate) {
  const values: Record<string, string> = { panel_name: template.panel_name, title: 'Sertifikat segera kedaluwarsa', message: 'Sertifikat project perlu diperbarui dalam 7 hari.', category: 'ssl_expiring', action_url: '/sites/1?tab=ssl' };
  const render = (s: string) => s.replace(/\{([^{}]+)\}/g, (token, key: string) => values[key] ?? token);
  return { subject: render(template.subject), body: render(template.body) };
}
