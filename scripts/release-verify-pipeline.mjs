#!/usr/bin/env node
import { spawnSync } from 'node:child_process'
/**
 * 本地模拟 CI 各阶段（macOS + Windows），改 release 脚本后必须先跑通。
 */
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..')
const scripts = join(repoRoot, 'scripts')

function run(nodeArgs, extraEnv = {}) {
  const result = spawnSync(process.execPath, nodeArgs, {
    cwd: repoRoot,
    encoding: 'utf8',
    env: { ...process.env, ...extraEnv },
  })
  if (result.status !== 0) {
    console.error(result.stdout)
    console.error(result.stderr)
    throw new Error(`命令失败: node ${nodeArgs.join(' ')}`)
  }
  return result.stdout
}

function seedCollected(dir) {
  mkdirSync(join(dir, 'release-assets-macos'), { recursive: true })
  mkdirSync(join(dir, 'release-assets-windows'), { recursive: true })
  writeFileSync(join(dir, 'release-assets-macos', 'xbxrc-0.1.0-beta.6-macos.dmg'), '')
  writeFileSync(join(dir, 'release-assets-macos', 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz'), '')
  writeFileSync(join(dir, 'release-assets-macos', 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz.sig'), 'sig-mac')
  writeFileSync(join(dir, 'release-assets-windows', 'xbxrc-0.1.0-beta.6-windows-setup.exe'), '')
  writeFileSync(join(dir, 'release-assets-windows', 'xbxrc-0.1.0-beta.6-windows-setup.exe.sig'), 'sig-win')
}

function seedBundleRoot(bundleRoot) {
  mkdirSync(join(bundleRoot, 'dmg'), { recursive: true })
  mkdirSync(join(bundleRoot, 'macos'), { recursive: true })
  mkdirSync(join(bundleRoot, 'nsis'), { recursive: true })
  writeFileSync(join(bundleRoot, 'dmg', 'xbxrc_0.1.0-beta.6_aarch64.dmg'), '')
  writeFileSync(join(bundleRoot, 'macos', 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz'), '')
  writeFileSync(join(bundleRoot, 'macos', 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz.sig'), 'sig-mac')
  writeFileSync(join(bundleRoot, 'nsis', 'xbxrc_0.1.0-beta.6_x64-setup.exe'), '')
  writeFileSync(join(bundleRoot, 'nsis', 'xbxrc_0.1.0-beta.6_x64-setup.exe.sig'), 'sig-win')
}

function assertIncludes(text, part, message) {
  if (!text.includes(part)) {
    throw new Error(`${message}\n期望包含: ${part}\n实际:\n${text}`)
  }
}

const tmp = mkdtempSync(join(tmpdir(), 'release-pipeline-'))

try {
  // build job：参数与 release-beta.yml 一致
  const macDir = join(tmp, 'release-assets-macos')
  mkdirSync(macDir, { recursive: true })
  writeFileSync(join(macDir, 'xbxrc-0.1.0-beta.6-macos.dmg'), '')
  writeFileSync(join(macDir, 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz'), '')
  writeFileSync(join(macDir, 'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz.sig'), '')
  run([join(scripts, 'release-check-collected.mjs'), macDir, 'macos'])
  console.log('ok  CI build job: release-check-collected release-assets-macos macos')

  const winDir = join(tmp, 'release-assets-windows')
  mkdirSync(winDir, { recursive: true })
  writeFileSync(join(winDir, 'xbxrc-0.1.0-beta.6-windows-setup.exe'), '')
  writeFileSync(join(winDir, 'xbxrc-0.1.0-beta.6-windows-setup.exe.sig'), '')
  run([join(scripts, 'release-check-collected.mjs'), winDir, 'windows'])
  console.log('ok  CI build job: release-check-collected release-assets-windows windows')

  // bundle 校验 + collect（Tauri 真实文件名）
  const bundleRoot = join(tmp, 'bundle')
  seedBundleRoot(bundleRoot)
  const bundleEnv = { RELEASE_BUNDLE_ROOT: bundleRoot }

  run([join(scripts, 'release-check-bundle.mjs'), 'macos'], bundleEnv)
  run([join(scripts, 'release-check-bundle.mjs'), 'windows'], bundleEnv)

  const collectMac = join(tmp, 'collected-macos')
  const collectWin = join(tmp, 'collected-windows')
  run([
    join(scripts, 'release-collect-assets.mjs'),
    collectMac,
    '0.1.0-beta.6',
    'macos',
  ], bundleEnv)
  run([
    join(scripts, 'release-collect-assets.mjs'),
    collectWin,
    '0.1.0-beta.6',
    'windows',
  ], bundleEnv)
  run([join(scripts, 'release-check-collected.mjs'), collectMac, 'macos'])
  run([join(scripts, 'release-check-collected.mjs'), collectWin, 'windows'])
  console.log('ok  check-bundle + collect + check-collected (macOS + Windows)')

  // publish job
  const publishRoot = join(tmp, 'publish', 'release-assets')
  seedCollected(publishRoot)
  run([join(scripts, 'release-check-collected.mjs'), publishRoot])
  run([
    join(scripts, 'release-write-latest-json.mjs'),
    publishRoot,
    '0.1.0-beta.6',
    'https://github.com/xuasir/xbxrc/releases/download/beta',
  ])
  const latest = JSON.parse(readFileSync(join(publishRoot, 'latest.json'), 'utf8'))
  assertIncludes(
    latest.platforms['windows-x86_64'].url,
    'xbxrc-0.1.0-beta.6-windows-setup.exe',
    'latest.json Windows url',
  )
  assertIncludes(
    latest.platforms['darwin-aarch64'].url,
    'xbxrc_0.1.0-beta.6_aarch64.app.tar.gz',
    'latest.json macOS url',
  )
  console.log('ok  CI publish job: check-collected + latest.json')

  console.log('\nrelease 全流程脚本校验通过（macOS + Windows）')
}
finally {
  rmSync(tmp, { recursive: true, force: true })
}
