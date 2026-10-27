import { defineConfig, type ProxyOptions } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';
import type { ClientRequest, IncomingMessage } from 'node:http';

// Mirror the production proxy boundary: browser headers cannot assert TLS.
function panelProxy(ws = false): ProxyOptions {
  return {
    target: 'http://127.0.0.1:7700', changeOrigin: false, ws,
    configure(proxy) {
      const sanitize = (outgoing: ClientRequest, incoming: IncomingMessage) => {
        for (const name of ['x-qpanel-cf-visitor', 'cf-visitor', 'forwarded', 'x-forwarded-host']) outgoing.removeHeader(name);
        outgoing.setHeader('x-forwarded-proto', 'encrypted' in incoming.socket && incoming.socket.encrypted ? 'https' : 'http');
      };
      proxy.on('proxyReq', sanitize);
      proxy.on('proxyReqWs', sanitize);
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': panelProxy(),
      '/healthz': panelProxy(),
      '/ws': panelProxy(true),
      '/connectivity/verify': panelProxy(),
      '/swagger-ui': panelProxy(),
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        manualChunks: {
          'vendor-react': ['react', 'react-dom'],
          'vendor-icons': ['lucide-react'],
        },
      },
    },
  },
});
