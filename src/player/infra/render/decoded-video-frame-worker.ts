import type { DecodedFrameWorkerMessage, TrackProcessorConstructor } from './decoded-video-frame-reader'

const worker = globalThis as unknown as {
  MediaStreamTrackProcessor?: TrackProcessorConstructor
  onmessage: (event: MessageEvent<{ type: 'start' | 'ack' | 'stop', track?: MediaStreamTrack }>) => void
  postMessage: (message: DecodedFrameWorkerMessage, transfer?: Transferable[]) => void
}
let reader: ReadableStreamDefaultReader<VideoFrame> | undefined
let track: MediaStreamTrack | undefined
let stopped = false
let acknowledge: (() => void) | undefined

async function stop(): Promise<void> {
  stopped = true
  acknowledge?.()
  track?.stop()
  try {
    await reader?.cancel()
  }
  catch {
    // 已结束的reader继续完成停止确认。
  }
  finally { worker.postMessage({ type: 'stopped' }) }
}

worker.onmessage = (event) => {
  const message = event.data
  if (message.type === 'ack') {
    acknowledge?.()
    acknowledge = undefined
  }
  else if (message.type === 'stop') {
    void stop()
  }
  else if (message.type === 'start' && !reader && !stopped) {
    track = message.track
    void (async () => {
      try {
        if (!worker.MediaStreamTrackProcessor || !track) {
          worker.postMessage({ type: 'failed', reason: 'unsupported' })
          return
        }
        reader = new worker.MediaStreamTrackProcessor({ track, maxBufferSize: 1 }).readable.getReader()
        worker.postMessage({ type: 'ready' })
        for (;;) {
          if (stopped)
            return
          const { value: frame, done } = await reader.read()
          if (done) {
            if (!stopped)
              worker.postMessage({ type: 'failed', reason: 'trackEnded' })
            return
          }
          if (stopped) {
            frame.close()
            return
          }
          // 最多一帧在途；主线程完成同步纹理导入后确认，再读取最新帧。
          const consumed = new Promise<void>(resolve => acknowledge = resolve)
          try {
            worker.postMessage({ type: 'frame', frame }, [frame])
          }
          catch (error) {
            frame.close()
            throw error
          }
          await consumed
        }
      }
      catch {
        if (!stopped)
          worker.postMessage({ type: 'failed', reason: 'readerFailed' })
      }
      finally {
        track?.stop()
        reader?.releaseLock()
      }
    })()
  }
}
