#!/usr/bin/env node
import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { EDITION_POLICIES, repositoryRoot } from './release-edition.mjs'

const scripts = join(repositoryRoot, 'scripts')
const tmp = mkdtempSync(join(tmpdir(), 'xbxrc-release-pipeline-'))
try {
  for (const [edition, policy] of Object.entries(EDITION_POLICIES)) {
    const root = join(tmp, edition)
    mkdirSync(join(root, 'src-tauri'), { recursive: true })
    const version = `0.1.0-${policy.rollingTag}.8`
    writeFileSync(join(root, 'package.json'), JSON.stringify({ version }))
    writeFileSync(join(root, 'app-edition.json'), JSON.stringify({ edition, developmentBranch: policy.developmentBranch, releaseBranch: policy.releaseBranch, rollingTag: policy.rollingTag }))
    const config = JSON.parse(readFileSync(join(repositoryRoot, 'src-tauri/tauri.conf.json'), 'utf8'))
    config.version = version
    config.identifier = policy.identifier
    config.productName = policy.productName
    config.plugins.updater.endpoints = [edition === 'browser' ? 'https://github.com/xuasir/xbxrc/releases/latest/download/latest.json' : 'https://github.com/xuasir/xbxrc/releases/download/native-wip/latest.json']
    writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(config))
    execFileSync('git', ['init', '--quiet', root])
    execFileSync('git', ['-C', root, 'add', '.'])
    execFileSync('git', ['-C', root, '-c', 'user.name=Release fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'fixture'])
    const commit = execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
    const bundle = join(root, 'bundle')
    for (const subdir of ['macos', 'dmg', 'nsis'])
      mkdirSync(join(bundle, subdir), { recursive: true })
    for (const [path, contents] of [
      [`dmg/${policy.productName}_${version}_aarch64.dmg`, 'installer'],
      [`macos/${policy.productName}.app.tar.gz`, 'updater'],
      [`macos/${policy.productName}.app.tar.gz.sig`, 'sig-mac'],
      [`nsis/${policy.productName}_${version}_x64-setup.exe`, 'installer'],
      [`nsis/${policy.productName}_${version}_x64-setup.exe.sig`, 'sig-win'],
    ]) writeFileSync(join(bundle, path), contents)
    const env = { ...process.env, RELEASE_REPO_ROOT: root, RELEASE_BUNDLE_ROOT: bundle, GITHUB_SHA: commit }
    const run = (script, args = [], success = true) => {
      const result = spawnSync(process.execPath, [join(scripts, script), ...args], { encoding: 'utf8', env })
      if (success && result.status !== 0)
        throw new Error(`${script}: ${result.stdout}\n${result.stderr}`)
      if (!success)
        assert.notEqual(result.status, 0, `${script} 应拒绝错配产物`)
    }
    const assets = join(root, 'release-assets')
    for (const [label, arch] of [['macos', 'aarch64'], ['windows', 'x86_64']]) {
      const dir = join(assets, `release-assets-${label}`)
      run('release-check-bundle.mjs', [label])
      run('release-collect-assets.mjs', [dir, version, label, arch])
      run('release-check-collected.mjs', [dir, label])
    }
    run('release-check-collected.mjs', [assets])
    const baseUrl = `https://github.com/xuasir/xbxrc/releases/download/${policy.rollingTag}`
    run('release-write-latest-json.mjs', [assets, version, baseUrl])
    const latest = JSON.parse(readFileSync(join(assets, 'latest.json'), 'utf8'))
    assert.equal(latest.edition, edition)
    assert.equal(latest.identifier, policy.identifier)
    assert.equal(latest.sourceCommit, commit)
    assert.equal(Object.keys(latest.platforms).length, 2)
    assert.match(latest.platforms['darwin-aarch64'].url, /macos-aarch64\.app\.tar\.gz$/)
    assert.match(latest.platforms['windows-x86_64'].url, /windows-x86_64-setup\.exe$/)
    run('release-write-latest-json.mjs', [assets, version, 'https://github.com/xuasir/xbxrc/releases/download/wrong'], false)
    run('release-write-latest-json.mjs', [assets, version.replace('.8', '.9'), baseUrl], false)
    const manifestPath = join(assets, 'release-assets-windows/manifest.json')
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
    writeFileSync(manifestPath, JSON.stringify({ ...manifest, edition: 'wrong' }))
    run('release-check-collected.mjs', [assets], false)
    writeFileSync(manifestPath, JSON.stringify(manifest))
    writeFileSync(join(assets, 'release-assets-windows', manifest.files[0].name), 'tampered')
    run('release-write-latest-json.mjs', [assets, version, baseUrl], false)
    console.log(`ok ${edition}: bundle -> manifest -> latest.json；跨通道、版本、身份和内容错配被拒绝`)
  }
}
finally { rmSync(tmp, { recursive: true, force: true }) }
