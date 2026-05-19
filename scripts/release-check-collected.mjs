#!/usr/bin/env node
/**
 * 校验 release-collect-assets 输出，并确保 latest.json 能生成。
 * 用法: node scripts/release-check-collected.mjs <assetsRoot> [macos|windows]
 */
import { readdirSync } from 'node:fs'
import process from 'node:process'
import { checkReleaseCollected, COLLECTED_RELEASE_EXPECTATIONS } from './release-bundle-lib.mjs'

const assetsRoot = process.argv[2]
const onlyLabel = process.argv[3]

if (!assetsRoot) {
  console.error('用法: node scripts/release-check-collected.mjs <assetsRoot> [macos|windows]')
  process.exit(1)
}

const labels = onlyLabel ? [onlyLabel] : Object.keys(COLLECTED_RELEASE_EXPECTATIONS)
let failed = false

for (const label of labels) {
  const { dir, found, missing } = checkReleaseCollected(assetsRoot, label)
  if (!dir) {
    console.error(`[${label}] 缺少目录: ${assetsRoot}/release-assets-${label}`)
    failed = true
    continue
  }
  console.log(`[${label}] ${dir}`)
  for (const item of found) {
    console.log(`  ok  ${item.hint}: ${item.name}`)
  }
  if (missing.length > 0) {
    failed = true
    console.error(`[${label}] 缺少文件:`)
    for (const item of missing) {
      console.error(`  - ${item.hint} (*${item.suffix})`)
      console.error(`    当前: ${item.listing}`)
    }
  }
}

if (failed) {
  process.exit(1)
}

if (!onlyLabel) {
  const names = readdirSync(assetsRoot)
  const updaterReady = labels.every((label) => {
    const { missing } = checkReleaseCollected(assetsRoot, label)
    return missing.length === 0
  })
  if (!updaterReady) {
    process.exit(1)
  }
  console.log(`\n校验通过 (${names.length} 项于 ${assetsRoot})，可生成 latest.json`)
}
else {
  console.log('\n校验通过')
}
