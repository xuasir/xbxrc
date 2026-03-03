import type { StartupFlags } from '../../domain/types'

function resolveAutoConnectFromArg(argv: readonly string[]): string | undefined {
  for (const entry of argv) {
    if (!entry.startsWith('--auto-connect=')) {
      continue
    }
    const value = entry.slice('--auto-connect='.length).trim()
    if (value !== '') {
      return value
    }
  }
  return undefined
}

/**
 * 解析启动参数
 * - 命令行优先于配置，避免 legacy 中循环覆盖导致优先级混乱
 */
export function resolveStartupFlags(
  argv: readonly string[],
  fallbackAutoConnect: string
): StartupFlags {
  const hasFullscreenArg = argv.some((entry) => entry.includes('--fullscreen'))
  const fromArg = resolveAutoConnectFromArg(argv)

  return {
    fullscreen: hasFullscreenArg,
    autoConnect: fromArg ?? fallbackAutoConnect.trim()
  }
}
