import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'

test('发行夹具在 hooks、模板、忽略规则与 Git 环境干扰下完成两版型验证', () => {
  const directory = mkdtempSync(join(tmpdir(), 'xbxrc-hostile-git-'))
  try {
    const hooks = join(directory, 'hooks')
    const template = join(directory, 'template')
    const ignore = join(directory, 'ignore')
    const config = join(directory, 'config')
    mkdirSync(hooks)
    mkdirSync(join(template, 'hooks'), { recursive: true })
    const failedHook = '#!/bin/sh\nexit 1\n'
    writeFileSync(join(hooks, 'pre-commit'), failedHook, { mode: 0o755 })
    writeFileSync(join(template, 'hooks/pre-commit'), failedHook, { mode: 0o755 })
    writeFileSync(ignore, '*\n')
    const pathValue = path => JSON.stringify(path.replaceAll('\\', '/'))
    writeFileSync(config, `[core]
  hooksPath = ${pathValue(hooks)}
  excludesFile = ${pathValue(ignore)}
  autocrlf = true
[init]
  templateDir = ${pathValue(template)}
  defaultBranch = hostile-fixture
[commit]
  gpgSign = true
[gpg]
  program = missing-fixture-signer
`)
    const poisonedRepository = join(directory, 'missing-repository')
    const result = spawnSync(process.execPath, [resolve(import.meta.dirname, '../../scripts/release-verify-pipeline.mjs')], {
      encoding: 'utf8',
      timeout: 120_000,
      env: {
        ...process.env,
        GIT_CONFIG_GLOBAL: config,
        git_config_global: config,
        GIT_CONFIG_SYSTEM: config,
        GIT_CONFIG_NOSYSTEM: '0',
        GIT_CONFIG_COUNT: '1',
        GIT_CONFIG_KEY_0: 'core.hooksPath',
        GIT_CONFIG_VALUE_0: hooks,
        GIT_CONFIG_PARAMETERS: "'commit.gpgsign=true'",
        GIT_TEMPLATE_DIR: template,
        GIT_DIR: join(poisonedRepository, '.git'),
        GIT_COMMON_DIR: join(poisonedRepository, '.git'),
        GIT_WORK_TREE: poisonedRepository,
        GIT_INDEX_FILE: join(poisonedRepository, 'index'),
        GIT_OBJECT_DIRECTORY: join(poisonedRepository, 'objects'),
        GIT_ALTERNATE_OBJECT_DIRECTORIES: join(poisonedRepository, 'alternate-objects'),
        GIT_NAMESPACE: 'hostile-fixture',
        GIT_AUTHOR_NAME: 'Hostile fixture',
        GIT_AUTHOR_EMAIL: 'hostile@example.invalid',
        GIT_COMMITTER_DATE: 'invalid-fixture-date',
        RELEASE_REPO_ROOT: poisonedRepository,
        RELEASE_BUNDLE_ROOT: poisonedRepository,
        GITHUB_SHA: 'invalid-fixture-commit',
      },
    })
    assert.equal(result.status, 0, `${result.error ?? ''}\n${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /ok browser:/)
    assert.match(result.stdout, /ok native-wip:/)
  }
  finally { rmSync(directory, { recursive: true, force: true }) }
})
