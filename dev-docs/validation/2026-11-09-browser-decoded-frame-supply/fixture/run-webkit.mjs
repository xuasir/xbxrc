import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import path from 'node:path'
import http from 'node:http'
const root = '/tmp/xbxrc-2026-11-09-frame-cadence-probe'
const external = process.argv.includes('--external')
const canvasSource = process.argv.includes('--canvas-source')
const sourceJitter = process.argv.includes('--source-jitter')
const drawable = process.argv.includes('--drawable-identity')
const direct = process.argv.includes('--direct')
const option = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length+1)
const tag = option('--tag')
const modes = option('--modes')
const markerStride = option('--marker-stride')
const chromeReceiver = process.argv.includes('--chrome-receiver')
const oldRuntime = process.argv.includes('--old-runtime')
const outputPixels = process.argv.includes('--output-pixels')
const duration = option('--duration-ms')
if (tag && !/^[a-z0-9-]+$/.test(tag)) throw new Error('Invalid output tag')
const names = external ? ['index.html', 'bundle.js', 'decoded-video-frame-worker.ts', 'sender.html', 'sender.js', 'source.mp4'] : ['index.html', 'bundle.js', 'decoded-video-frame-worker.ts']
const assets = new Map(await Promise.all(names.map(async name => [name, await fs.readFile(name === 'source.mp4' ? '/tmp/xbxrc-sr-render-probe/source.mp4' : path.join(root, name === 'decoded-video-frame-worker.ts' ? 'decoded-video-frame-worker.js' : name))])))
let cdp
let nextPeer = 0
let chrome
let socket
let receiverSocket
let receiverChrome
let debuggingPort
const pending = new Map()
const bodyOf = request => new Promise((resolve, reject) => {
  let body = ''
  request.on('data', chunk => { body = (body + chunk.toString()).slice(0, 32000) })
  request.on('end', () => resolve(body))
  request.on('error', reject)
})
const senderCall = async expression => {
  const result = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, timeout: 20000 })
  if (result.exceptionDetails) throw new Error(`Local source operation: ${result.exceptionDetails.text}`)
  return result.result.value
}
const server = http.createServer(async (request, response) => {
  try {
  if (external && request.url.startsWith('/rtc/')) {
    let value
    if (request.url === '/rtc/offer') {
      const id = ++nextPeer
      value = { id, ...await senderCall(`XBXSender.offer(${id})`) }
    }
    else {
      const body = JSON.parse(await bodyOf(request))
      if (request.url === '/rtc/answer') value = await senderCall(`XBXSender.answer(${JSON.stringify(body.id)},${JSON.stringify(body.description)})`)
      else if (request.url === '/rtc/debug') value = await senderCall(`XBXSender.debug(${JSON.stringify(body.id)})`)
      else value = await senderCall(`XBXSender.close(${JSON.stringify(body.id)})`)
    }
    response.writeHead(200, { 'Content-Type': 'application/json' }); response.end(JSON.stringify(value)); return
  }
  if (request.url === '/progress' && request.method === 'POST') {
    console.log(await bodyOf(request)); response.writeHead(200); response.end('ok')
    return
  }
  const name = (request.url ?? '').split('?')[0].slice(1)
  const bytes = assets.get(name)
  if (!bytes) { response.writeHead(404); response.end(); return }
  const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range ?? '')
  if (range) {
    const start = Number(range[1]), end = range[2] ? Math.min(Number(range[2]), bytes.length - 1) : bytes.length - 1
    response.writeHead(206, { 'Content-Type': 'video/mp4', 'Accept-Ranges': 'bytes', 'Content-Range': `bytes ${start}-${end}/${bytes.length}`, 'Content-Length': end-start+1 })
    response.end(bytes.subarray(start,end+1)); return
  }
  response.writeHead(200, { 'Content-Type': name.endsWith('.js') || name.endsWith('.ts') ? 'text/javascript' : name.endsWith('.mp4') ? 'video/mp4' : 'text/html' })
  response.end(bytes)
  }
  catch (error) { response.writeHead(500, { 'Content-Type': 'application/json' }); response.end(JSON.stringify({ error: String(error) })) }
})
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const output = path.join(root, tag ? `result-${tag}.json` : drawable ? 'result-drawable-identity.json' : external ? canvasSource ? sourceJitter ? 'result-external-jitter.json' : 'result-external-canvas.json' : 'result-external.json' : 'result.json')
const url = `http://127.0.0.1:${server.address().port}/index.html?source=webrtc${external ? '&external=1' : ''}${drawable ? '&drawable=1' : ''}${direct ? '&direct=1' : ''}${modes ? `&modes=${encodeURIComponent(modes)}` : ''}${markerStride ? `&markerStride=${encodeURIComponent(markerStride)}` : ''}${outputPixels ? '&outputPixels=1' : ''}${duration ? `&durationMs=${encodeURIComponent(duration)}` : ''}`
let child
let stderr = ''
try {
  if (external) {
    const senderUrl = `http://127.0.0.1:${server.address().port}/sender.html${canvasSource ? `?producer=canvas${sourceJitter ? '&jitter=1' : ''}` : ''}`
    const profile = await fs.mkdtemp(path.join(root, 'chrome-profile-'))
    chrome = spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-renderer-backgrounding', '--disable-background-timer-throttling', '--disable-background-media-suspend', '--autoplay-policy=no-user-gesture-required', '--mute-audio', '--use-gl=angle', '--use-angle=metal', senderUrl], { stdio: ['ignore', 'ignore', 'pipe'] })
    chrome.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-1500) })
    let port
    const deadline = Date.now()+20000
    while (!port && Date.now()<deadline) {
      try { port = Number((await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]) } catch {}
      if (!port) await new Promise(resolve=>setTimeout(resolve,100))
    }
    if (!port) throw new Error(`Local Chrome start timeout: ${stderr}`)
    debuggingPort = port
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()
    const target = targets.find(t=>t.type==='page' && t.url===senderUrl)
    if (!target) throw new Error('Local source page unavailable')
    socket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})})
    let id = 0
    socket.addEventListener('message', event=>{
      const value=JSON.parse(event.data);const waiter=pending.get(value.id)
      if (waiter) { pending.delete(value.id); value.error ? waiter.reject(value.error) : waiter.resolve(value.result) }
    })
    socket.addEventListener('close',()=>{for(const waiter of pending.values())waiter.reject(new Error('Local source closed'));pending.clear()})
    cdp = (method, params)=>new Promise((resolve,reject)=>{const key=++id;pending.set(key,{resolve,reject});socket.send(JSON.stringify({id:key,method,params}))})
    await cdp('Runtime.enable',{})
    let ready=false
    for(let i=0;i<50;i++) {const r=await cdp('Runtime.evaluate',{expression:'typeof XBXSender',returnByValue:true});if(r.result.value==='object'){ready=true;break}await new Promise(resolve=>setTimeout(resolve,100))}
    if(!ready)throw new Error('Local source script unavailable')
  }
  if (chromeReceiver) {
    const receiverProfile=await fs.mkdtemp(path.join(root,'chrome-receiver-profile-'))
    receiverChrome=spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--remote-debugging-port=0',`--user-data-dir=${receiverProfile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-renderer-backgrounding','--disable-background-timer-throttling','--disable-backgrounding-occluded-windows','--autoplay-policy=no-user-gesture-required','--mute-audio','--use-gl=angle','--use-angle=metal',url],{stdio:['ignore','ignore','pipe']})
    receiverChrome.stderr.on('data',chunk=>{stderr=(stderr+chunk.toString()).slice(-1500)})
    let receiverPort
    const deadline=Date.now()+20000
    while(!receiverPort&&Date.now()<deadline){try{receiverPort=Number((await fs.readFile(path.join(receiverProfile,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{}if(!receiverPort)await new Promise(resolve=>setTimeout(resolve,100))}
    if(!receiverPort)throw new Error('Chromium receiver start timeout')
    const receiverTargets=await(await fetch(`http://127.0.0.1:${receiverPort}/json/list`)).json()
    const target=receiverTargets.find(value=>value.type==='page'&&value.url===url)
    receiverSocket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve,reject)=>{receiverSocket.addEventListener('open',resolve,{once:true});receiverSocket.addEventListener('error',reject,{once:true})})
    const requests = new Map()
    let receiverId=0
    receiverSocket.addEventListener('message',event=>{const message=JSON.parse(event.data);const item=requests.get(message.id);if(item){requests.delete(message.id);message.error?item.reject(message.error):item.resolve(message.result)}})
    const evaluate=expression=>new Promise((resolve,reject)=>{const id=++receiverId;requests.set(id,{resolve,reject});receiverSocket.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true,timeout:120000}}))})
    for(let count=0;count<50;count++){const ready=await evaluate('typeof XBXProbe');if(ready.result?.value==='object')break;await new Promise(resolve=>setTimeout(resolve,100))}
    if(oldRuntime)await evaluate('window.MediaStreamTrackProcessor=undefined;window.Worker=undefined')
    const result=await evaluate('XBXProbe.run()')
    if(result.exceptionDetails)throw new Error(JSON.stringify(result.exceptionDetails))
    const report=result.result.value
    if(report.error)throw new Error(JSON.stringify(report))
    await fs.writeFile(output,JSON.stringify(report,null,2))
    console.log(JSON.stringify({completed:report.runs.length,engine:'Chromium receiver',capabilities:report.environment.capabilities,output}))
  }
  else {
  child = spawn(path.join(root, 'probe-webkit'), [url, output], { stdio: ['ignore', 'pipe', 'pipe'] })
  child.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-1500) })
  console.log('Started local 1440p WKWebView frame-supply ablation')
  const code = await new Promise((resolve, reject) => { child.on('exit', resolve); child.on('error', reject) })
  if (code !== 0) throw new Error(`WKWebView probe failed: ${code}; ${stderr}`)
  const report = JSON.parse(await fs.readFile(output, 'utf8'))
  if (report.error) throw new Error(report.error)
  console.log(JSON.stringify({ completed: report.runs.length, foregroundValid: report.runs.every(run => run.foregroundValid), nativeWindowSamples: report.nativeWindowStates.length, nativeWindowVisible: report.nativeWindowStates.every(state => state.visible && state.occlusionVisible && !state.miniaturized), output }))
  }
}
finally { child?.kill('SIGTERM'); receiverSocket?.close(); socket?.close(); receiverChrome?.kill('SIGTERM'); chrome?.kill('SIGTERM'); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)) }
