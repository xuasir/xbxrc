import type { InputWorkerCommand, InputWorkerReply } from './input-worker-protocol'
import { InputWorkerRuntime } from './input-worker-runtime'

const worker = globalThis as unknown as {
  onmessage: (event: MessageEvent<InputWorkerCommand>) => void
  postMessage: (message: InputWorkerReply, transfer?: Transferable[]) => void
}
const runtime = new InputWorkerRuntime(worker)
worker.onmessage = event => runtime.handle(event.data)
