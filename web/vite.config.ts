import path from 'path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { tanstackRouter } from '@tanstack/router-plugin/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [
    tanstackRouter({ target: 'react', autoCodeSplitting: true }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  server: {
    // Everything the frontend talks to lives under /api, so a single proxy
    // entry covers it and the SPA's own routes (/, /apps/..., /runs/...) are
    // left for Vite to serve, keeping deep links working in dev.
    proxy: {
      '/api': 'http://127.0.0.1:8088',
    },
  },
})
