import type { ChannelContext } from './BaseChannel'
import { STREAM_CONTROL_PROFILE } from '../networkProfile'
import { BaseChannel } from './BaseChannel'

export interface ControlChannelDelegate {
  onClose: () => void
}

export class ControlChannel extends BaseChannel {
  private started = false
  private pendingStart = false
  private keyframeRequestTotal = 0
  private keyframeRequestSuccess = 0
  private lastError: string | undefined
  private sendFailBurst = 0

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
    this.lastError = 'channelClosed'
    this.delegate.onClose()
  }

  override onError(): void {
    this.lastError = 'channelError'
  }

  sendGamepadAdded(gamepadIndex: number): void {
    this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: true }))
  }

  sendGamepadRemoved(gamepadIndex: number): void {
    this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: false }))
  }

  requestKeyframe(): boolean {
    this.keyframeRequestTotal += 1
    const sent = this.send(JSON.stringify({ message: 'videoKeyframeRequested', ifrRequested: true }))
    if (sent) {
      this.keyframeRequestSuccess += 1
      this.sendFailBurst = 0
      this.lastError = undefined
    }
    else {
      this.sendFailBurst += 1
      this.lastError = `sendFailed:${this.context.readyState()}`
    }
    return sent
  }

  getHealthSnapshot(): {
    state: RTCDataChannelState
    lastError?: string
    keyframeRequestTotal: number
    keyframeRequestSuccess: number
    keyframeRequestSuccessRate?: number
    sendFailBurst: number
    bufferedAmount: number
  } {
    const state = this.context.readyState()
    const keyframeRequestSuccessRate = this.keyframeRequestTotal > 0
      ? this.keyframeRequestSuccess / this.keyframeRequestTotal
      : undefined
    return {
      state,
      lastError: this.lastError,
      keyframeRequestTotal: this.keyframeRequestTotal,
      keyframeRequestSuccess: this.keyframeRequestSuccess,
      keyframeRequestSuccessRate,
      sendFailBurst: this.sendFailBurst,
      bufferedAmount: this.context.bufferedAmount(),
    }
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
