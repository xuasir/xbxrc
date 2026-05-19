#!/usr/bin/env node
/**
 * 根据 release-assets 目录生成 Tauri updater latest.json
 * 用法: node scripts/release-write-latest-json.mjs <assetsRoot> <version> <downloadBaseUrl>
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import process from 'node:process'

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

function findFile(dir, pattern) {
  if (!existsSync(dir)) {
    return null
  }
  const re = new RegExp(pattern)
  return readdirSync(dir).find(name => re.test(name)) ?? null
}

function resolvePlatformDir(root, label) {
  const candidates = [
    join(root, label),
    join(root, `release-assets-${label}`),
  ]
  return candidates.find(path => existsSync(path)) ?? null
}

const platforms = {}

const macDir = resolvePlatformDir(assetsRoot, 'macos')
const macTar = findFile(macDir, '\\.tar\\.gz$')
const macSig = findFile(macDir, '\\.tar\\.gz\\.sig$')
if (macTar && macSig) {
  platforms['darwin-aarch64'] = {
    signature: readSig(join(macDir, macSig)),
    url: `${downloadBase}/${macTar}`,
  }
}

const winDir = resolvePlatformDir(assetsRoot, 'windows')
const winExe = findFile(winDir, 'setup\\.exe$')
const winSig = findFile(winDir, 'setup\\.exe\\.sig$')
if (winExe && winSig) {
  platforms['windows-x86_64'] = {
    signature: readSig(join(winDir, winSig)),
    url: `${downloadBase}/${winExe}`,
  }
}

if (Object.keys(platforms).length === 0) {
  console.error('未找到任何平台 updater 资产，无法生成 latest.json')
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
