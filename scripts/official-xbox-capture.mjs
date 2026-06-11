#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import { mkdir, writeFile } from 'node:fs/promises'
import process from 'node:process'
import { createInterface } from 'node:readline/promises'

const DEFAULT_URL = 'https://www.xbox.com/play'
const DEFAULT_PORT = 9222
const DEFAULT_SAMPLE_INTERVAL_MS = 2000

const networkRequests = new Map()
let args
let startedAt
let outDir
let port
let sampleIntervalMs
let profileDir
let targetUrl
let eventStream
let statsStream
let lastCandidates = []
let cdp
let chrome
let sampleTimer

function parseArgs(values) {
  const parsed = {}
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index]
    if (!value.startsWith('--')) {
      continue
    }
    const key = value.slice(2)
    if (key === 'no-launch') {
      parsed.noLaunch = true
      continue
    }
    const next = values[index + 1]
    index += 1
    if (key === 'out')
      parsed.out = next
    else if (key === 'port')
      parsed.port = next
    else if (key === 'profile')
      parsed.profile = next
    else if (key === 'url')
      parsed.url = next
    else if (key === 'sample-interval-ms')
      parsed.sampleIntervalMs = next
  }
  return parsed
}

function launchChrome({ port, profileDir }) {
  const chromePath = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  return spawn(chromePath, [
    `--remote-debugging-port=${port}`,
    '--remote-allow-origins=*',
    `--user-data-dir=${profileDir}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-background-networking',
    '--window-size=1440,1000',
    'about:blank',
  ], {
    detached: false,
    stdio: 'ignore',
  })
}

async function waitForChrome(port) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`)
      if (response.ok) {
        return
      }
    }
    catch {}
    await delay(250)
  }
  throw new Error(`Chrome DevTools endpoint did not become ready on port ${port}`)
}

async function createBlankTarget(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })
  if (!response.ok) {
    throw new Error(`failed to create target: HTTP ${response.status}`)
  }
  return await response.json()
}

async function setupPage(client) {
  await client.send('Page.enable')
  await client.send('Runtime.enable')
  await client.send('Log.enable')
  await client.send('Network.enable', {
    maxPostDataSize: 0,
  })

  await client.send('Page.addScriptToEvaluateOnNewDocument', {
    source: WEBRTC_HOOK_SOURCE, // eslint-disable-line no-use-before-define
  })

  client.on('Network.requestWillBeSent', (params) => {
    networkRequests.set(params.requestId, {
      requestId: params.requestId,
      startedAt: params.wallTime,
      type: params.type,
      method: params.request?.method,
      url: sanitizeUrl(params.request?.url),
      initiator: summarizeInitiator(params.initiator),
    })
    writeEvent('network-request', networkRequests.get(params.requestId))
  })
  client.on('Network.responseReceived', (params) => {
    const entry = networkRequests.get(params.requestId) ?? { requestId: params.requestId }
    entry.response = sanitizeResponse(params.response)
    entry.type = params.type ?? entry.type
    writeEvent('network-response', entry)
  })
  client.on('Network.loadingFinished', (params) => {
    const entry = networkRequests.get(params.requestId) ?? { requestId: params.requestId }
    entry.finished = {
      encodedDataLength: params.encodedDataLength,
      timestamp: params.timestamp,
    }
    writeEvent('network-finished', entry)
    void captureOfficialControlBody(client, params.requestId, entry)
  })
  client.on('Network.loadingFailed', (params) => {
    writeEvent('network-failed', {
      requestId: params.requestId,
      type: params.type,
      errorText: params.errorText,
      canceled: params.canceled,
      blockedReason: params.blockedReason,
    })
  })
  client.on('Network.webSocketCreated', (params) => {
    writeEvent('websocket-created', {
      requestId: params.requestId,
      url: sanitizeUrl(params.url),
      initiator: summarizeInitiator(params.initiator),
    })
  })
  client.on('Network.webSocketFrameSent', (params) => {
    writeEvent('websocket-frame-sent', {
      requestId: params.requestId,
      frame: sanitizeFrame(params.response),
    })
  })
  client.on('Network.webSocketFrameReceived', (params) => {
    writeEvent('websocket-frame-received', {
      requestId: params.requestId,
      frame: sanitizeFrame(params.response),
    })
  })
  client.on('Runtime.consoleAPICalled', (params) => {
    writeEvent('console', {
      type: params.type,
      timestamp: params.timestamp,
      args: params.args?.map(sanitizeRemoteObject).slice(0, 8),
    })
  })
  client.on('Runtime.exceptionThrown', (params) => {
    writeEvent('exception', {
      timestamp: params.timestamp,
      text: params.exceptionDetails?.text,
      url: sanitizeUrl(params.exceptionDetails?.url),
      lineNumber: params.exceptionDetails?.lineNumber,
      columnNumber: params.exceptionDetails?.columnNumber,
    })
  })
  client.on('Log.entryAdded', (params) => {
    writeEvent('browser-log', {
      level: params.entry?.level,
      source: params.entry?.source,
      text: sanitizeText(params.entry?.text),
      url: sanitizeUrl(params.entry?.url),
    })
  })
}

