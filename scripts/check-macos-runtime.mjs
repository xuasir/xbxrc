#!/usr/bin/env node
import { resolve } from 'node:path'
import { checkMacosRuntime } from './macos-runtime-lib.mjs'

try {
  const target = process.argv[2]
  if (process.platform !== 'darwin' || !target)
    throw new Error('用法（macOS）: node scripts/check-macos-runtime.mjs <app目录> [最低系统版本]')
  const result = checkMacosRuntime(resolve(target), { minimumSystemVersion: process.argv[3] })
  console.log(JSON.stringify(result, null, 2))
}
catch (error) {
  console.error(error instanceof Error ? error.message : error)
  process.exitCode = 1
}
