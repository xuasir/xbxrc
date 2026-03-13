import type { ChannelContext } from './BaseChannel'
import { STREAM_CONTROL_PROFILE } from '../networkProfile'
import { BaseChannel } from './BaseChannel'

export interface ControlChannelDelegate {
  onClose: () => void
}

export class ControlChannel extends BaseChannel {
  private started = false
  private pendingStart = false

  constructor(context: ChannelContext, private readonly delegate: ControlChannelDelegate) {
    super(context)
  }

  onOpen(): void {
    console.info('[player][control] open')
    if (this.pendingStart) {
      this.pendingStart = false
      this.start()
    }
  }

  onMessage(event: MessageEvent): void {
    try {
      JSON.parse(event.data)
    }
    catch {

    }
  }

  onClose(): void {
    this.started = false
    this.pendingStart = false
    this.delegate.onClose()
  }

  sendGamepadAdded(gamepadIndex: number): void {
    this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: true }))
  }

  sendGamepadRemoved(gamepadIndex: number): void {
    this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: false }))
  }

  requestKeyframe(): void {
    this.send(JSON.stringify({ message: 'videoKeyframeRequested', ifrRequested: true }))
  }

  start(): void {
    if (this.started) {
      return
    }
    if (this.context.readyState() !== 'open') {
      console.info('[player][control] start deferred until open')
      this.pendingStart = true
      return
    }
    console.info('[player][control] start authorization flow')
    this.started = true
    this.send(JSON.stringify({
      message: 'authorizationRequest',
      accessKey: STREAM_CONTROL_PROFILE.accessKey,
    }))
    this.sendGamepadRemoved(0)
    // 浏览器 runtime 这里仅做一次性关键帧请求，用于缩短首帧空窗。
    // 周期性轮询在部分浏览器/远端组合上会持续触发 data channel send 错误。
    this.requestKeyframe()
    window.setTimeout(
      () => this.sendGamepadAdded(0),
      STREAM_CONTROL_PROFILE.gamepadAddedDelayMs,
    )
  }
}