async function navigate(client, url) {
  await client.send('Page.navigate', { url })
}

async function printStatus() {
  const value = await evaluate(`({
    url: location.href,
    title: document.title,
    readyState: document.readyState,
    installed: Boolean(window.__xbxOfficialCapture),
    peerConnectionCount: window.__xbxOfficialCapture?.pcs?.length ?? 0
  })`)
  console.log(`[capture] status ${JSON.stringify(value, null, 2)}`)
}

async function printCandidates() {
  // eslint-disable-next-line no-use-before-define
  const candidates = await evaluate(CANDIDATES_EXPRESSION)
  lastCandidates = candidates
  await writeFile(`${outDir}/dom-candidates.json`, `${JSON.stringify(candidates, null, 2)}\n`)
  console.log('[capture] candidates:')
  for (const candidate of candidates.slice(0, 30)) {
    console.log(`${candidate.index}\t${Math.round(candidate.rect.x)},${Math.round(candidate.rect.y)} ${Math.round(candidate.rect.width)}x${Math.round(candidate.rect.height)}\t${candidate.label}`)
  }
}

async function clickCandidate(index) {
  const candidate = lastCandidates.find(item => item.index === index)
  if (!candidate) {
    throw new Error(`candidate ${index} not found; run candidates first`)
  }
  const x = candidate.rect.x + candidate.rect.width / 2
  const y = candidate.rect.y + candidate.rect.height / 2
  await clickXY(x, y)
}

async function clickXY(x, y) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new TypeError('click coordinates must be finite numbers')
  }
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none' })
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 })
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 })
  writeEvent('agent-click', { x, y })
  console.log(`[capture] clicked ${Math.round(x)},${Math.round(y)}`)
}

async function drainWebrtcEvents() {
  const events = await evaluate(`window.__xbxOfficialCapture?.drainEvents?.() ?? []`)
  for (const event of events) {
    writeEvent('webrtc-hook', event)
  }
}

async function collectStats() {
  const sample = await evaluate(`window.__xbxOfficialCapture?.collectStats?.() ?? null`, true)
  if (!sample) {
    return
  }
  statsStream.write(`${JSON.stringify(sample)}\n`)
}

async function evaluate(expression, awaitPromise = false) {
  const result = await cdp.send('Runtime.evaluate', {
    expression,
    awaitPromise,
    returnByValue: true,
    timeout: 10_000,
  })
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text ?? 'Runtime.evaluate failed')
  }
  return result.result?.value
}

async function finish(reason) {
  clearInterval(sampleTimer)
  await drainWebrtcEvents().catch(() => {})
  await collectStats().catch(() => {})
  const summary = buildNetworkSummary()
  await writeFile(`${outDir}/network-summary.json`, `${JSON.stringify(summary, null, 2)}\n`)
  await writeFile(`${outDir}/README.md`, [
    '# Official Xbox Capture',
    '',
    `Started: ${startedAt.toISOString()}`,
    `Finished: ${new Date().toISOString()}`,
    `Reason: ${reason}`,
    '',
    '- `events.jsonl`: sanitized Network / WebSocket / console / WebRTC hook events',
    '- `webrtc-stats.jsonl`: periodic RTCPeerConnection stats snapshots',
    '- `network-summary.json`: host and path level request summary',
    '- `dom-candidates.json`: latest visible clickable candidates',
    '',
  ].join('\n'))
  eventStream.end()
  statsStream.end()
  await cdp?.close()
  console.log(`[capture] finished reason=${reason} output=${outDir}`)
}

