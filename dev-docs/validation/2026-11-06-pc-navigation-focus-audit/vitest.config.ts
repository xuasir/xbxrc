import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from '../../../vite.config'

// 审计用不变量：独立运行，记录当前版本的失败行为。
export default defineConfig(async env => mergeConfig(await viteConfig(env), {
  test: {
    include: ['dev-docs/validation/2026-11-06-pc-navigation-focus-audit/*.test.ts'],
  },
}))
