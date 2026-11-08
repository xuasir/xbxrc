import { describe, expect, it } from 'vitest'
import { productionTraceEvents, projectProductionTracePayload } from './production-trace-contract'
import contract from './production-trace-contract.json'
import fixtures from './production-trace.fixture.json'

describe('shared production evidence contract', () => {
  it('projects the same canonical fixtures as the Rust boundary', () => {
    for (const fixture of fixtures) {
      expect(projectProductionTracePayload(fixture.event, fixture.input)).toEqual(fixture.expected)
    }
    expect(productionTraceEvents).toEqual(contract.events)
    expect(new Set(productionTraceEvents.map(item => item.event)).size).toBe(productionTraceEvents.length)
  })
  it('keeps finite zero and rejects unsafe values before IPC', () => {
    const payload = projectProductionTracePayload('browserPlaybackAnomalySample', { decodeFps: 0, gpuMs: Number.NaN, rttMs: Infinity, sampleId: 'https://private/token', phase: 'after' })
    expect(payload).toEqual({ productionContractVersion: 1, decodeFps: 0, phase: 'after' })
  })
})
