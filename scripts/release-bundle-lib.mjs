import { existsSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

export function resolveBundleRoot(root) {
  const candidates = [
    join(root, 'target', 'release', 'bundle'),
    join(root, 'src-tauri', 'target', 'release', 'bundle'),
  ]
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
  return (
    readdirSync(dir).find((name) => {
      if (!name.endsWith(suffix)) {
        return false
      }
      if (excludeSuffix && name.endsWith(excludeSuffix)) {
        return false
      }
      return true
    }) ?? null
  )
}

/** 收集后的 artifact 目录（release-assets-<label>/）应有文件 */
export const COLLECTED_RELEASE_EXPECTATIONS = {
  macos: [
    { suffix: '-macos.dmg', hint: 'macOS 安装包' },
    { suffix: '.tar.gz', hint: 'updater 包', excludeSuffix: '.tar.gz.sig' },
    { suffix: '.tar.gz.sig', hint: 'updater 签名' },
  ],
  windows: [
    { suffix: '-windows-setup.exe', hint: 'Windows 安装包', excludeSuffix: '.exe.sig' },
    { suffix: '-windows-setup.exe.sig', hint: 'updater 签名' },
  ],
}

export function resolveCollectedDir(assetsRoot, label) {
  const candidates = [
    join(assetsRoot, label),
    join(assetsRoot, `release-assets-${label}`),
  ]
  return candidates.find(path => existsSync(path)) ?? null
}

export function checkReleaseCollected(assetsRoot, label) {
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
  return { dir, found, missing }
}

export function checkReleaseBundle(label, root = resolve(import.meta.dirname, '..')) {
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
