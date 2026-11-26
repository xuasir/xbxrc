import type { RenderWorkerCommand, RenderWorkerReply } from './render-worker-protocol'
import { RenderWorkerRuntime } from './render-worker-runtime'

const worker = globalThis as unknown as {
  onmessage: (event: MessageEvent<RenderWorkerCommand>) => void
  postMessage: (message: RenderWorkerReply, transfer?: Transferable[]) => void
}
const runtime = new RenderWorkerRuntime(worker)
worker.onmessage = event => runtime.handle(event.data)