function buildNetworkSummary() {
  const byHost = new Map()
  const byPath = new Map()
  for (const request of networkRequests.values()) {
    const host = request.url?.host ?? 'unknown'
    const pathKey = `${host}${request.url?.path ?? ''}`
    const hostEntry = byHost.get(host) ?? { host, count: 0, statuses: {} }
    hostEntry.count += 1
    const status = request.response?.status ?? 'pending'
    hostEntry.statuses[status] = (hostEntry.statuses[status] ?? 0) + 1
    byHost.set(host, hostEntry)

    const pathEntry = byPath.get(pathKey) ?? { host, path: request.url?.path ?? '', count: 0, methods: {}, statuses: {} }
    pathEntry.count += 1
    pathEntry.methods[request.method ?? 'unknown'] = (pathEntry.methods[request.method ?? 'unknown'] ?? 0) + 1
    pathEntry.statuses[status] = (pathEntry.statuses[status] ?? 0) + 1
    byPath.set(pathKey, pathEntry)
  }
  return {
    generatedAt: new Date().toISOString(),
    requestCount: networkRequests.size,
    byHost: [...byHost.values()].sort((a, b) => b.count - a.count),
    byPath: [...byPath.values()].sort((a, b) => b.count - a.count),
  }
}

function writeEvent(kind, payload) {
  eventStream.write(`${JSON.stringify({
    at: new Date().toISOString(),
    kind,
    payload,
  })}\n`)
}

function sanitizeUrl(raw) {
  if (typeof raw !== 'string' || raw.length === 0) {
    return null
  }
  try {
    const url = new URL(raw)
    const queryKeys = [...new Set([...url.searchParams.keys()])].sort()
    return {
      host: url.host,
      path: url.pathname,
      queryKeys,
      hashPresent: url.hash.length > 0,
      display: `${url.origin}${url.pathname}${queryKeys.length ? `?${queryKeys.map(key => `${key}=<redacted>`).join('&')}` : ''}`,
    }
  }
  catch {
    return { display: sanitizeText(raw) }
  }
}

function sanitizeResponse(response) {
  if (!response) {
    return null
  }
  return {
    url: sanitizeUrl(response.url),
    status: response.status,
    statusText: response.statusText,
    mimeType: response.mimeType,
    protocol: response.protocol,
    fromDiskCache: response.fromDiskCache,
    fromServiceWorker: response.fromServiceWorker,
    connectionReused: response.connectionReused,
    connectionId: response.connectionId,
    remoteAddressFamily: addressFamily(response.remoteIPAddress),
    remotePortPresent: Number.isFinite(response.remotePort),
    headers: sanitizeHeaders(response.headers),
    timing: response.timing
      ? {
          proxyStart: response.timing.proxyStart,
          proxyEnd: response.timing.proxyEnd,
          dnsStart: response.timing.dnsStart,
          dnsEnd: response.timing.dnsEnd,
          connectStart: response.timing.connectStart,
          connectEnd: response.timing.connectEnd,
          sslStart: response.timing.sslStart,
          sslEnd: response.timing.sslEnd,
          sendStart: response.timing.sendStart,
          sendEnd: response.timing.sendEnd,
          receiveHeadersEnd: response.timing.receiveHeadersEnd,
        }
      : null,
  }
}

