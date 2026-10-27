import { InputController } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamInputRouteController } from './stream-input-route-controller'

vi.mock('@shared/gamepad/runtime', () => ({ inputController: new InputController() }))

beforeEach(async () => {
  await streamInputRouteController.resetOnLeaveStream()
  inputController.setStreamReady(true)
})

describe('stream capture integration', () => {
  it('replaces overlay captures without exposing a stream frame between sheets', async () => {
    await streamInputRouteController.captureUiInput('menu')
    const owners: string[] = []
    const dispose = inputController.subscribe(s => owners.push(s.owner))
    await streamInputRouteController.replaceUiCapture('menu', 'sheet')
    expect(owners.every(owner => owner === 'ui')).toBe(true)
    await streamInputRouteController.releaseUiInput('sheet-close')
    expect(inputController.getSnapshot().owner).toBe('stream')
    dispose()
  })
  it('releases only the named capture and leaves other overlays active', async () => {
    await streamInputRouteController.captureUiInput('menu')
    await streamInputRouteController.captureUiInput('diagnostics')
    await streamInputRouteController.releaseUiInput('menu-close')
    expect(inputController.getSnapshot().owner).toBe('ui')
    await streamInputRouteController.releaseUiInput('diagnostics-close')
    expect(inputController.getSnapshot().owner).toBe('stream')
  })
  it('leaving a session releases stream captures and preserves an independent settings capture', async () => {
    await streamInputRouteController.captureUiInput('menu')
    inputController.setUiCapture('settings', true, true)
    await streamInputRouteController.resetOnLeaveStream()
    expect(inputController.getSnapshot()).toMatchObject({ streamReady: false, exclusiveCapture: true })
    inputController.setUiCapture('settings', false)
    expect(inputController.getSnapshot().uiCapturing).toBe(false)
  })
})
