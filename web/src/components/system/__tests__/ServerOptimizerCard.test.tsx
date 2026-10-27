import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { ServerOptimizerCard } from '../ServerOptimizerCard';
import { OptimizerStatusResponseDto } from '../../../types/api';

describe('ServerOptimizerCard Component', () => {
  const mockStatusData: OptimizerStatusResponseDto = {
    hardware: {
      vcpu_count: 4,
      ram_total_mb: 8192,
      ram_free_mb: 5120,
      swap_total_mb: 4096,
      swap_free_mb: 4096,
      kernel_version: '6.8.0-45-generic',
      os_distro: 'Ubuntu 24.04 LTS',
    },
    recommended_profile: 'high_performance',
    active_profile: 'high_performance',
    applied_at: 1710000000,
    current_settings: {
      profile_type: 'high_performance',
      display_name: 'High Performance (4 - 16 GB RAM)',
      description: 'Tuning agresif untuk lalu lintas tinggi dan throughput maksimal.',
      nginx: {
        worker_processes: 'auto',
        worker_connections: 4096,
        worker_rlimit_nofile: 16384,
        multi_accept: true,
        keepalive_timeout_secs: 65,
        client_body_buffer_size_kb: 128,
        fastcgi_buffer_size_kb: 64,
        fastcgi_buffers: '64 64k',
        open_file_cache_max: 20000,
        open_file_cache_inactive_secs: 60,
      },
      sysctl: {
        somaxconn: 16384,
        tcp_max_syn_backlog: 8192,
        tcp_tw_reuse: 1,
        ip_local_port_range: '10240 65535',
        swappiness: 10,
        file_max: 500000,
        netdev_max_backlog: 5000,
      },
      php_fpm: {
        pm_type: 'dynamic',
        max_children: 60,
        start_servers: 8,
        min_spare_servers: 4,
        max_spare_servers: 16,
        process_idle_timeout_secs: 10,
        max_requests: 2000,
      },
      hints: {
        recommended_node_workers: 4,
        recommended_node_max_old_space_mb: 1024,
        recommended_python_workers: 9,
      },
    },
    recommended_settings: {
      profile_type: 'high_performance',
      display_name: 'High Performance (4 - 16 GB RAM)',
      description: 'Tuning agresif untuk lalu lintas tinggi dan throughput maksimal.',
      nginx: {
        worker_processes: 'auto',
        worker_connections: 4096,
        worker_rlimit_nofile: 16384,
        multi_accept: true,
        keepalive_timeout_secs: 65,
        client_body_buffer_size_kb: 128,
        fastcgi_buffer_size_kb: 64,
        fastcgi_buffers: '64 64k',
        open_file_cache_max: 20000,
        open_file_cache_inactive_secs: 60,
      },
      sysctl: {
        somaxconn: 16384,
        tcp_max_syn_backlog: 8192,
        tcp_tw_reuse: 1,
        ip_local_port_range: '10240 65535',
        swappiness: 10,
        file_max: 500000,
        netdev_max_backlog: 5000,
      },
      php_fpm: {
        pm_type: 'dynamic',
        max_children: 60,
        start_servers: 8,
        min_spare_servers: 4,
        max_spare_servers: 16,
        process_idle_timeout_secs: 10,
        max_requests: 2000,
      },
      hints: {
        recommended_node_workers: 4,
        recommended_node_max_old_space_mb: 1024,
        recommended_python_workers: 9,
      },
    },
    has_backup: true,
    last_backup_at: 1709999000,
  };

  it('renders hardware specs and active profile status', () => {
    const html = renderToString(<ServerOptimizerCard initialData={mockStatusData} />);

    expect(html).toContain('Pengoptimal Server &amp; Tuning Performa');
    expect(html).toContain('4 vCPU');
    expect(html).toContain('8.0 GB');
    expect(html).toContain('4.0 GB');
    expect(html).toContain('Ubuntu 24.04 LTS');
    expect(html).toContain('Kernel');
    expect(html).toContain('6.8.0-45-generic');
    expect(html).toContain('high_performance');
  });

  it('renders recommendation banner with smart profile rationale', () => {
    const html = renderToString(<ServerOptimizerCard initialData={mockStatusData} />);

    expect(html).toContain('Rekomendasi Cerdas:');
    expect(html).toContain('high_performance');
    expect(html).toContain('High Performance (4 - 16 GB RAM)');
  });

  it('renders all 4 available profiles and recommended badge', () => {
    const html = renderToString(<ServerOptimizerCard initialData={mockStatusData} />);

    expect(html).toContain('Micro VPS (&lt; 2 GB RAM)');
    expect(html).toContain('Standard (2 - 4 GB RAM)');
    expect(html).toContain('High Performance (4 - 16 GB RAM)');
    expect(html).toContain('Dedicated Large (&gt; 16 GB RAM)');
    expect(html).toContain('★ Disarankan');
    expect(html).toContain('Aktif');
  });

  it('renders rollback button and backup metadata when rollback snapshot exists', () => {
    const html = renderToString(<ServerOptimizerCard initialData={mockStatusData} />);

    expect(html).toContain('Pulihkan Cadangan');
    expect(html).toContain('Cadangan konfigurasi tersedia');
    expect(html).toContain('Rollback atomik siap');
  });

  it('hides rollback button when no snapshot is available', () => {
    const noBackupData: OptimizerStatusResponseDto = {
      ...mockStatusData,
      has_backup: false,
      last_backup_at: null,
    };

    const html = renderToString(<ServerOptimizerCard initialData={noBackupData} />);

    expect(html).not.toContain('Cadangan konfigurasi tersedia');
  });
});
