import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

// 夹具显式持有 Git 配置，子进程共享同一份环境与空 hooks/template。
export function fixtureGitEnvironment(directory, parentEnvironment = process.env) {
  mkdirSync(directory, { recursive: true })
  const hooks = join(directory, 'hooks')
  const template = join(directory, 'template')
  const ignore = join(directory, 'ignore')
  const attributes = join(directory, 'attributes')
  const config = join(directory, 'config')
  mkdirSync(hooks)
  mkdirSync(template)
  writeFileSync(ignore, '')
  writeFileSync(attributes, '')
  const pathValue = path => JSON.stringify(path.replaceAll('\\', '/'))
  writeFileSync(config, `[user]
  name = Release fixture
  email = fixture@example.invalid
[commit]
  gpgSign = false
[tag]
  gpgSign = false
[core]
  hooksPath = ${pathValue(hooks)}
  excludesFile = ${pathValue(ignore)}
  attributesFile = ${pathValue(attributes)}
  autocrlf = false
  fsmonitor = false
[init]
  templateDir = ${pathValue(template)}
  defaultBranch = main
`)
  const environment = Object.fromEntries(
    Object.entries(parentEnvironment).filter(([key]) => !/^GIT_/i.test(key)),
  )
  return {
    ...environment,
    GIT_CONFIG_GLOBAL: config,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_ATTR_NOSYSTEM: '1',
    GIT_TERMINAL_PROMPT: '0',
  }
}
