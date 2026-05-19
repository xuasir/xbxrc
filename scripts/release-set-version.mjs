#!/usr/bin/env node
/**
 * 将 package.json 与 src-tauri/tauri.conf.json 的 version 设为同一 semver（不含 v 前缀）。
 * 用法: node scripts/release-set-version.mjs 0.2.0
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const root = resolve(__dirname, '..')

const version = process.argv[2]
if (!version) {
  console.error('用法: node scripts/release-set-version.mjs <semver>')
  console.error('示例: node scripts/release-set-version.mjs 0.2.0')
  process.exit(1)
}

if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`无效版本号: ${version}（期望形如 0.2.0，勿带 v 前缀）`)
  process.exit(1)
}

if (version.includes('-beta.')) {
  console.error('基础版本不要包含 -beta.N；beta 后缀由 CI 在 release/test 上自动追加')
  process.exit(1)
}

const pkgPath = resolve(root, 'package.json')
const tauriPath = resolve(root, 'src-tauri/tauri.conf.json')

const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'))
const tauri = JSON.parse(readFileSync(tauriPath, 'utf8'))

const prev = pkg.version
pkg.version = version
tauri.version = version

writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`)
writeFileSync(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`)

console.log(`version: ${prev} -> ${version}`)
console.log(`已更新: ${pkgPath}`)
console.log(`已更新: ${tauriPath}`)
