#!/usr/bin/env node
/**
 * 本地 / CI：校验 tauri build 产物是否满足发布收集要求（不复制文件）。
 * 用法: node scripts/release-check-bundle.mjs <macos|windows>
 */
import process from 'node:process'
import { checkReleaseBundle } from './release-bundle-lib.mjs'

const label = process.argv[2]
if (!label) {
  console.error('用法: node scripts/release-check-bundle.mjs <macos|windows>')
  process.exit(1)
}

try {
  const { bundleRoot, found, missing } = checkReleaseBundle(label)
  console.log(`bundle root: ${bundleRoot}`)
  for (const item of found) {
    console.log(`ok  ${item.hint}: ${item.name}`)
  }
  if (missing.length > 0) {
    console.error('\n缺少发布所需产物:')
    for (const item of missing) {
      console.error(`  - ${item.hint}`)
      console.error(`    目录: ${item.dir}`)
      console.error(`    后缀: *${item.suffix}`)
      console.error(`    当前: ${item.listing}`)
    }
    if (label === 'macos') {
      console.error('\n提示: macOS updater 需要 app bundle，请执行:')
      console.error('  pnpm tauri:build:release:macos')
    }
    else {
      console.error('\n提示: 请执行: pnpm tauri:build:release:windows')
    }
    process.exit(1)
  }
  console.log('\n校验通过，可执行 release-collect-assets.mjs')
}
catch (error) {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
}
