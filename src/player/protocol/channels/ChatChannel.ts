import type { ChannelContext } from './BaseChannel'
import { BaseChannel } from './BaseChannel'

export interface ChatChannelDelegate {
  startMicCapture: () => Promise<void>
  stopMicCapture: () => void
}

export class ChatChannel extends BaseChannel {
  isCapturing = false
  isPaused = true

  constructor(context: ChannelContext, private readonly delegate: ChatChannelDelegate) {
    super(context)
  }

  onMessage(event: MessageEvent): void {
    try {
      JSON.parse(event.data)
    }
    catch {

    }
  }

  async startMic(): Promise<void> {
    await this.delegate.startMicCapture()
    this.isCapturing = true
    this.isPaused = false
  }

  stopMic(): void {
    this.delegate.stopMicCapture()
    this.isCapturing = false
    this.isPaused = true
  }
}
