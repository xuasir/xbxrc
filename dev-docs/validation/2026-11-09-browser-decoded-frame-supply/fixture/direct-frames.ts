import { getVideoFrameSource } from '/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source'

// 仅用于验证标准解码帧接口；生产帧源和渲染器通过受控入口接收同一 VideoFrame。
export async function installDirectFrameProbe(video: HTMLVideoElement, renderer: any) {
  const source: any = getVideoFrameSource(video)
  const processor = renderer.processor ?? renderer.player
  const gl: WebGL2RenderingContext = processor.gl
  const originalNotify = source.notify.bind(source)
  const originalUpload = gl.texImage2D.bind(gl)
  let activeFrame: VideoFrame | undefined
  let frames = 0, errors = 0, stopped = false, requestId = 0
  let firstTimestamp: number | undefined, lastTimestamp: number | undefined
  let worker: Worker | undefined, reader: ReadableStreamDefaultReader<VideoFrame> | undefined
  const clone = (video.srcObject as MediaStream).getVideoTracks()[0].clone()
  const consume = (frame: VideoFrame) => {
    if (stopped) { frame.close(); return }
    activeFrame = frame
    frames++
    firstTimestamp ??= frame.timestamp
    lastTimestamp = frame.timestamp
    try { originalNotify(performance.now(), { videoGeneration: source.videoGeneration, localSequence: ++source.frameSequence, newFrame: true, trackingSource: 'decodedFrame', mediaTime: frame.timestamp / 1e6 }) }
    catch { errors++ }
    finally { activeFrame = undefined; frame.close() }
  }
  source.notify = (at: number, metadata: any) => originalNotify(at, { ...metadata, newFrame: false })
  ;(gl as any).texImage2D = (...args: any[]) => (originalUpload as any)(...args.map(value => value === video && activeFrame ? activeFrame : value))
  const Constructor = (window as any).MediaStreamTrackProcessor
  let mode: string
  if (typeof Constructor === 'function') {
    mode = 'main-thread'
    reader = new Constructor({ track: clone, maxBufferSize: 1 }).readable.getReader()
    void (async () => { while (!stopped) { const item = await reader!.read(); if (item.done) return; consume(item.value) } })().catch(() => errors++)
  }
  else {
    mode = 'worker'
    const script = new Blob([`
      let reader, track, stopped=false, acknowledge;
      onmessage=async ({data})=>{
        if(data.type==='ack'){acknowledge?.();return}
        if(data.type==='stop'){stopped=true;acknowledge?.();await reader?.cancel();track?.stop();return}
        if(data.type!=='start')return;
        try{
          track=data.track;
          reader=new MediaStreamTrackProcessor({track,maxBufferSize:1}).readable.getReader();
          postMessage({type:'ready'});
          while(!stopped){
            const {value:frame,done}=await reader.read();if(done)return;
            const acknowledged=new Promise(resolve=>acknowledge=resolve);
            postMessage({type:'frame',frame},[frame]);await acknowledged;
          }
        }catch(error){postMessage({type:'error',name:error.name,message:error.message})}
      }`], { type: 'text/javascript' })
    const url = URL.createObjectURL(script)
    worker = new Worker(url)
    URL.revokeObjectURL(url)
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('decoded processor initialization timeout')), 5000)
      worker!.onerror = () => { clearTimeout(timer); reject(new Error('decoded processor worker error')) }
      worker!.onmessage = ({ data }) => {
        if (data.type === 'ready') { clearTimeout(timer); resolve() }
        if (data.type === 'error') { clearTimeout(timer); reject(new Error(JSON.stringify(data))) }
        if (data.type === 'frame') { consume(data.frame); worker!.postMessage({ type: 'ack' }) }
      }
      worker!.postMessage({ type: 'start', track: clone }, [clone as unknown as Transferable])
    })
  }
  const keepAlive = () => source.request(() => { requestId = keepAlive() })
  requestId = keepAlive()
  return { snapshot: () => ({ mode, frames, errors, firstTimestamp, lastTimestamp }), close: async () => {
    stopped = true
    source.cancel(requestId)
    source.notify = originalNotify
    ;(gl as any).texImage2D = originalUpload
    if (worker) { worker.postMessage({ type: 'stop' }); await new Promise(resolve => setTimeout(resolve, 50)); worker.terminate() }
    await reader?.cancel()
    clone.stop()
  } }
}
