import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const evidence = path.dirname(fileURLToPath(import.meta.url))
const project = path.resolve(evidence, '../../..')
const preview = '/private/tmp/xbxrc-home-host-cache-20261112'
const outDir = '/private/tmp/xbxrc-cloud-performance-p0-p2-20261112/dist/home-validation'
mkdirSync(preview, { recursive: true })
for (const folder of ['src', 'public']) {
  if (existsSync(path.join(project, folder))) cpSync(path.join(project, folder), path.join(preview, folder), { recursive: true })
}
const fixturePath = 'dev-docs/validation/2026-11-12-home-host-cache'
cpSync(evidence, path.join(preview, fixturePath), { recursive: true })
cpSync(path.join(project, 'package.json'), path.join(preview, 'package.json'))
if (!existsSync(path.join(preview, 'node_modules'))) symlinkSync(path.join(project, 'node_modules'), path.join(preview, 'node_modules'))
writeFileSync(path.join(preview, 'fixture.config.mjs'), `import path from 'node:path'
import vue from '@vitejs/plugin-vue'
export default { root: ${JSON.stringify(preview)}, base: '/home-validation/', plugins: [vue()], resolve: { alias: { '@': path.join(${JSON.stringify(preview)}, 'src'), '@shared': path.join(${JSON.stringify(preview)}, 'src/shared') } }, define: { __APP_VERSION__: JSON.stringify('0.1.0') }, build: { outDir: ${JSON.stringify(outDir)}, emptyOutDir: false, rollupOptions: { input: path.join(${JSON.stringify(preview)}, ${JSON.stringify(`${fixturePath}/index.html`)}) } } }
`)
const files = ['src/pages/Home.vue', 'src/pages/home/hosts-store.ts', 'src/pages/home/use-hosts.ts', 'src/shared/rpc/hosts.ts', 'src/shared/rpc/contract.ts', 'src/shared/events/contract.ts', 'src/shared/events/data.ts', 'src/App.vue', 'src/router/index.ts', 'src/navigation/core/engine.ts']
const hash = filename => createHash('sha256').update(readFileSync(filename)).digest('hex')
const sources = Object.fromEntries(files.map(file => [file, { workspace: hash(path.join(project, file)), preview: hash(path.join(preview, file)) }]))
writeFileSync(path.join(evidence, 'preview-source.json'), `${JSON.stringify({ date: '2026-11-12', preview, outDir, browserUrl: `http://127.0.0.1:1435/home-validation/${fixturePath}/index.html`, method: 'production Vite build；实际 App/router/页面/导航与模拟 Rust RPC，复用本任务已启动的本地 HTTP 验证服务', sources }, null, 2)}\n`)
