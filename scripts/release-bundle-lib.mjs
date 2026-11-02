import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import { readReleaseIdentity, repositoryRoot, sourceCommit, validateReleaseVersion } from './release-edition.mjs'

export function resolveBundleRoot(root) {
  const candidates = process.env.RELEASE_BUNDLE_ROOT
    ? [resolve(process.env.RELEASE_BUNDLE_ROOT)]
    : process.env.CARGO_TARGET_DIR
      ? [resolve(root, process.env.CARGO_TARGET_DIR, 'release/bundle')]
      : [resolve(root, 'target', readReleaseIdentity(root).edition, 'release/bundle')]
  const hit = candidates.find(path => existsSync(path))
  if (!hit) {
    throw new Error(
      `未找到 bundle 目录，已尝试:\n${candidates.map(path => `  - ${path}`).join('\n')}`,
    )
  }
  return hit
}

/** @type {Record<string, Array<{ subdir: string, suffix: string, hint: string }>>} */
export const RELEASE_BUNDLE_EXPECTATIONS = {
  macos: [
    { subdir: 'dmg', suffix: '.dmg', hint: 'macOS 安装包' },
    { subdir: 'macos', suffix: '.tar.gz', hint: 'updater 包（需 --bundles app,dmg 且已配置签名密钥）' },
    { subdir: 'macos', suffix: '.tar.gz.sig', hint: 'updater 签名' },
  ],
  windows: [
    { subdir: 'nsis', suffix: '-setup.exe', hint: 'Windows 安装包' },
    { subdir: 'nsis', suffix: '-setup.exe.sig', hint: 'updater 签名' },
  ],
}

export function findBundleFile(dir, suffix, options = {}) {
  if (!existsSync(dir)) {
    return null
  }
  const { excludeSuffix } = options
  const matches = readdirSync(dir).filter((name) => {
      if (!name.endsWith(suffix)) {
        return false
      }
      if (excludeSuffix && name.endsWith(excludeSuffix)) {
        return false
      }
      return statSync(join(dir, name)).isFile()
    })
  if (matches.length > 1)
    throw new Error(`产物目录存在多个 *${suffix}，请清理旧产物: ${matches.join(', ')}`)
  return matches[0] ?? null
}

/** 收集后的 artifact 目录（release-assets-<label>/）应有文件 */
export const COLLECTED_RELEASE_EXPECTATIONS = {
  macos: [
    { suffix: '.dmg', hint: 'macOS 安装包' },
    { suffix: '.tar.gz', hint: 'updater 包', excludeSuffix: '.tar.gz.sig' },
    { suffix: '.tar.gz.sig', hint: 'updater 签名' },
  ],
  windows: [
    { suffix: '-setup.exe', hint: 'Windows 安装包', excludeSuffix: '.exe.sig' },
    { suffix: '-setup.exe.sig', hint: 'updater 签名' },
  ],
}

function dirHasCollectedFiles(dir, label) {
  const expectations = COLLECTED_RELEASE_EXPECTATIONS[label]
  if (!expectations || !existsSync(dir)) {
    return false
  }
  return expectations.every(item => findBundleFile(dir, item.suffix, item))
}

export function resolveCollectedDir(assetsRoot, label) {
  const root = resolve(assetsRoot)
  const platformDirName = `release-assets-${label}`

  // build job：collect 写入的目录（通常为 release-assets-<label>/）
  if (basename(root) === platformDirName && existsSync(root)) {
    return root
  }

  // 传入路径本身就是已收集的平台目录
  if (dirHasCollectedFiles(root, label)) {
    return root
  }

  // publish job：download-artifact 后为 release-assets/release-assets-macos/
  const nested = join(root, platformDirName)
  if (existsSync(nested)) {
    return nested
  }

  const flat = join(root, label)
  if (existsSync(flat)) {
    return flat
  }

  return null
}