async function captureOfficialControlBody(client, requestId, entry) {
  if (!shouldCaptureOfficialControlBody(entry)) {
    return
  }
  try {
    const body = await client.send('Network.getResponseBody', { requestId })
    const raw = body?.body ?? ''
    writeEvent('network-response-body', {
      requestId,
      method: entry.method ?? null,
      status: entry.response?.status ?? null,
      url: entry.response?.url ?? entry.url ?? null,
      body: body?.base64Encoded
        ? {
            base64Encoded: true,
            length: raw.length,
            sha256: sha256(raw),
          }
        : {
            length: raw.length,
            sha256: sha256(raw),
            parsed: sanitizePayload(raw),
          },
    })
  }
  catch (error) {
    writeEvent('collector-error', {
      source: 'network-response-body',
      requestId,
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

function shouldCaptureOfficialControlBody(entry) {
  const url = entry.response?.url ?? entry.url
  const host = url?.host ?? ''
  const path = url?.path ?? ''
  if (!host.endsWith('gssv-play-prodxhome.xboxlive.com')) {
    return false
  }
  return path === '/v6/servers/home'
    || path === '/v5/sessions/home/play'
    || /^\/v5\/sessions\/home\/[^/]+\/(?:state|configuration|sdp|ice)$/.test(path)
}

function sanitizeHeaders(headers = {}) {
  const output = {}
  for (const [key, value] of Object.entries(headers)) {
    const lower = key.toLowerCase()
    if (/auth|cookie|token|secret|signature|sig|key|credential|xbl|xsts|msa|jwt/.test(lower)) {
      output[key] = '<redacted>'
    }
    else if (['content-type', 'content-length', 'cache-control', 'server', 'date', 'expires', 'pragma', 'access-control-allow-origin'].includes(lower)) {
      output[key] = String(value).slice(0, 160)
    }
    else {
      output[key] = '<present>'
    }
  }
  return output
}

function sanitizeFrame(frame) {
  const payload = frame?.payloadData ?? ''
  return {
    opcode: frame?.opcode,
    mask: frame?.mask,
    length: payload.length,
    sha256: sha256(payload),
    parsed: sanitizePayload(payload),
  }
}

function sanitizePayload(value) {
  if (typeof value !== 'string' || value.length === 0) {
    return null
  }
  if (value.includes('candidate:')) {
    return { candidate: parseCandidate(value) }
  }
  if (value.includes('v=0') && value.includes('m=')) {
    return { sdp: summarizeSdp(value) }
  }
  try {
    return sanitizeJson(JSON.parse(value))
  }
  catch {
    return { preview: sanitizeText(value.slice(0, 240)), truncated: value.length > 240 }
  }
}

function sanitizeJson(value) {
  if (Array.isArray(value)) {
    return value.slice(0, 40).map(sanitizeJson)
  }
  if (value && typeof value === 'object') {
    const output = {}
    for (const [key, child] of Object.entries(value)) {
      const lower = key.toLowerCase()
      if (/auth|cookie|token|secret|signature|sig|key|credential|xbl|xsts|msa|jwt|sessionid|userid|gamertag|email|xuid|deviceid|consoleid|serverid|serial|hostname|ipaddress|address|displayname|name/.test(lower)) {
        output[key] = '<redacted>'
      }
      else if (lower.includes('sdp') && typeof child === 'string') {
        output[key] = summarizeSdp(child)
      }
      else if (lower.includes('candidate') && typeof child === 'string') {
        output[key] = parseCandidate(child)
      }
      else {
        output[key] = sanitizeJson(child)
      }
    }
    return output
  }
  if (typeof value === 'string') {
    if (value.includes('candidate:'))
      return parseCandidate(value)
    if (value.includes('v=0') && value.includes('m='))
      return summarizeSdp(value)
    return sanitizeText(value)
  }
  return value
}

function summarizeSdp(raw) {
  const lines = String(raw).split(/\r?\n/).map(line => line.trim()).filter(Boolean)
  return {
    media: lines.filter(line => line.startsWith('m=')),
    groups: lines.filter(line => line.startsWith('a=group:')),
    mids: lines.filter(line => line.startsWith('a=mid:')),
    directions: lines.filter(line => /^a=(?:sendrecv|sendonly|recvonly|inactive)$/.test(line)),
    codecs: lines.filter(line => line.startsWith('a=rtpmap:')),
    fmtp: lines.filter(line => line.startsWith('a=fmtp:')),
    rtcpFb: lines.filter(line => line.startsWith('a=rtcp-fb:')),
    extmaps: lines.filter(line => line.startsWith('a=extmap:')),
    setup: lines.filter(line => line.startsWith('a=setup:')),
    iceOptions: lines.filter(line => line.startsWith('a=ice-options:')),
    candidates: lines.filter(line => line.startsWith('a=candidate:')).map(parseCandidate),
    fingerprintPresent: lines.some(line => line.startsWith('a=fingerprint:')),
    iceCredentialsPresent: lines.some(line => line.startsWith('a=ice-ufrag:') || line.startsWith('a=ice-pwd:')),
  }
}

function parseCandidate(raw) {
  const text = String(raw).replace(/^a=/, '')
  const parts = text.split(/\s+/)
  const typIndex = parts.indexOf('typ')
  const protocol = parts[2]?.toLowerCase()
  const address = parts[4]
  return {
    protocol,
    component: parts[1],
    type: typIndex >= 0 ? parts[typIndex + 1] : undefined,
    addressFamily: addressFamily(address),
    tcpType: parts.includes('tcptype') ? parts[parts.indexOf('tcptype') + 1] : undefined,
    relatedAddressFamily: parts.includes('raddr') ? addressFamily(parts[parts.indexOf('raddr') + 1]) : undefined,
    portPresent: Number.isFinite(Number(parts[5])),
  }
}

function addressFamily(address) {
  if (typeof address !== 'string' || address.length === 0)
    return undefined
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(address))
    return 'ipv4'
  if (address.includes(':'))
    return 'ipv6'
  return 'fqdn-or-mdns'
}

function summarizeInitiator(initiator) {
  if (!initiator) {
    return null
  }
  return {
    type: initiator.type,
    url: sanitizeUrl(initiator.url),
    lineNumber: initiator.lineNumber,
  }
}

function sanitizeRemoteObject(value) {
  if (Object.hasOwn(value, 'value')) {
    return sanitizeJson(value.value)
  }
  return {
    type: value.type,
    subtype: value.subtype,
    description: sanitizeText(value.description),
  }
}

function sanitizeText(value) {
  if (typeof value !== 'string') {
    return value
  }
  return value
    .replaceAll(/Bearer\s+[\w.~+/=-]+/gi, 'Bearer <redacted>')
    .replaceAll(
      /(?:token|sig|signature|authorization|xbl|xsts|jwt)=[^&\s]+/gi,
      match => `${match.slice(0, match.indexOf('='))}=<redacted>`,
    )
    .slice(0, 1000)
}

function sha256(value) {
  return createHash('sha256').update(String(value)).digest('hex')
}

function delay(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

class CdpClient {
  constructor(url) {
    this.url = url
    this.nextId = 1
    this.pending = new Map()
    this.handlers = new Map()
  }

  async open() {
    this.ws = new WebSocket(this.url)
    this.ws.addEventListener('message', event => this.handleMessage(event))
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true })
      this.ws.addEventListener('error', reject, { once: true })
    })
  }

  send(method, params = {}) {
    const id = this.nextId++
    this.ws.send(JSON.stringify({ id, method, params }))
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method })
      setTimeout(() => {
        if (!this.pending.has(id)) {
          return
        }
        this.pending.delete(id)
        reject(new Error(`CDP timeout: ${method}`))
      }, 15_000)
    })
  }

  on(method, handler) {
    const handlers = this.handlers.get(method) ?? []
    handlers.push(handler)
    this.handlers.set(method, handlers)
  }

  handleMessage(event) {
    const message = JSON.parse(event.data)
    if (message.id) {
      const pending = this.pending.get(message.id)
      if (!pending)
        return
      this.pending.delete(message.id)
      if (message.error) {
        pending.reject(new Error(`${pending.method}: ${message.error.message}`))
      }
      else {
        pending.resolve(message.result ?? {})
      }
      return
    }
    for (const handler of this.handlers.get(message.method) ?? []) {
      handler(message.params ?? {})
    }
  }

  async close() {
    this.ws?.close()
  }
}

