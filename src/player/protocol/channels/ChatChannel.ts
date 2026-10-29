import type { ChannelContext } from './BaseChannel'
import { BaseChannel } from './BaseChannel'

export interface ChatChannelDelegate {
  startMicCapture: () => Promise<void>
  stopMicCapture: () => void
}

export class ChatChannel extends BaseChannel {
  constructor(context: ChannelContext, private readonly delegate: ChatChannelDelegate) {
    super(context)
  }

  onMessage(): void {}

  startMic(): Promise<void> {
    return this.delegate.startMicCapture()
  }

  stopMic(): void {
    this.delegate.stopMicCapture()
  }
}
