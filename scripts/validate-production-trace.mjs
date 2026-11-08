import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

// 用单个测试文件固定成本测量条件，避免全量测试和编译任务共享 CPU。
const root = fileURLToPath(new URL('../', import.meta.url))
const result = spawnSync(process.execPath, [
  fileURLToPath(new URL('../node_modules/vitest/vitest.mjs', import.meta.url)),
  'run', 'src/streaming/runtime/production-trace-budget.test.ts',
], { cwd: root, env: { ...process.env, PRODUCTION_TRACE_VERIFY_COST: '1' }, stdio: 'inherit' })
if (result.error) throw result.error
process.exitCode = result.status ?? 1
