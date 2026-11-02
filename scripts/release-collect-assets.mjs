#!/usr/bin/env node
import { copyFileSync, mkdirSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { assetName, checkReleaseBundle, fileDigest, validateCollectedManifest } from './release-bundle-lib.mjs'
import { readReleaseIdentity, sourceCommit, validateReleaseVersion } from './release-edition.mjs'

const outDir = resolve(process.argv[2] ?? 'release-assets')
const version = process.argv[3]
const label = process.argv[4]
const architecture = process.argv[5]
const identity = readReleaseIdentity()
validateReleaseVersion(identity, version)
if (version !== identity.version)
  throw new Error('收集版本必须等于实际构建版本')
if (!['macos', 'windows'].includes(label) || !architecture)
  throw new Error('用法: node scripts/release-collect-assets.mjs <outDir> <version> <macos|windows> <aarch64|x86_64>')
const { found, missing } = checkReleaseBundle(label)
if (missing.length)
  throw new Error(`收集前缺少产物: ${missing.map(item => item.hint).join(', ')}`)
mkdirSync(outDir, { recursive: true })
if (readdirSync(outDir).length)
  throw new Error('收集目录必须为空')
const kinds = label === 'macos' ? ['installer', 'updater', 'signature'] : ['updater', 'signature']
const files = found.map((file, index) => {
  const kind = kinds[index]
  const name = assetName(identity, version, label, architecture, kind)
  const dest = join(outDir, name)
  copyFileSync(file.path, dest)
  return { name, kind, bytes: statSync(dest).size, sha256: fileDigest(dest) }
})
const commit = sourceCommit()
const manifest = {
  schemaVersion: 1, version, label, architecture, edition: identity.edition,
  identifier: identity.identifier, sourceCommit: commit, features: identity.features, files,
}
writeFileSync(join(outDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
validateCollectedManifest(outDir, label, identity, commit)
console.log(`已收集 ${identity.edition} ${version} ${label}/${architecture} (${commit})`)
