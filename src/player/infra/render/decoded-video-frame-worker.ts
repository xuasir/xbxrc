import type { DecodedFrameWorkerMessage, TrackProcessorConstructor } from './decoded-video-frame-reader'
import type { DecodedFrameWorkerRequest } from './decoded-video-frame-worker-source'
import type { FrameLinkScope, FramePortRequest } from './render-worker-protocol'
import { DecodedVideoFrameWorkerSource } from './decoded-video-frame-worker-source'
import { sameFrameScope } from './render-worker-protocol'

const worker = globalThis as unknown as {
  MediaStreamTrackProcessor?: TrackProcessorConstructor
  onmessage: (event: MessageEvent<DecodedFrameWorkerRequest>) => void
  postMessage: (message: DecodedFrameWorkerMessage & { scope?: FrameLinkScope }, transfer?: Transferable[]) => void
}
let port: MessagePort | undefined
let scope: FrameLinkScope | undefined
const source = new DecodedVideoFrameWorkerSource({
  MediaStreamTrackProcessor: worker.MediaStreamTrackProcessor,
  postMessage(message, transfer) {
    const envelope = scope ? { ...message, scope } : message
    if (port && (message.type === 'frame' || message.type === 'clock'))
      port.postMessage(envelope, transfer ?? [])
    else
      worker.postMessage(envelope, transfer)
    if (message.type === 'stopped') {
      port?.close()
      port = undefined
    }
  },
})
worker.onmessage = (event) => {
  const message = event.data
  if (message.type === 'start' && message.port && !port) {
    port = message.port
    scope = message.scope
    port.onmessage = (reply: MessageEvent<FramePortRequest>) => {
      if (sameFrameScope(reply.data.scope, scope))
        source.handle(reply.data)
    }
    port.onmessageerror = () => source.handle({ type: 'stop' })
    port.start()
  }
  if (!scope || sameFrameScope(message.scope, scope))
    source.handle(message)
}
