import { constants } from 'node:fs'
import { access, mkdir, mkdtemp, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { isAbsolute, join, resolve } from 'node:path'

export const CAPTURE_HELP = `用法：
  node scripts/official-xbox-capture.mjs --browser-path PATH [选项]
  node scripts/official-xbox-capture.mjs --no-launch [--port PORT] [选项]

选项：
  --browser-path PATH       Chromium 浏览器可执行文件的绝对路径
  --no-launch               连接已经启用远程调试的浏览器
  --port PORT               远程调试端口，默认 9222
  --profile DIR             显式指定 profile；启动时默认创建独立系统临时目录
  --out DIR                 采集输出目录，默认 runtime-logs/official-xbox-capture-时间戳
  --url URL                 采集页面，默认 https://www.xbox.com/play
  --sample-interval-ms MS    采样间隔，默认 2000 毫秒
  --help, -h                显示帮助
`

export function parseCaptureArgs(values) {
  if (values.includes('--help') || values.includes('-h'))
    return { help: true }
  const parsed = { noLaunch: false, port: 9222, sampleIntervalMs: 2000, url: 'https://www.xbox.com/play' }
  const options = {
    'browser-path': 'browserPath',
    'out': 'out',
    'port': 'port',
    'profile': 'profile',
    'url': 'url',
    'sample-interval-ms': 'sampleIntervalMs',
  }
  for (let index = 0; index < values.length; index++) {
    const option = values[index]
    if (option === '--no-launch') {
      parsed.noLaunch = true
      continue
    }
    const name = option.slice(2)
    if (!option.startsWith('--') || !Object.hasOwn(options, name))
      throw new Error(`未知参数：${option}`)
    const key = options[name]
    const value = values[++index]
    if (!value || value.startsWith('--'))
      throw new Error(`${option} 需要一个值`)
    parsed[key] = value
  }
  parsed.port = Number(parsed.port)
  parsed.sampleIntervalMs = Number(parsed.sampleIntervalMs)
  if (!Number.isInteger(parsed.port) || parsed.port < 1 || parsed.port > 65535)
    throw new Error('--port 必须为 1 到 65535 的整数')
  if (!Number.isInteger(parsed.sampleIntervalMs) || parsed.sampleIntervalMs < 1 || parsed.sampleIntervalMs > 2_147_483_647)
    throw new Error('--sample-interval-ms 必须为有效的正整数毫秒数')
  const url = new URL(parsed.url)
  if (!['http:', 'https:'].includes(url.protocol))
    throw new Error('--url 必须使用 http 或 https')
  if (!parsed.noLaunch && !parsed.browserPath)
    throw new Error('启动采集需要 --browser-path PATH；连接已有浏览器可使用 --no-launch')
  return parsed
}

export async function validateBrowserPath({ browserPath }) {
  if (!browserPath)
    return
  if (!isAbsolute(browserPath))
    throw new Error('--browser-path 必须为绝对路径')
  let info
  try {
    info = await stat(browserPath)
    await access(browserPath, constants.X_OK)
  }
  catch (cause) {
    throw new Error(`浏览器文件必须存在且可执行：${browserPath}`, { cause })
  }
  if (!info.isFile())
    throw new Error(`浏览器路径必须指向普通文件：${browserPath}`)
}

export async function captureProfileDirectory({ profile, noLaunch }) {
  if (noLaunch)
    return profile ? resolve(profile) : null
  if (profile) {
    const directory = resolve(profile)
    await mkdir(directory, { recursive: true })
    return directory
  }
  return mkdtemp(join(tmpdir(), 'xbxrc-official-xbox-profile-'))
}
