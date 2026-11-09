import path from 'node:path'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  resolve: { alias: { '@': path.resolve(import.meta.dirname, '../../../src'), '@shared': path.resolve(import.meta.dirname, '../../../src/shared') } },
  test: { include: ['dev-docs/validation/2026-11-09-gamepad-focus-action-audit/*.test.ts'] },
})
