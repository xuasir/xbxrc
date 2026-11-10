import type { DecodedFrameWorkerMessage, TrackProcessorConstructor } from './decoded-video-frame-reader'
import type { DecodedFrameWorkerRequest } from './decoded-video-frame-worker-source'
import { DecodedVideoFrameWorkerSource } from './decoded-video-frame-worker-source'

const worker = globalThis as unknown as {
  MediaStreamTrackProcessor?: TrackProcessorConstructor
  onmessage: (event: MessageEvent<DecodedFrameWorkerRequest>) => void
  postMessage: (message: DecodedFrameWorkerMessage, transfer?: Transferable[]) => void
}
const source = new DecodedVideoFrameWorkerSource(worker)
worker.onmessage = event => source.handle(event.data)