const CANDIDATES_EXPRESSION = `(() => {
  const elements = [...document.querySelectorAll('button,a,[role="button"],[tabindex],div')]
  const visible = []
  let index = 0
  for (const el of elements) {
    const rect = el.getBoundingClientRect()
    const style = getComputedStyle(el)
    if (rect.width < 40 || rect.height < 24) continue
    if (rect.bottom < 0 || rect.right < 0 || rect.top > innerHeight || rect.left > innerWidth) continue
    if (style.visibility === 'hidden' || style.display === 'none' || Number(style.opacity) === 0) continue
    const label = [
      el.getAttribute('aria-label'),
      el.getAttribute('title'),
      el.innerText,
    ].filter(Boolean).join(' ').replace(/\\s+/g, ' ').trim()
    if (!label) continue
    const clickable = el.tagName === 'BUTTON' || el.tagName === 'A' || el.getAttribute('role') === 'button' || el.tabIndex >= 0 || getComputedStyle(el).cursor === 'pointer'
    if (!clickable && rect.width < 180 && rect.height < 100) continue
    visible.push({
      index: index++,
      tag: el.tagName.toLowerCase(),
      role: el.getAttribute('role'),
      label: label.slice(0, 180),
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    })
  }
  return visible
    .sort((a, b) => (a.rect.y - b.rect.y) || (a.rect.x - b.rect.x))
    .slice(0, 80)
})()`

