import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

export const EDITION_POLICIES = {
  browser: {
    developmentBranch: 'main', releaseBranch: 'release/test', rollingTag: 'beta',
    identifier: 'com.xuasir.xbxrc', productName: 'xbxrc', artifactPrefix: 'xbxrc',
    features: [],
  },
  'native-wip': {
    developmentBranch: 'rust', releaseBranch: 'release/native-wip', rollingTag: 'native-wip',
    identifier: 'com.xuasir.xbxrc.native-wip', productName: 'xbxrc Native WIP', artifactPrefix: 'xbxrc-native-wip',
    features: ['native-media', 'native-gamepad'],
  },
}

export const BASE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/
export const repositoryRoot = process.env.RELEASE_REPO_ROOT ?? resolve(import.meta.dirname, '..')

export function readReleaseIdentity(root = repositoryRoot) {
  const manifest = JSON.parse(readFileSync(resolve(root, 'app-edition.json'), 'utf8'))
  const config = JSON.parse(readFileSync(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'))
  const pkg = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'))
  const policy = EDITION_POLICIES[manifest.edition]
  if (!policy)
    throw new Error(`未知 edition: ${manifest.edition}`)
  for (const field of ['developmentBranch', 'releaseBranch', 'rollingTag']) {
    if (manifest[field] !== policy[field])
      throw new Error(`edition 的 ${field} 必须为 ${policy[field]}`)
  }
  if (config.identifier !== policy.identifier || config.productName !== policy.productName)
    throw new Error(`应用身份与 ${manifest.edition} edition 不符`)
  if (pkg.version !== config.version)
    throw new Error('package.json 与 tauri.conf.json 的版本必须一致')
  if (config.bundle.macOS.minimumSystemVersion !== '26.0')
    throw new Error('应用最低 macOS 版本必须保持 26.0')
  const endpoint = config.plugins.updater.endpoints
  const expectedEndpoint = manifest.edition === 'browser'
    ? 'https://github.com/xuasir/xbxrc/releases/latest/download/latest.json'
    : 'https://github.com/xuasir/xbxrc/releases/download/native-wip/latest.json'
  if (endpoint.length !== 1 || endpoint[0] !== expectedEndpoint)
    throw new Error('应用更新源与 edition 不符')
  return { ...policy, edition: manifest.edition, version: pkg.version }
}

export function validateReleaseVersion(identity, version) {
  if (BASE_VERSION.test(version)) {
    if (identity.edition !== 'browser')
      throw new Error('Native WIP 发行必须使用 native-wip 预览版本')
    return 'stable'
  }
  const suffix = `-${identity.rollingTag}.`
  const index = version.indexOf(suffix)
  if (index < 0 || !BASE_VERSION.test(version.slice(0, index)) || !/^[1-9]\d*$/.test(version.slice(index + suffix.length)))
    throw new Error(`发行版本与 ${identity.edition} 不符: ${version}`)
  return identity.rollingTag
}

export function git(root, args) {
  return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
}

export function sourceCommit(root = repositoryRoot) {
  const commit = git(root, ['rev-parse', 'HEAD'])
  if (!/^[a-f0-9]{40}$/.test(commit) || (process.env.GITHUB_SHA && commit !== process.env.GITHUB_SHA))
    throw new Error('源码提交与 CI 来源不符')
  return commit
}

// 分支来源检查在 CI 改写预览版本之前执行。
export function verifyReleaseSource(identity, {
  ref = process.env.GITHUB_REF ?? `refs/heads/${git(repositoryRoot, ['branch', '--show-current'])}`,
  baseRef = process.env.GITHUB_BASE_REF,
  isAncestor = branch => {
    execFileSync('git', ['-C', repositoryRoot, 'merge-base', '--is-ancestor', 'HEAD', `origin/${branch}`])
    return true
  },
} = {}) {
  if (!BASE_VERSION.test(identity.version))
    throw new Error('仓库版本必须保持基础 X.Y.Z')
  if (ref.startsWith('refs/pull/')) {
    if (baseRef !== identity.developmentBranch)
      throw new Error('PR 目标分支与 edition 不符')
    return
  }
  if (ref === `refs/heads/${identity.developmentBranch}`)
    return
  const release = ref === `refs/heads/${identity.releaseBranch}`
  const stable = identity.edition === 'browser' && ref === `refs/tags/v${identity.version}`
  if (!release && !stable)
    throw new Error(`发行来源与 ${identity.edition} 不符: ${ref}`)
  if (!isAncestor(identity.developmentBranch))
    throw new Error(`发行提交必须来自 ${identity.developmentBranch}`)
}

export function verifyBrowserSource(root = repositoryRoot) {
  const nativePaths = [
    'crates/xbxengine', 'crates/ohmygamepad', 'src-tauri/resources/ffmpeg',
    'src-tauri/src/mods/xbxengine', 'src-tauri/src/mods/native_video', 'src-tauri/src/mods/gamepad',
    'src/shared/gamepad/native-backend.ts', 'src/streaming/runtime/rust-runtime.ts',
    'scripts/with-ffmpeg-sdk.mjs',
  ]
  for (const path of nativePaths) {
    if (existsSync(resolve(root, path)))
      throw new Error(`Browser 源码边界包含原生 WIP: ${path}`)
  }
  const tree = execFileSync('cargo', ['tree', '-p', 'xbxrc', '--edges', 'normal', '--locked', '--offline', '--prefix', 'none'], {
    cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  })
  const forbidden = /^(?:xbxengine(?:-\S*)?|ohmygamepad-\S*|sdl\S*|wgpu\S*|ffmpeg\S*|cpal|opus\S*|rtc(?:-\S*)?)\s/m
  if (forbidden.test(tree))
    throw new Error(`Browser 依赖树包含原生 WIP: ${tree.match(forbidden)[0].trim()}`)
}

export function validateDownloadBase(identity, version, downloadBase) {
  const channel = validateReleaseVersion(identity, version)
  const tag = channel === 'stable' ? `v${version}` : identity.rollingTag
  if (downloadBase !== `https://github.com/xuasir/xbxrc/releases/download/${tag}`)
    throw new Error(`更新下载地址必须归属 ${tag}`)
}
