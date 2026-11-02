import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config'

export default defineConfig(async env => mergeConfig(await viteConfig(env), {
  test: {
    include: ['src/**/*.test.ts'],
  },
}))