const WEBRTC_HOOK_SOURCE = `(() => {
  if (window.__xbxOfficialCaptureInstalled) return
  window.__xbxOfficialCaptureInstalled = true
  const Native = window.RTCPeerConnection || window.webkitRTCPeerConnection
  const capture = {
    installedAt: Date.now(),
    events: [],
    pcs: [],
    nextId: 1,
    push(event) {
      this.events.push({ t: Date.now(), ...event })
      if (this.events.length > 4000) this.events.splice(0, this.events.length - 4000)
    },
    drainEvents() {
      return this.events.splice(0)
    },
    async collectStats() {
      const peerConnections = []
      for (const item of this.pcs) {
        const report = await item.pc.getStats()
        peerConnections.push({
          id: item.id,
          states: stateSnapshot(item.pc),
          stats: summarizeStats(report),
        })
      }
      return {
        t: Date.now(),
        url: location.origin + location.pathname,
        peerConnections,
      }
    },
  }
  window.__xbxOfficialCapture = capture
  capture.push({ kind: 'hook-installed', nativePresent: Boolean(Native) })
  if (!Native) return

  function WrappedRTCPeerConnection(config, constraints) {
    const pc = new Native(config, constraints)
    return attachPeerConnection(pc, config)
  }
  Object.setPrototypeOf(WrappedRTCPeerConnection, Native)
  WrappedRTCPeerConnection.prototype = Native.prototype
  window.RTCPeerConnection = WrappedRTCPeerConnection
  if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = WrappedRTCPeerConnection

  function attachPeerConnection(pc, config) {
    const id = capture.nextId++
    capture.pcs.push({ id, pc })
    capture.push({ kind: 'pc-created', id, config: sanitizeConfig(config), states: stateSnapshot(pc) })

    for (const name of ['iceconnectionstatechange', 'connectionstatechange', 'signalingstatechange', 'icegatheringstatechange']) {
      pc.addEventListener(name, () => capture.push({ kind: name, id, states: stateSnapshot(pc) }))
    }
    pc.addEventListener('icecandidate', (event) => capture.push({ kind: 'icecandidate', id, candidate: parseCandidate(event.candidate?.candidate ?? '') }))
    pc.addEventListener('track', (event) => capture.push({ kind: 'track', id, trackKind: event.track?.kind, streams: event.streams?.length ?? 0 }))
    pc.addEventListener('datachannel', (event) => capture.push({ kind: 'datachannel', id, label: event.channel?.label, protocol: event.channel?.protocol }))

    wrapAsync(pc, 'createOffer', async (result) => {
      capture.push({ kind: 'createOffer', id, description: summarizeDescription(result) })
    })
    wrapAsync(pc, 'createAnswer', async (result) => {
      capture.push({ kind: 'createAnswer', id, description: summarizeDescription(result) })
    })
    wrapAsync(pc, 'setLocalDescription', async (_result, args) => {
      capture.push({ kind: 'setLocalDescription', id, description: summarizeDescription(args[0] ?? pc.localDescription), states: stateSnapshot(pc) })
    })
    wrapAsync(pc, 'setRemoteDescription', async (_result, args) => {
      capture.push({ kind: 'setRemoteDescription', id, description: summarizeDescription(args[0] ?? pc.remoteDescription), states: stateSnapshot(pc) })
    })
    wrapAsync(pc, 'addIceCandidate', async (_result, args) => {
      capture.push({ kind: 'addIceCandidate', id, candidate: parseCandidate(args[0]?.candidate ?? ''), states: stateSnapshot(pc) })
    })
    return pc
  }

  function wrapAsync(target, method, after) {
    const original = target[method]
    if (typeof original !== 'function') return
    target[method] = function wrapped(...args) {
      const result = original.apply(this, args)
      Promise.resolve(result)
        .then(value => after(value, args))
        .catch(error => capture.push({ kind: method + '-failed', error: String(error?.message ?? error) }))
      return result
    }
  }

  function sanitizeConfig(config) {
    if (!config) return null
    return {
      iceTransportPolicy: config.iceTransportPolicy,
      bundlePolicy: config.bundlePolicy,
      rtcpMuxPolicy: config.rtcpMuxPolicy,
      iceServers: (config.iceServers ?? []).map(server => ({
        urls: Array.isArray(server.urls) ? server.urls : [server.urls].filter(Boolean),
        usernamePresent: Boolean(server.username),
        credentialPresent: Boolean(server.credential),
      })),
    }
  }

  function summarizeDescription(description) {
    if (!description) return null
    return {
      type: description.type,
      sdp: summarizeSdp(description.sdp ?? ''),
    }
  }

  function summarizeSdp(raw) {
    const lines = String(raw).split(/\\r?\\n/).map(line => line.trim()).filter(Boolean)
    return {
      media: lines.filter(line => line.startsWith('m=')),
      groups: lines.filter(line => line.startsWith('a=group:')),
      mids: lines.filter(line => line.startsWith('a=mid:')),
      directions: lines.filter(line => /^a=(sendrecv|sendonly|recvonly|inactive)$/.test(line)),
      codecs: lines.filter(line => line.startsWith('a=rtpmap:')),
      fmtp: lines.filter(line => line.startsWith('a=fmtp:')),
      rtcpFb: lines.filter(line => line.startsWith('a=rtcp-fb:')),
      extmaps: lines.filter(line => line.startsWith('a=extmap:')),
      setup: lines.filter(line => line.startsWith('a=setup:')),
      iceOptions: lines.filter(line => line.startsWith('a=ice-options:')),
      candidates: lines.filter(line => line.startsWith('a=candidate:')).map(parseCandidate),
      fingerprintPresent: lines.some(line => line.startsWith('a=fingerprint:')),
      iceCredentialsPresent: lines.some(line => line.startsWith('a=ice-ufrag:') || line.startsWith('a=ice-pwd:')),
    }
  }

  function parseCandidate(raw) {
    const text = String(raw).replace(/^a=/, '')
    const parts = text.split(/\\s+/)
    const typIndex = parts.indexOf('typ')
    const address = parts[4]
    return {
      protocol: parts[2]?.toLowerCase(),
      component: parts[1],
      type: typIndex >= 0 ? parts[typIndex + 1] : undefined,
      addressFamily: addressFamily(address),
      tcpType: parts.includes('tcptype') ? parts[parts.indexOf('tcptype') + 1] : undefined,
      relatedAddressFamily: parts.includes('raddr') ? addressFamily(parts[parts.indexOf('raddr') + 1]) : undefined,
      portPresent: Number.isFinite(Number(parts[5])),
    }
  }

  function stateSnapshot(pc) {
    return {
      signalingState: pc.signalingState,
      iceConnectionState: pc.iceConnectionState,
      connectionState: pc.connectionState,
      iceGatheringState: pc.iceGatheringState,
    }
  }

  function summarizeStats(report) {
    const rows = []
    report.forEach((stat) => {
      if (![
        'inbound-rtp',
        'outbound-rtp',
        'remote-inbound-rtp',
        'remote-outbound-rtp',
        'candidate-pair',
        'local-candidate',
        'remote-candidate',
        'transport',
        'codec',
        'data-channel',
      ].includes(stat.type)) return
      rows.push(summarizeStat(stat))
    })
    return rows
  }

  function summarizeStat(stat) {
    const keep = {
      id: stat.id,
      type: stat.type,
      timestamp: stat.timestamp,
      kind: stat.kind,
      mediaType: stat.mediaType,
      mimeType: stat.mimeType,
      clockRate: stat.clockRate,
      sdpFmtpLine: stat.sdpFmtpLine,
      transportId: stat.transportId,
      codecId: stat.codecId,
      localCandidateId: stat.localCandidateId,
      remoteCandidateId: stat.remoteCandidateId,
      selectedCandidatePairId: stat.selectedCandidatePairId,
      state: stat.state,
      nominated: stat.nominated,
      selected: stat.selected,
      candidateType: stat.candidateType,
      protocol: stat.protocol,
      relayProtocol: stat.relayProtocol,
      networkType: stat.networkType,
      url: stat.url ? sanitizeTurnUrl(stat.url) : undefined,
      addressFamily: addressFamily(stat.address ?? stat.ip),
      dtlsState: stat.dtlsState,
      iceRole: stat.iceRole,
      iceState: stat.iceState,
      packetsSent: stat.packetsSent,
      packetsReceived: stat.packetsReceived,
      packetsLost: stat.packetsLost,
      bytesSent: stat.bytesSent,
      bytesReceived: stat.bytesReceived,
      framesEncoded: stat.framesEncoded,
      framesDecoded: stat.framesDecoded,
      framesDropped: stat.framesDropped,
      framesReceived: stat.framesReceived,
      framesPerSecond: stat.framesPerSecond,
      frameWidth: stat.frameWidth,
      frameHeight: stat.frameHeight,
      jitter: stat.jitter,
      jitterBufferDelay: stat.jitterBufferDelay,
      jitterBufferEmittedCount: stat.jitterBufferEmittedCount,
      totalDecodeTime: stat.totalDecodeTime,
      totalInterFrameDelay: stat.totalInterFrameDelay,
      totalSquaredInterFrameDelay: stat.totalSquaredInterFrameDelay,
      freezeCount: stat.freezeCount,
      pauseCount: stat.pauseCount,
      totalFreezesDuration: stat.totalFreezesDuration,
      pliCount: stat.pliCount,
      firCount: stat.firCount,
      nackCount: stat.nackCount,
      qpSum: stat.qpSum,
      currentRoundTripTime: stat.currentRoundTripTime,
      availableOutgoingBitrate: stat.availableOutgoingBitrate,
      availableIncomingBitrate: stat.availableIncomingBitrate,
      requestsSent: stat.requestsSent,
      requestsReceived: stat.requestsReceived,
      responsesSent: stat.responsesSent,
      responsesReceived: stat.responsesReceived,
      consentRequestsSent: stat.consentRequestsSent,
      fractionLost: stat.fractionLost,
      roundTripTime: stat.roundTripTime,
      targetBitrate: stat.targetBitrate,
      qualityLimitationReason: stat.qualityLimitationReason,
      decoderImplementation: stat.decoderImplementation,
      encoderImplementation: stat.encoderImplementation,
      powerEfficientDecoder: stat.powerEfficientDecoder,
      label: stat.label,
      messagesSent: stat.messagesSent,
      messagesReceived: stat.messagesReceived,
    }
    return Object.fromEntries(Object.entries(keep).filter(([, value]) => value !== undefined))
  }

  function sanitizeTurnUrl(raw) {
    try {
      const url = new URL(raw)
      return url.protocol + '//' + url.host
    }
    catch {
      return undefined
    }
  }

  function addressFamily(address) {
    if (typeof address !== 'string' || address.length === 0) return undefined
    if (/^\\d{1,3}(?:\\.\\d{1,3}){3}$/.test(address)) return 'ipv4'
    if (address.includes(':')) return 'ipv6'
    return 'fqdn-or-mdns'
  }
})()`

