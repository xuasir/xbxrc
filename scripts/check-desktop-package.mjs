#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { readReleaseIdentity, repositoryRoot } from './release-edition.mjs'
import { checkMacosRuntime } from './macos-runtime-lib.mjs'

const identity = readReleaseIdentity()
const target = resolve(repositoryRoot, process.env.CARGO_TARGET_DIR ?? `target/${identity.edition}`, 'release')
const architecture = process.argv[2]
if (!["aarch64", "x86_64"].includes(architecture))
  throw new Error("安装包检查需要明确架构 aarch64 / x86_64")
const forbidden = /(?:ffmpeg|libavcodec|libavutil|libavformat|libswscale|libswresample|sdl3)/i
if (process.platform === 'darwin') {
  const app = join(target, 'bundle/macos', `${identity.productName}.app`)
  const plist = key => execFileSync('plutil', ['-extract', key, 'raw', '-o', '-', join(app, 'Contents/Info.plist')], { encoding: 'utf8' }).trim()
  if (plist('CFBundleIdentifier') !== identity.identifier || plist('CFBundleShortVersionString') !== identity.version)
    throw new Error('app 的安装身份或版本与当前 edition 不符')
  const closure = checkMacosRuntime(app, { architectures: [architecture === 'aarch64' ? 'arm64' : 'x86_64'] })
  const paths = []
  const walk = path => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name)
      paths.push(child)
      if (entry.isDirectory()) walk(child)
    }
  }
  walk(app)
  if (identity.edition === 'browser' && paths.some(path => forbidden.test(path) || path.endsWith('.dll')))
    throw new Error('Browser app 包含原生 WIP 运行库或平台冗余资源')
  console.log(JSON.stringify({ edition: identity.edition, identifier: identity.identifier, version: identity.version, app, closure }, null, 2))
}
else if (process.platform === 'win32') {
  if (architecture !== 'x86_64')
    throw new Error('Windows 发行架构必须为 x86_64')
  const binary = join(target, 'xbxrc.exe')
  const path = binary.replaceAll("'", "''")
  const details = JSON.parse(execFileSync('powershell', ['-NoProfile', '-Command', `(Get-Item -LiteralPath '${path}').VersionInfo | Select-Object ProductName,ProductVersion | ConvertTo-Json -Compress`], { encoding: 'utf8' }))
  if (details.ProductName !== identity.productName || details.ProductVersion !== identity.version)
    throw new Error('exe 的产品名称或版本与当前 edition 不符')
  const dependencies = execFileSync('dumpbin', ['/dependents', binary], { encoding: 'utf8' })
  if (identity.edition === 'browser' && forbidden.test(dependencies))
    throw new Error('Browser exe 依赖原生 WIP 运行库')
  console.log(JSON.stringify({ edition: identity.edition, details, binary }, null, 2))
}
else {
  throw new Error('桌面发行检查支持 macOS 和 Windows')
}
