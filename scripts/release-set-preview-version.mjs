#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { BASE_VERSION, readReleaseIdentity, repositoryRoot } from './release-edition.mjs'

const identity = readReleaseIdentity()
const run = process.argv[2]
if (!BASE_VERSION.test(identity.version) || !/^[1-9]\d*$/.test(run ?? ''))
  throw new Error('用法: node scripts/release-set-preview-version.mjs <正整数run>；仓库保持基础版本')
const version = `${identity.version}-${identity.rollingTag}.${run}`
for (const path of ['package.json', 'src-tauri/tauri.conf.json']) {
  const absolute = resolve(repositoryRoot, path)
  const config = JSON.parse(readFileSync(absolute, 'utf8'))
  config.version = version
  writeFileSync(absolute, `${JSON.stringify(config, null, 2)}\n`)
}
if (process.env.GITHUB_OUTPUT)
  appendFileSync(process.env.GITHUB_OUTPUT, `value=${version}\n`)
console.log(version)