async function main() {
  args = parseArgs(process.argv.slice(2))
  startedAt = new Date()
  const stamp = startedAt.toISOString().replaceAll(/[:.]/g, '-')
  outDir = args.out ?? `runtime-logs/official-xbox-capture-${stamp}`
  port = Number(args.port ?? DEFAULT_PORT)
  sampleIntervalMs = Number(args.sampleIntervalMs ?? DEFAULT_SAMPLE_INTERVAL_MS)
  profileDir = args.profile ?? `/private/tmp/xbxrc-official-xbox-chrome-profile-${port}`
  targetUrl = args.url ?? DEFAULT_URL

  await mkdir(outDir, { recursive: true })
  eventStream = createWriteStream(`${outDir}/events.jsonl`, { flags: 'a' })
  statsStream = createWriteStream(`${outDir}/webrtc-stats.jsonl`, { flags: 'a' })

  process.on('SIGINT', async () => {
    await finish('sigint')
    process.exit(0)
  })

  await writeFile(`${outDir}/capture-meta.json`, `${JSON.stringify({
    startedAt: startedAt.toISOString(),
    url: targetUrl,
    port,
    profileDir,
    sampleIntervalMs,
  }, null, 2)}\n`)

  if (!args.noLaunch) {
    chrome = launchChrome({ port, profileDir })
    console.log(`[capture] launched Chrome pid=${chrome.pid} profile=${profileDir}`)
  }

  await waitForChrome(port)
  const target = await createBlankTarget(port)
  cdp = new CdpClient(target.webSocketDebuggerUrl)
  await cdp.open()
  await setupPage(cdp)
  await navigate(cdp, targetUrl)

  console.log(`[capture] output=${outDir}`)
  console.log('[capture] 在新 Chrome 窗口登录 Xbox，并切到主机列表。完成后告诉我“已就绪”。')
  console.log('[capture] 可用指令：status, candidates, click-candidate <index>, click-xy <x> <y>, finish')

  sampleTimer = setInterval(() => {
    drainWebrtcEvents().catch(error => writeEvent('collector-error', { phase: 'drain', error: String(error?.message ?? error) }))
    collectStats().catch(error => writeEvent('collector-error', { phase: 'stats', error: String(error?.message ?? error) }))
  }, sampleIntervalMs)

  const rl = createInterface({ input: process.stdin, output: process.stdout })
  for await (const line of rl) {
    const [command, ...rest] = line.trim().split(/\s+/)
    try {
      if (!command) {
        continue
      }
      if (command === 'status') {
        await printStatus()
      }
      else if (command === 'candidates') {
        await printCandidates()
      }
      else if (command === 'click-candidate') {
        await clickCandidate(Number(rest[0]))
      }
      else if (command === 'click-xy') {
        await clickXY(Number(rest[0]), Number(rest[1]))
      }
      else if (command === 'finish') {
        await finish('command')
        process.exit(0)
      }
      else {
        console.log(`[capture] unknown command=${command}`)
      }
    }
    catch (error) {
      console.error(`[capture] command failed: ${String(error?.stack ?? error)}`)
    }
  }
}

main().catch((error) => {
  console.error(`[capture] fatal: ${String(error?.stack ?? error)}`)
  process.exit(1)
})
