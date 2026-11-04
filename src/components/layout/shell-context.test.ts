import { describe, expect, it } from 'vitest'
import { createShellContext } from './shell-context'

describe('shell hint ownership', () => {
  it('keeps the new page hints when a cached previous page deactivates', () => {
    const shell = createShellContext()
    const home = Symbol('home')
    const cloud = Symbol('cloud')
    shell.setPage(home, { status: '主机在线' })
    shell.setPage(cloud, { status: '120 款游戏', regionLabel: '切换区域' })
    shell.setPage(home)
    expect(shell.page.value).toEqual({ status: '120 款游戏', regionLabel: '切换区域' })
    shell.setPage(cloud)
    expect(shell.page.value).toEqual({})
  })

  it('keeps confirmation hints above parent updates and restores the parent after close', () => {
    const shell = createShellContext()
    const account = Symbol('account')
    const confirm = Symbol('confirm')
    shell.setOverlay(account, { title: '账户' })
    shell.setOverlay(confirm, { title: '退出账户' })
    shell.setOverlay(account, { title: '账户', busy: true })
    expect(shell.overlay.value?.title).toBe('退出账户')
    shell.setOverlay(confirm)
    expect(shell.overlay.value).toEqual({ title: '账户', busy: true })
    shell.setOverlay(account)
    expect(shell.overlay.value).toBeUndefined()
  })
})
