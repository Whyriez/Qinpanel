import { request } from './client';
import type { JobItem } from '../types/api';

export interface ProjectDomain { domain_id: number; site_id: number; hostname: string; kind: string; status: string; tls: boolean }
export interface NetworkOverview { ipv4: string | null; ipv6: string | null; projects: ProjectDomain[] }
export interface ManagedTunnel { id: string; local_status: string; routes: { hostname: string; zone_id: string }[] }
export interface CloudflareStatus { configured: boolean; account_id: string | null; connector_installed: boolean; managed_tunnels: ManagedTunnel[] }
export interface Zone { id: string; name: string; status: string; name_servers: string[] }
export interface DnsRecord { id: string; type: string; name: string; content: string; ttl: number; proxied: boolean; priority?: number }
export interface Tunnel { id: string; name: string; status: string; managed: boolean; remote_config: boolean }
export interface CloudflarePage<T> { items: T[]; pagination: { page: number; total_pages: number; total_count: number } }
export type RecordDraft = Omit<DnsRecord, 'id' | 'type'> & { kind: string };
export type CloudflareCommand =
  | { action: 'configure'; account_id: string; api_token: string }
  | { action: 'zones' | 'tunnels'; page: number }
  | { action: 'create_zone' | 'create_tunnel'; name: string }
  | { action: 'records'; zone_id: string; page: number }
  | { action: 'save_record'; zone_id: string; record_id: string | null; record: RecordDraft }
  | { action: 'delete_record'; zone_id: string; record_id: string }
  | { action: 'tunnel_action'; tunnel_id: string; operation: 'start' | 'stop' | 'remove' | 'import' }
  | { action: 'publish'; zone_id: string; tunnel_id: string; hostname: string; tls: boolean }
  | { action: 'unpublish'; zone_id: string; tunnel_id: string; hostname: string };

export const fetchNetwork = () => request<NetworkOverview>('/api/v1/network');
export const savePublicAddresses = (ipv4: string, ipv6: string) => request('/api/v1/network', { method: 'PUT', body: JSON.stringify({ ipv4, ipv6 }) });
export async function fetchCloudflareStatus(signal?: AbortSignal): Promise<CloudflareStatus> {
  let value: CloudflareStatus;
  try {
    value = await request<CloudflareStatus>('/api/v1/network/cloudflare', { signal });
  } catch (error) {
    const status = (error as { status?: number } | null)?.status;
    if (status === 502 || status === 504 || error instanceof TypeError) {
      throw new Error('Status Cloudflare belum dapat diperbarui karena koneksi panel terputus. Muat ulang status setelah layanan kembali tersedia; tidak perlu memasang ulang Tunnel.');
    }
    throw error;
  }
  if (!value || typeof value.configured !== 'boolean' || typeof value.connector_installed !== 'boolean'
    || !(value.account_id === null || typeof value.account_id === 'string') || !Array.isArray(value.managed_tunnels)
    || value.managed_tunnels.some(tunnel => !tunnel || typeof tunnel.id !== 'string' || typeof tunnel.local_status !== 'string'
      || !Array.isArray(tunnel.routes) || tunnel.routes.some(route => !route || typeof route.hostname !== 'string' || typeof route.zone_id !== 'string'))) {
    throw new Error('Respons status Cloudflare tidak valid. Muat ulang status atau perbarui paket Qinpanel lengkap.');
  }
  return value;
}
export const cloudflareCommand = <T,>(command: CloudflareCommand) => request<T>('/api/v1/network/cloudflare', { method: 'POST', body: JSON.stringify(command) });
export const installCloudflare = () => request<JobItem>('/api/v1/network/cloudflare/install', { method: 'POST' });
export const checkProjectDns = (domain_id: number) => request<{ hostname: string; addresses: string[]; expected: string[]; matches: boolean }>('/api/v1/network/dns-check', { method: 'POST', body: JSON.stringify({ domain_id }) });
