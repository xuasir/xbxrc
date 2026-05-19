#!/usr/bin/env node
/**
 * 根据 release-assets 目录生成 Tauri updater latest.json
 * 用法: node scripts/release-write-latest-json.mjs <assetsRoot> <version> <downloadBaseUrl>
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import process from 'node:process'
import {
  checkReleaseCollected,
  findBundleFile,
  resolveCollectedDir,
} from './release-bundle-lib.mjs'

const assetsRoot = resolve(process.argv[2] ?? 'release-assets')
const version = process.argv[3]
const downloadBase = process.argv[4]?.replace(/\/$/, '')

if (!version || !downloadBase) {
  console.error('用法: node scripts/release-write-latest-json.mjs <assetsRoot> <version> <downloadBaseUrl>')
  process.exit(1)
}

function readSig(path) {
  return readFileSync(path, 'utf8').trim()
}

const platforms = {}
const errors = []

for (const [label, platformKey, tarSuffix, sigSuffix] of [
  ['macos', 'darwin-aarch64', '.tar.gz', '.tar.gz.sig'],
  ['windows', 'windows-x86_64', '-windows-setup.exe', '-windows-setup.exe.sig'],
]) {
  const { missing } = checkReleaseCollected(assetsRoot, label)
  if (missing.length > 0) {
    errors.push(`${label}: 缺少 ${missing.map(item => item.hint).join(', ')}`)
    continue
  }
  const dir = resolveCollectedDir(assetsRoot, label)
  const bundle = findBundleFile(dir, tarSuffix, { excludeSuffix: sigSuffix })
  const sig = findBundleFile(dir, sigSuffix)
  if (!bundle || !sig) {
    errors.push(`${label}: 无法定位 updater 包或签名`)
    continue
  }
  platforms[platformKey] = {
    signature: readSig(join(dir, sig)),
    url: `${downloadBase}/${bundle}`,
  }
}

if (errors.length > 0) {
  console.error('无法生成 latest.json:')
  for (const line of errors) {
    console.error(`  - ${line}`)
  }
  process.exit(1)
}

if (Object.keys(platforms).length === 0) {
  console.error('未找到任何平台 updater 资产')
  process.exit(1)
}

const latest = {
  version,
  notes: '',
  pub_date: new Date().toISOString(),
  platforms,
}

const outPath = join(assetsRoot, 'latest.json')
writeFileSync(outPath, `${JSON.stringify(latest, null, 2)}\n`)
console.log(`已写入 ${outPath}`)
console.log('platforms:', Object.keys(platforms).join(', '))
