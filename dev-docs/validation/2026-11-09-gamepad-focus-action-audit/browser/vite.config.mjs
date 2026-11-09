import path from 'node:path'
import { fileURLToPath } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

const root = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(root, '../../../..')
export default defineConfig({
  root,
  plugins: [{
    name: 'audit-isolate-feedback',
    resolveId(id, importer) {
      if (id === './haptics' && importer?.includes('/src/navigation/core/'))
        return '\0audit-feedback'
      if (id === '@shared/gamepad/runtime' || (id === './runtime' && importer?.includes('/src/shared/gamepad/')))
        return '\0audit-controller'
    },
    load(id) {
      if (id === '\0audit-feedback')
        return 'export function playNavSound() {} export function triggerNavHaptic() {} export function syncHapticsConfig() {}'
      if (id === '\0audit-controller')
        return 'const snapshot = { preferences: {controlSource: "gamepad"} }; export const inputController = {getSnapshot: () => snapshot, subscribe: handler => {handler(snapshot); return () => {}}, rumble: async () => {}}'
    },
  }, vue()],
  resolve: { alias: { '@': path.join(repo, 'src'), '@shared': path.join(repo, 'src/shared') } },
  server: { host: '127.0.0.1', port: 8947, strictPort: true, fs: { allow: [repo] } },
})
