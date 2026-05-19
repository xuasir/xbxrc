#!/usr/bin/env node
/**
 * 收集 pnpm tauri build 产物到统一目录，供 GitHub Release 上传。
 * 用法: node scripts/release-collect-assets.mjs <outDir> <version> <label>
 *   label: macos | windows
 */
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import process from 'node:process'

const outDir = resolve(process.argv[2] ?? 'release-assets')
const version = process.argv[3]
const label = process.argv[4]

if (!version || !label) {
  console.error('用法: node scripts/release-collect-assets.mjs <outDir> <version> <macos|windows>')
  process.exit(1)
}

const root = resolve(import.meta.dirname, '..')
const bundleRoot = join(root, 'src-tauri', 'target', 'release', 'bundle')

function copyFirstMatch(dir, pattern, destName) {
  if (!existsSync(dir)) {
    throw new Error(`目录不存在: ${dir}`)
  }
  const names = readdirSync(dir)
  const re = new RegExp(pattern)
  const hit = names.find(name => re.test(name))
  if (!hit) {
    throw new Error(`在 ${dir} 未找到匹配 ${pattern} 的文件`)
  }
  const src = join(dir, hit)
  const dest = join(outDir, destName === null ? basename(src) : (destName ?? basename(src)))
  copyFileSync(src, dest)
  console.log(`collect: ${src} -> ${dest}`)
  return dest
}

mkdirSync(outDir, { recursive: true })

if (label === 'macos') {
  const dmgDir = join(bundleRoot, 'dmg')
  const macDir = join(bundleRoot, 'macos')
  copyFirstMatch(dmgDir, '\\.dmg$', `xbxrc-${version}-macos.dmg`)
  const macTar = copyFirstMatch(macDir, '\\.tar\\.gz$', null)
  copyFirstMatch(macDir, '\\.tar\\.gz\\.sig$', `${basename(macTar)}.sig`)
}
else if (label === 'windows') {
  const nsisDir = join(bundleRoot, 'nsis')
  copyFirstMatch(nsisDir, 'setup\\.exe$', `xbxrc-${version}-windows-setup.exe`)
  copyFirstMatch(nsisDir, 'setup\\.exe\\.sig$', `xbxrc-${version}-windows-setup.exe.sig`)
}
else {
  console.error(`未知 label: ${label}`)
  process.exit(1)
}

const manifest = {
  version,
  label,
  files: readdirSync(outDir),
}
writeFileSync(join(outDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
console.log(`已写入 ${join(outDir, 'manifest.json')}`)
