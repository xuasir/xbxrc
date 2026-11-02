#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { checkReleaseCollected, validateCollectedManifest } from './release-bundle-lib.mjs'
import { readReleaseIdentity, sourceCommit, validateDownloadBase } from './release-edition.mjs'

const assetsRoot = resolve(process.argv[2] ?? 'release-assets')
const version = process.argv[3]
const downloadBase = process.argv[4]?.replace(/\/$/, '')
const identity = readReleaseIdentity()
validateDownloadBase(identity, version, downloadBase)
const commit = sourceCommit()
const platforms = {}
for (const label of ['macos', 'windows']) {
  const { dir, missing } = checkReleaseCollected(assetsRoot, label, identity, commit)
  if (missing.length)
    throw new Error(`${label} 缺少 updater 资产`)
  const manifest = validateCollectedManifest(dir, label, identity, commit)
  if (manifest.version !== version)
    throw new Error(`${label} 产物版本不符`)
  const bundle = manifest.files.find(file => file.kind === 'updater')
  const sig = manifest.files.find(file => file.kind === 'signature')
  const platform = `${label === 'macos' ? 'darwin' : 'windows'}-${manifest.architecture}`
  platforms[platform] = {
    signature: readFileSync(join(dir, sig.name), 'utf8').trim(),
    url: `${downloadBase}/${bundle.name}`,
  }
}
const latest = {
  version, edition: identity.edition, identifier: identity.identifier, sourceCommit: commit,
  notes: '', pub_date: new Date().toISOString(), platforms,
}
const path = join(assetsRoot, 'latest.json')
writeFileSync(path, `${JSON.stringify(latest, null, 2)}\n`)
console.log(`已写入 ${path}`)
