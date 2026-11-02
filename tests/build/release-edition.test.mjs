import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { assetName, fileDigest, validateCollectedManifest } from '../../scripts/release-bundle-lib.mjs'
import { EDITION_POLICIES, readReleaseIdentity, validateDownloadBase, validateReleaseVersion, verifyReleaseSource } from '../../scripts/release-edition.mjs'

const identities = Object.entries(EDITION_POLICIES).map(([edition, policy]) => ({ ...policy, edition, version: '0.1.0' }))
const commit = 'a'.repeat(40)
for (const identity of identities) {
  test(`${identity.edition}: 开发、发行、PR与稳定tag来源`, () => {
    let checks = 0
    const isAncestor = branch => { assert.equal(branch, identity.developmentBranch); checks++; return true }
    verifyReleaseSource(identity, { ref: `refs/heads/${identity.developmentBranch}`, isAncestor })
    verifyReleaseSource(identity, { ref: `refs/heads/${identity.releaseBranch}`, isAncestor })
    verifyReleaseSource(identity, { ref: 'refs/pull/9/merge', baseRef: identity.developmentBranch, isAncestor })
    assert.equal(checks, 1)
    assert.throws(() => verifyReleaseSource(identity, { ref: `refs/heads/${identity.releaseBranch}`, isAncestor: () => false }))
    assert.throws(() => verifyReleaseSource(identity, { ref: 'refs/pull/9/merge', baseRef: 'wrong' }))
    assert.throws(() => verifyReleaseSource(identity, { ref: 'refs/heads/wrong' }))
    if (identity.edition === 'browser')
      verifyReleaseSource(identity, { ref: 'refs/tags/v0.1.0', isAncestor })
    else
      assert.throws(() => verifyReleaseSource(identity, { ref: 'refs/tags/v0.1.0', isAncestor }))
    for (const tag of ['v0.1.0-beta.2', 'v0.1.0-native-wip.3', 'v0.1.1', 'v01.1.0'])
      assert.throws(() => verifyReleaseSource(identity, { ref: `refs/tags/${tag}`, isAncestor }))
  })

  test(`${identity.edition}: 版本和下载通道`, () => {
    const version = `0.1.0-${identity.rollingTag}.8`
    assert.equal(validateReleaseVersion(identity, version), identity.rollingTag)
    validateDownloadBase(identity, version, `https://github.com/xuasir/xbxrc/releases/download/${identity.rollingTag}`)
    assert.throws(() => validateDownloadBase(identity, version, 'https://github.com/xuasir/xbxrc/releases/latest/download'))
    const other = identity.edition === 'browser' ? 'native-wip' : 'beta'
    assert.throws(() => validateReleaseVersion(identity, `0.1.0-${other}.8`))
    for (const bad of [`0.1.0-${identity.rollingTag}.0`, '01.1.0', '0.1.0-rc.1', '0.1.0+foo'])
      assert.throws(() => validateReleaseVersion(identity, bad))
    if (identity.edition === 'browser') {
      assert.equal(validateReleaseVersion(identity, '0.1.0'), 'stable')
      validateDownloadBase(identity, '0.1.0', 'https://github.com/xuasir/xbxrc/releases/download/v0.1.0')
    }
    else {
      assert.throws(() => validateReleaseVersion(identity, '0.1.0'))
    }
  })

  test(`${identity.edition}: manifest身份、哈希与目录闭包`, () => {
    const root = mkdtempSync(join(tmpdir(), 'xbxrc-release-gate-'))
    try {
      const version = `0.1.0-${identity.rollingTag}.8`
      const files = ['installer', 'updater', 'signature'].map(kind => {
        const name = assetName(identity, version, 'macos', 'aarch64', kind)
        const path = join(root, name)
        writeFileSync(path, `mock-${kind}`)
        return { name, kind, bytes: readFileSync(path).length, sha256: fileDigest(path) }
      })
      const valid = { schemaVersion: 1, version, label: 'macos', architecture: 'aarch64', edition: identity.edition, identifier: identity.identifier, sourceCommit: commit, features: identity.features, files }
      const save = value => writeFileSync(join(root, 'manifest.json'), JSON.stringify(value))
      const check = () => validateCollectedManifest(root, 'macos', identity, commit)
      save(valid)
      check()
      for (const change of [{ edition: 'wrong' }, { identifier: 'wrong' }, { sourceCommit: 'b'.repeat(40) }, { architecture: 'x86' }, { features: ['wrong'] }, { files: [files[0]] }]) {
        save({ ...valid, ...change })
        assert.throws(check)
      }
      save(valid)
      writeFileSync(join(root, files[0].name), 'tampered')
      assert.throws(check)
      writeFileSync(join(root, files[0].name), 'mock-installer')
      check()
      writeFileSync(join(root, 'extra.exe'), 'extra')
      assert.throws(check)
    }
    finally { rmSync(root, { recursive: true, force: true }) }
  })

  test(`${identity.edition}: 应用配置身份校验`, () => {
    const root = mkdtempSync(join(tmpdir(), 'xbxrc-edition-'))
    try {
      mkdirSync(join(root, 'src-tauri'))
      const app = { edition: identity.edition, developmentBranch: identity.developmentBranch, releaseBranch: identity.releaseBranch, rollingTag: identity.rollingTag }
      const config = { version: '0.1.0', identifier: identity.identifier, productName: identity.productName, bundle: { macOS: { minimumSystemVersion: '26.0' } }, plugins: { updater: { endpoints: [identity.edition === 'browser' ? 'https://github.com/xuasir/xbxrc/releases/latest/download/latest.json' : 'https://github.com/xuasir/xbxrc/releases/download/native-wip/latest.json'] } } }
      writeFileSync(join(root, 'app-edition.json'), JSON.stringify(app))
      writeFileSync(join(root, 'package.json'), JSON.stringify({ version: '0.1.0' }))
      writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(config))
      assert.equal(readReleaseIdentity(root).edition, identity.edition)
      config.identifier = 'wrong'
      writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(config))
      assert.throws(() => readReleaseIdentity(root))
    }
    finally { rmSync(root, { recursive: true, force: true }) }
  })
}
