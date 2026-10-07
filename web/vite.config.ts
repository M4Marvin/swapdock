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
    proxy: {
      '/health': 'http://127.0.0.1:8088',
      '/apps': 'http://127.0.0.1:8088',
      '/validate': 'http://127.0.0.1:8088',
      '/render': 'http://127.0.0.1:8088',
      '/apply': 'http://127.0.0.1:8088',
      '/runs': 'http://127.0.0.1:8088',
    },
  },
})
