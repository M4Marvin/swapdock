import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import tailwindcss from '@tailwindcss/vite'
import viteReact from '@vitejs/plugin-react'
import path from 'node:path'
import { defineConfig } from 'vite'

export default defineConfig({
  resolve: {
    alias: { '@': path.resolve(import.meta.dirname, './src') },
  },
  plugins: [
    tailwindcss(),
    tanstackStart({
      prerender: {
        enabled: true,
        crawlLinks: true,
        autoStaticPathsDiscovery: true,
        failOnError: true,
      },
    }),
    viteReact(),
  ],
})
