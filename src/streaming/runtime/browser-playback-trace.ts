import type { BrowserTraceRecordOptions } from './browser-runtime-trace'
import type { Counter, Counters, PlaybackObservation, PlaybackTraceFacts } from './playback-health'
import { compactDecodedSupply, counterIdentity, COUNTERS, measured, observationIdentity, PlaybackHealthWindow, readCounters } from './playback-health'

interface Episode {
  id: string
  startedAtMs: number
  endAtMs: number
  reasons: Set<string>
  samples: number
  beforeMs: number
}

/** 观测生命周期：使用已有采样和 supervision tick；各缓冲与输出预算固定。 */
export function createBrowserPlaybackTrace(input: {
  enabled: () => boolean
  write: (event: string, payload: Record<string, unknown>, options?: BrowserTraceRecordOptions) => boolean
  facts: () => PlaybackTraceFacts
  trace: () => Record<string, unknown>
  now?: () => number
}) {
  const now = input.now ?? (() => performance.now())
  let active = false
  let startedAtMs = 0
  let nextHealthAtMs = 0
  let nextCheckpointAtMs = 0
  let window = new PlaybackHealthWindow()
  let latest: PlaybackObservation | undefined
  let collecting = false
  let collectingAtMs = 0
  let healthSequence = 0
  let observationEpoch = 0
  let healthRecords = 0
  let validRecords = 0
  let validMs = 0
  let totalWindowMs = 0
  let maxSampleGapMs = 0
  let resets = 0
  let lastManifest = ''
  let episode: Episode | undefined
  let episodeSequence = 0
  let lastEpisodeAtMs = -Infinity
  let anomalyBytes = 0
  let anomalyMinuteAtMs = 0
  let anomalyCount = 0
  let anomalyDurationMs = 0
  let symptomSinceMs: number | undefined
  let lastSymptomAtMs = 0
  const symptomReasons = new Set<string>()
  let suppressed = 0
  const ring: Array<Record<string, unknown>> = []
  const totals: Counters = {}
  const lastCounters = new Map<Counter, { identity: string, value: number }>()
  const encoder = new TextEncoder()

  function emit(event: string, payload: Record<string, unknown>, importance?: 'essential'): boolean {
    try {
      return input.write(event, {
        productionContractVersion: 1,
        observationEpoch,
        ...payload,
      }, {
        intervalManaged: true,
        importance,
      })
    }
    catch {
      suppressed++
      return false
    }
  }

  function manifest(reason: string): void {
    if (!active || !input.enabled())
      return
    const facts = input.facts()
    const stamp = JSON.stringify([facts.configuration, facts.capabilities, input.trace().profile, latest?.sample.peerGeneration, latest?.stats.statsVideoGeneration, latest?.stats.rendererGeneration, facts.visibility])
    if (stamp === lastManifest)
      return
    lastManifest = stamp
    emit('playbackObservationManifest', {
      reason,
      configuration: facts.configuration,
      capabilities: facts.capabilities,
      lifecyclePhase: facts.lifecyclePhase,
      visibility: facts.visibility,
      peerGeneration: latest?.sample.peerGeneration,
      videoGeneration: latest?.stats.statsVideoGeneration,
      rendererGeneration: latest?.stats.rendererGeneration,
      cadence: { collectionMs: 250, healthMs: 1000, checkpointMs: 30000, anomalyMs: 250 },
      clock: { monotonicMs: measured(now(), 1), wallTimeMs: Date.now(), timeOriginMs: measured(performance.timeOrigin, 1) },
    })
  }

  function publishHealth(at: number, partial = false): void {
    if (latest?.stats.rendererDecodedFrameSupply)
      emit('decodedFrameSupplyObserved', compactDecodedSupply(latest.stats))
    const payload = window.payload(at, input.facts(), ++healthSequence, input.trace(), partial, { pending: collecting, startedAtMs: collectingAtMs })
    const coverage = payload.coverage as { validMs: number, elapsedMs: number, ratio: number }
    if (emit('browserPlaybackHealthObserved', payload)) {
      healthRecords++
      if (coverage.ratio >= 0.9)
        validRecords++
    }
    validMs += coverage.validMs
    totalWindowMs += coverage.elapsedMs
    maxSampleGapMs = Math.max(maxSampleGapMs, Number((payload.coverage as Record<string, unknown>).maxSampleGapMs) || 0)
    window.advance((payload.window as { endMs: number }).endMs)
  }

  function summary(ended: boolean, reason: string): void {
    const at = now()
    emit('playbackSessionSummary', {
      ended,
      reason,
      durationMs: measured(at - startedAtMs, 1),
      healthSequence,
      totals: { ...totals },
      coverage: { healthRecords, mediaValidRecords: validRecords, expectedHealthRecords: Math.floor((at - startedAtMs) / 1000), elapsedMs: measured(totalWindowMs, 1), validMs: measured(validMs, 1), ratio: totalWindowMs > 0 ? measured(validMs / totalWindowMs, 4) : 0, maxSampleGapMs: measured(maxSampleGapMs, 1), resetCount: resets },
      anomalies: { count: anomalyCount, active: symptomSinceMs !== undefined, durationMs: measured(anomalyDurationMs + (symptomSinceMs === undefined ? 0 : at - symptomSinceMs), 1), suppressed, bytes: anomalyBytes },
      trace: input.trace(),
      cleanupState: ended ? 'stopping' : 'running',
    }, ended ? 'essential' : undefined)
  }

  function compactSample(observation: PlaybackObservation): Record<string, unknown> {
    const { sample, stats } = observation
    return {
      sampleId: sample.sampleId,
      sampledAtMs: measured(sample.sampledAtMs, 1),
      peerGeneration: sample.peerGeneration,
      videoGeneration: stats.statsVideoGeneration,
      rendererGeneration: stats.rendererGeneration,
      observationState: stats.statsObservationState,
      receivedFps: measured(stats.inboundVideoFps),
      decodeFps: measured(stats.decodeFps),
      videoPresentFps: measured(stats.videoPresentFps),
      callbackFps: measured(stats.callbackFps),
      rendererSubmitFps: measured(stats.rendererSubmitFps),
      rttMs: measured(stats.networkRttMs),
      jitterMs: measured(stats.networkJitterMs),
      lossRatio: measured(stats.networkPacketLossRatio, 4),
      uploadMs: measured(stats.renderUploadDurationMsAvg),
      drawMs: measured(stats.renderDrawDurationMsAvg),
      gpuMs: measured(stats.rendererGpuTiming?.duration.p99Ms),
      submitAgeMs: measured(stats.rendererSubmitAgeMs),
      videoPresentAgeMs: measured(stats.videoPresentAgeMs),
      freezeCount: sample.inboundVideo?.freezeCount,
      framesDropped: sample.inboundVideo?.framesDropped,
      callbackGapMs: measured(stats.maxCallbackIntervalMs),
    }
  }

  function emitAnomalySample(sample: Record<string, unknown>, phase: 'before' | 'after'): void {
    if (!episode)
      return
    const payload = { ...sample, episodeId: episode.id, phase }
    const bytes = encoder.encode(JSON.stringify(payload)).byteLength + 512
    if (bytes > 2048 || anomalyBytes + bytes > 131072) {
      suppressed++
      return
    }
    if (emit('browserPlaybackAnomalySample', payload)) {
      anomalyBytes += bytes
      episode.samples++
    }
  }

  function finishEpisode(reason: string): void {
    if (!episode)
      return
    const duration = Math.max(0, now() - episode.startedAtMs)
    emit('browserPlaybackAnomalyObserved', { episodeId: episode.id, phase: 'windowEnd', reason, reasons: [...episode.reasons], durationMs: measured(duration, 1), sampleCount: episode.samples, coverage: { beforeMs: episode.beforeMs, afterMs: measured(Math.min(10000, duration), 1), partial: now() < episode.endAtMs } })
    episode = undefined
  }

  function trigger(reason: string): void {
    if (!active || !input.enabled())
      return
    const at = now()
    lastSymptomAtMs = at
    if (symptomSinceMs === undefined) {
      symptomSinceMs = at
      anomalyCount++
      symptomReasons.clear()
      emit('browserPlaybackAnomalyObserved', { phase: 'start', episodeId: `${++episodeSequence}`, reason, expectedFps: measured(input.facts().expectedFps) })
    }
    if (!symptomReasons.has(reason)) {
      symptomReasons.add(reason)
      if (symptomReasons.size > 1)
        emit('browserPlaybackAnomalyObserved', { phase: 'upgrade', episodeId: `${episodeSequence}`, reason, reasons: [...symptomReasons] })
    }
    if (episode) {
      episode.reasons.add(reason)
      return
    }
    if (at - lastEpisodeAtMs < 60000) {
      suppressed++
      return
    }
    if (at - anomalyMinuteAtMs >= 60000) {
      anomalyMinuteAtMs = at
      anomalyBytes = 0
    }
    lastEpisodeAtMs = at
    const before = ring.filter(sample => typeof sample.sampledAtMs === 'number' && sample.sampledAtMs >= at - 5000)
    episode = { id: `${episodeSequence}`, startedAtMs: at, endAtMs: at + 10000, reasons: new Set(symptomReasons), samples: 0, beforeMs: before.length ? measured(at - Number(before[0]!.sampledAtMs), 1)! : 0 }
    emit('browserPlaybackAnomalyObserved', { episodeId: episode.id, phase: 'windowStart', reason, expectedFps: measured(input.facts().expectedFps), beforeMs: 5000, afterMs: 10000 })
    before.forEach(sample => emitAnomalySample(sample, 'before'))
  }

  function closeSymptom(reason: string): void {
    if (symptomSinceMs === undefined)
      return
    const durationMs = now() - symptomSinceMs
    anomalyDurationMs += durationMs
    emit('browserPlaybackAnomalyObserved', { episodeId: `${episodeSequence}`, phase: 'end', reason, durationMs: measured(durationMs, 1), reasons: [...symptomReasons] })
    symptomSinceMs = undefined
  }

  function observe(observation: PlaybackObservation): void {
    if (!active || !input.enabled() || observation.sample.sampleId === latest?.sample.sampleId)
      return
    const at = observation.sample.sampledAtMs
    const previous = latest
    if (previous && observationIdentity(previous) !== observationIdentity(observation)) {
      resets++
      finishEpisode('generationReset')
      ring.length = 0
    }
    const counters = readCounters(observation)
    for (const counter of COUNTERS) {
      const value = counters[counter]
      if (value === undefined)
        continue
      const identity = counterIdentity(counter, observation)
      const before = lastCounters.get(counter)
      totals[counter] = (totals[counter] ?? 0) + (before?.identity === identity && value >= before.value ? value - before.value : Math.max(0, value))
      lastCounters.set(counter, { identity, value })
    }
    latest = observation
    window.observe(observation)
    manifest('capabilityChanged')
    const sample = compactSample(observation)
    if (encoder.encode(JSON.stringify(sample)).byteLength <= 1024) {
      ring.push(sample)
      if (ring.length > 60)
        ring.shift()
    }
    const existingEpisode = episode
    if (existingEpisode && at <= existingEpisode.endAtMs)
      emitAnomalySample(sample, 'after')
    if (existingEpisode && at >= existingEpisode.endAtMs)
      finishEpisode('completed')
    const short = observation.stats.browserStatsWindows?.short
    const target = Math.min(input.facts().expectedFps, observation.stats.rendererTargetFps ?? Infinity)
    if ((short?.durationMs ?? 0) >= 2700 && (short?.receivedCoverageMs ?? 0) >= 2700
      && (short?.inboundFpsAvg ?? Infinity) < target * 0.9) {
      trigger('supplyDeficit')
    }
    if ((short?.durationMs ?? 0) >= 2700 && (short?.rendererCoverageMs ?? 0) >= 2700
      && (short?.rendererSubmitFpsAvg ?? Infinity) < target * 0.9) {
      trigger('supplyDeficit')
    }
    if (previous && (observation.sample.inboundVideo?.freezeCount ?? 0) > (previous.sample.inboundVideo?.freezeCount ?? 0))
      trigger('freeze')
    if (previous && (observation.stats.rendererSubmitFailureCount ?? 0) > (previous.stats.rendererSubmitFailureCount ?? 0))
      trigger('submitFailed')
    if ((observation.stats.maxCallbackIntervalMs ?? 0) > 100)
      trigger('callbackGap')
    if ((observation.stats.rendererSubmitAgeMs ?? 0) > 100)
      trigger('submitDelayed')
    if (at >= nextHealthAtMs && (window.startMs === undefined || at - window.startMs >= 750)) {
      publishHealth(at)
      while (nextHealthAtMs <= at) nextHealthAtMs += 1000
    }
  }

  function tick(): void {
    if (!active || !input.enabled())
      return
    const at = now()
    if (episode && at >= episode.endAtMs)
      finishEpisode('completed')
    if (at >= nextHealthAtMs && (!latest || at - latest.sample.sampledAtMs > 1000 || (collecting && at >= nextHealthAtMs + 250))) {
      const facts = input.facts()
      publishHealth(at, true)
      while (nextHealthAtMs <= at) nextHealthAtMs += 1000
      if (facts.lifecyclePhase === 'connected' || latest?.sample.connectionState === 'connected')
        trigger('samplingUnavailable')
    }
    if (symptomSinceMs !== undefined && at - lastSymptomAtMs > 1000)
      closeSymptom('recovered')
    if (at >= nextCheckpointAtMs) {
      summary(false, 'checkpoint')
      nextCheckpointAtMs = at + 30000
    }
  }

  return {
    begin(): void {
      if (active)
        this.end('superseded')
      active = true
      observationEpoch++
      startedAtMs = now()
      nextHealthAtMs = startedAtMs + 1000
      nextCheckpointAtMs = startedAtMs + 30000
      window = new PlaybackHealthWindow()
      latest = undefined
      healthSequence = healthRecords = validRecords = validMs = totalWindowMs = maxSampleGapMs = resets = 0
      ring.length = 0
      lastCounters.clear()
      COUNTERS.forEach(counter => delete totals[counter])
      lastManifest = ''
      episode = undefined
      symptomSinceMs = undefined
      symptomReasons.clear()
      collecting = false
      episodeSequence = anomalyCount = anomalyDurationMs = suppressed = anomalyBytes = 0
      lastEpisodeAtMs = -Infinity
      anomalyMinuteAtMs = startedAtMs
      manifest('initial')
    },
    observe,
    collectionState(state: { pending: boolean, startedAtMs: number }): void {
      collecting = state.pending
      collectingAtMs = state.startedAtMs
      if (collecting)
        tick()
    },
    tick,
    trigger,
    manifest,
    end(reason = 'sessionEnded'): void {
      if (!active)
        return
      if (input.enabled()) {
        if (window.sampleCount > 0 || now() - (window.startMs ?? now()) > 0)
          publishHealth(now(), true)
        finishEpisode(reason)
        closeSymptom(reason)
        summary(true, reason)
      }
      active = false
      ring.length = 0
    },
    snapshot: () => ({ active, ringSamples: ring.length, healthSequence, healthRecords, episodeActive: episode !== undefined, anomalyBytes, suppressed, collecting, collectionAgeMs: collecting ? measured(now() - collectingAtMs) : undefined }),
  }
}
