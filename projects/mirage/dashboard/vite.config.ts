import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/mirage.simulator.Dashboard': {
        target: 'http://localhost:50051',
        changeOrigin: true,
      },
      '/api/terminal': {
        target: 'http://localhost:50051',
        changeOrigin: true,
      },
      '/api/session': {
        target: 'http://localhost:50051',
        changeOrigin: true,
      },
    },
  },
})
