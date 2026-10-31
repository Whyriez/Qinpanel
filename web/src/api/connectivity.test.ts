import { describe, expect, it, vi } from 'vitest';
import { request } from './client';
import { fetchConnectivity } from './connectivity';
vi.mock('./client', () => ({ request: vi.fn() }));

describe('Connectivity project query', () => {
  it('sends project scope with pagination and preserves existing unscoped callers', async () => {
    vi.mocked(request).mockResolvedValue({ can_manage: false, endpoints: [], projects: [], certificates: [], page: 1, per_page: 25, total: 0, total_pages: 0, service_counts: {} });
    await fetchConnectivity(2, 'ssh', 7);
    expect(request).toHaveBeenLastCalledWith('/api/v1/connectivity?page=2&per_page=25&service=ssh&site_id=7');
    await fetchConnectivity();
    expect(request).toHaveBeenLastCalledWith('/api/v1/connectivity?page=1&per_page=25');
  });
});