export function checkReleaseCollected(assetsRoot, label, identity = readReleaseIdentity(), commit = sourceCommit()) {
  const dir = resolveCollectedDir(assetsRoot, label)
  if (!dir) {
    return {
      dir: null,
      found: [],
      missing: (COLLECTED_RELEASE_EXPECTATIONS[label] ?? []).map(item => ({
        ...item,
        dir: assetsRoot,
        listing: '(未找到收集目录)',
      })),
    }
  }

  const expectations = COLLECTED_RELEASE_EXPECTATIONS[label]
  if (!expectations) {
    throw new Error(`未知 label: ${label}`)
  }

  const missing = []
  const found = []
  for (const item of expectations) {
    const hit = findBundleFile(dir, item.suffix, item)
    if (hit) {
      found.push({ ...item, path: join(dir, hit), name: hit })
    }
    else {
      const listing = readdirSync(dir).join(', ') || '(空)'
      missing.push({ ...item, dir, listing })
    }
  }
  if (missing.length === 0)
    validateCollectedManifest(dir, label, identity, commit)
  return { dir, found, missing }
}

export function assetName(identity, version, label, architecture, kind) {
  const base = `${identity.artifactPrefix}-${version}-${label}-${architecture}`
  const suffix = label === 'macos'
    ? { installer: '.dmg', updater: '.app.tar.gz', signature: '.app.tar.gz.sig' }
    : { updater: '-setup.exe', signature: '-setup.exe.sig' }
  if (!suffix[kind])
    throw new Error(`未知 ${label} 资产类型: ${kind}`)
  return `${base}${suffix[kind]}`
}

export function fileDigest(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

export function validateCollectedManifest(dir, label, identity, commit) {
  const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8'))
  if (manifest.schemaVersion !== 1 || manifest.edition !== identity.edition
    || manifest.identifier !== identity.identifier || manifest.sourceCommit !== commit
    || manifest.label !== label || JSON.stringify(manifest.features) !== JSON.stringify(identity.features))
    throw new Error(`${label} 产物清单的 edition、应用身份、features 或源码提交不符`)
  validateReleaseVersion(identity, manifest.version)
  const allowedArchitectures = label === 'macos' ? ['aarch64', 'x86_64'] : ['x86_64']
  if (!allowedArchitectures.includes(manifest.architecture))
    throw new Error(`${label} 产物架构不符: ${manifest.architecture}`)
  const kinds = label === 'macos' ? ['installer', 'updater', 'signature'] : ['updater', 'signature']
  if (!Array.isArray(manifest.files) || manifest.files.length !== kinds.length)
    throw new Error('产物清单必须完整且唯一')
  const files = []
  for (const kind of kinds) {
    const entry = manifest.files.find(file => file.kind === kind)
    const name = assetName(identity, manifest.version, label, manifest.architecture, kind)
    if (!entry || entry.name !== name)
      throw new Error(`产物文件名与清单不符: ${name}`)
    const path = join(dir, name)
    if (!statSync(path).isFile() || entry.bytes !== statSync(path).size || entry.sha256 !== fileDigest(path))
      throw new Error(`产物内容与清单哈希不符: ${name}`)
    if (kind === 'signature' && !readFileSync(path, 'utf8').trim())
      throw new Error(`updater 签名为空: ${name}`)
    files.push(name)
  }
  const actual = readdirSync(dir).sort()
  if (JSON.stringify(actual) !== JSON.stringify([...files, 'manifest.json'].sort()))
    throw new Error('产物目录包含清单以外的文件')
  return manifest
}

export function checkReleaseBundle(label, root = repositoryRoot) {
  const bundleRoot = resolveBundleRoot(root)
  const expectations = RELEASE_BUNDLE_EXPECTATIONS[label]
  if (!expectations) {
    throw new Error(`未知 label: ${label}`)
  }

  const missing = []
  const found = []

  for (const item of expectations) {
    const dir = join(bundleRoot, item.subdir)
    const hit = findBundleFile(dir, item.suffix)
    if (hit) {
      found.push({ ...item, path: join(dir, hit), name: hit })
    }
    else {
      const listing = existsSync(dir) ? readdirSync(dir).join(', ') : '(目录不存在)'
      missing.push({ ...item, dir, listing })
    }
  }

  return { bundleRoot, found, missing }
}
