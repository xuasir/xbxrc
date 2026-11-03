import type { UpdaterProgressEvent } from '@shared/events/updater'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const fixture = vi.hoisted(() => ({
  getVersion: vi.fn(),
  getChannel: vi.fn(),
  setChannel: vi.fn(),
  check: vi.fn(),
  download: vi.fn(),
  relaunch: vi.fn(),
  on: vi.fn(),
  progress: undefined as ((event: UpdaterProgressEvent) => void) | undefined,
}))
vi.mock('../services/rpc', () => ({ rpc: {
  app: { getVersion: fixture.getVersion },
  updater: { getChannel: fixture.getChannel, setChannel: fixture.setChannel, check: fixture.check, downloadAndInstall: fixture.download, relaunch: fixture.relaunch },
} }))
vi.mock('../services/events', () => ({ events: { on: fixture.on } }))

beforeEach(() => {
  vi.resetModules()
  vi.clearAllMocks()
  fixture.getVersion.mockResolvedValue('1.0.0')
  fixture.getChannel.mockResolvedValue({ channel: 'stable' })
  fixture.setChannel.mockResolvedValue({ channel: 'beta' })
  fixture.check.mockResolvedValue({ currentVersion: '1.0.0', updateAvailable: false })
  fixture.download.mockResolvedValue(undefined)
  fixture.relaunch.mockResolvedValue(undefined)
  fixture.on.mockImplementation((_name, listener) => {
    fixture.progress = listener
    return () => {}
  })
})
const available = { currentVersion: '1.0.0', updateAvailable: true, version: '1.1.0', notes: 'Changes', date: '2026-11-03' }
async function updater() {
  const module = await import('./useAppUpdater')
  const api = module.useAppUpdater()
  await Promise.resolve()
  return { api, use: module.useAppUpdater }
}

describe('application updater lifecycle', () => {
  it.each(['', ' \n ', null])('keeps release notes absent for an empty manifest value %j', async (notes) => {
    fixture.check.mockResolvedValue({ ...available, notes })
    const { api } = await updater()
    await api.checkForUpdate()
    expect(api.state.value).toBe('available')
    expect(api.releaseNotes.value).toBeNull()
  })
  it('shares state and one subscription across component lifetimes', async () => {
    const { api, use } = await updater()
    await api.checkForUpdate()
    expect(api.state.value).toBe('current')
    expect(api.checkedAt.value).toBeTypeOf('number')
    expect(use()).toBe(api)
    expect(use().state.value).toBe('current')
    expect(fixture.on).toHaveBeenCalledOnce()
  })
  it('prevents repeated checks and channel changes during a check', async () => {
    let resolve!: (value: typeof available) => void
    fixture.check.mockImplementation(() => new Promise(r => resolve = r))
    const { api } = await updater()
    const checking = api.checkForUpdate()
    await api.checkForUpdate()
    await api.setChannel('beta')
    expect(fixture.check).toHaveBeenCalledOnce()
    expect(fixture.setChannel).not.toHaveBeenCalled()
    resolve(available)
    await checking
    expect(api.state.value).toBe('available')
    expect(api.releaseDate.value).toBe(available.date)
  })
  it('keeps progress and installed state after another consumer joins', async () => {
    let resolve!: () => void
    fixture.check.mockResolvedValue(available)
    fixture.download.mockImplementation(() => new Promise<void>(r => resolve = r))
    const { api, use } = await updater()
    await api.checkForUpdate()
    const installing = api.downloadAndInstall()
    fixture.progress?.({ event: 'started', contentLength: 100 })
    fixture.progress?.({ event: 'progress', downloaded: 60, contentLength: 100 })
    expect(use().progressPercent.value).toBe(60)
    fixture.progress?.({ event: 'finished' })
    expect(use().state.value).toBe('installing')
    resolve()
    await installing
    await api.setChannel('beta')
    await api.checkForUpdate()
    expect(api.state.value).toBe('installed')
    expect(fixture.setChannel).not.toHaveBeenCalled()
    fixture.progress?.({ event: 'started', contentLength: 100 })
    expect(api.state.value).toBe('installed')
  })
  it('retries failed downloads and preserves the target release', async () => {
    fixture.check.mockResolvedValue(available)
    fixture.download.mockRejectedValueOnce(new Error('Network unavailable'))
    const { api } = await updater()
    await api.checkForUpdate()
    await api.downloadAndInstall()
    expect(api.state.value).toBe('error')
    expect(api.targetVersion.value).toBe('1.1.0')
    expect(api.errorMessage.value).toBe('Network unavailable')
    await api.downloadAndInstall()
    expect(api.state.value).toBe('installed')
    fixture.relaunch.mockRejectedValueOnce(new Error('Restart blocked'))
    await api.relaunch()
    expect(api.state.value).toBe('installed')
    expect(api.errorMessage.value).toBe('Restart blocked')
    await api.relaunch()
    expect(api.errorMessage.value).toBeNull()
  })
  it('retains a failed channel change and resets release information after success', async () => {
    fixture.check.mockResolvedValue(available)
    const { api } = await updater()
    await api.checkForUpdate()
    fixture.setChannel.mockRejectedValueOnce(new Error('Storage unavailable'))
    await expect(api.setChannel('beta')).rejects.toThrow('Storage unavailable')
    expect(api.channel.value).toBe('stable')
    expect(api.state.value).toBe('available')
    await api.setChannel('beta')
    expect(api.state.value).toBe('idle')
    expect(api.channel.value).toBe('beta')
    expect(api.targetVersion.value).toBeNull()
    expect(api.releaseDate.value).toBeNull()
  })
  it('shows a channel read failure and recovers through reload', async () => {
    fixture.getChannel.mockRejectedValueOnce(new Error('Channel unavailable'))
    const { api } = await updater()
    expect(api.channelReady.value).toBe(false)
    expect(api.channelError.value).toBe('Channel unavailable')
    await api.loadChannel()
    expect(api.channelReady.value).toBe(true)
    expect(api.channelError.value).toBeNull()
  })
  it('ignores an old initial channel read after a new selection', async () => {
    let resolve!: (value: { channel: string }) => void
    fixture.getChannel.mockImplementation(() => new Promise(r => resolve = r))
    const { api } = await updater()
    await api.setChannel('beta')
    resolve({ channel: 'stable' })
    await Promise.resolve()
    expect(api.channel.value).toBe('beta')
  })
})
