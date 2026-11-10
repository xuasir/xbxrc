import { describe, expect, it } from 'vitest'
import { DecodedFrameClock } from './decoded-frame-clock'

describe('bounded Worker clock calibration', () => {
  it('corrects both signs of clock offset with round-trip uncertainty', () => {
    for (const workerOffset of [-500, 500]) {
      const clock = new DecodedFrameClock()
      const probe = clock.probe(1000)!
      expect(clock.accept({ ...probe, receivedAtEpochMs: 1002 + workerOffset, sentAtEpochMs: 1002 + workerOffset }, 1004)).toBe(true)
      expect(clock.convert(990 + workerOffset, 1004)).toEqual({ atMs: 990, uncertaintyMs: 4 })
      expect(clock.snapshot(1004)).toMatchObject({ state: 'calibrated', offsetMs: -workerOffset, uncertaintyMs: 4 })
    }
  })

  it('bounds asymmetric dispatch so freshness uses a conservative age', () => {
    const clock = new DecodedFrameClock()
    const probe = clock.probe(1000)!
    clock.accept({ ...probe, receivedAtEpochMs: 1101, sentAtEpochMs: 1101 }, 1008)
    const converted = clock.convert(1058, 1010)!
    const trueAge = 1010 - 958
    const lowerBound = 1010 - converted.atMs - converted.uncertaintyMs
    expect(lowerBound).toBeLessThanOrEqual(trueAge)
    expect(lowerBound).toBeLessThanOrEqual(50)
    expect(clock.probe(1010)).toBeDefined()
  })

  it('isolates late probes and rejects high RTT or invalid internal order', () => {
    const clock = new DecodedFrameClock()
    const first = clock.probe(0)!
    expect(clock.probe(999)).toBeUndefined()
    const next = clock.probe(1000)!
    expect(clock.accept({ ...first, receivedAtEpochMs: 0, sentAtEpochMs: 0 }, 1001)).toBe(false)
    expect(clock.accept({ ...next, receivedAtEpochMs: 1000, sentAtEpochMs: 1000 }, 1010)).toBe(false)
    const bad = clock.probe(1011)!
    expect(clock.accept({ ...bad, receivedAtEpochMs: 1012, sentAtEpochMs: 1011 }, 1013)).toBe(false)
    expect(clock.snapshot(1013)).toMatchObject({ state: 'uncalibrated', rejected: 2 })
  })

  it('expires old calibration and replaces it with a fresh bounded measurement', () => {
    const clock = new DecodedFrameClock()
    const first = clock.probe(0)!
    clock.accept({ ...first, receivedAtEpochMs: 20, sentAtEpochMs: 20 }, 0)
    expect(clock.convert(20, 15001)).toBeUndefined()
    expect(clock.snapshot(15001).state).toBe('expired')
    const next = clock.probe(15001)!
    clock.accept({ ...next, receivedAtEpochMs: 15024, sentAtEpochMs: 15024 }, 15001)
    expect(clock.convert(15024, 15001)?.atMs).toBe(15001)
  })
})
