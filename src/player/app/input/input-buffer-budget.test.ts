import { describe, expect, it } from 'vitest'
import { InputBufferBudget } from './input-buffer-budget'

describe('browser buffer age and drain budget', () => {
  it('tracks partial drain age and accounts for newly accepted bytes', () => {
    const budget = new InputBufferBudget()
    budget.observe(0, 0)
    budget.accepted(100, 0)
    budget.accepted(100, 10)
    budget.observe(150, 20)
    expect(budget.oldestAgeMs(50)).toBe(50)
    budget.observe(100, 50)
    expect(budget.oldestAgeMs(50)).toBe(40)
    expect(budget.snapshot(50)).toMatchObject({ bufferedBytes: 100, drainedBytes: 100, drainBytesPerSecond: 2000, softLimitBytes: 612, drainAge: { p95Ms: 50 } })
    budget.observe(0, 60)
    expect(budget.snapshot(60)).toMatchObject({ bufferedBytes: 0, ledgerEntries: 0, oldestBufferedAgeMs: 0, drainAge: { count: 2 } })
  })

  it('bounds a long ledger, observed foreign bytes and high drain rates', () => {
    const budget = new InputBufferBudget()
    budget.observe(0, 0)
    for (let i = 0; i < 600; i++) budget.accepted(100, i / 100)
    expect(budget.snapshot(10)).toMatchObject({ ledgerEntries: 512, bufferedBytes: 60000 })
    budget.observe(0, 50)
    expect(budget.softLimitBytes).toBe(4096)
    budget.observe(100, 60)
    expect(budget.oldestAgeMs(100)).toBe(40)
    budget.observe(100, 150)
    expect(budget.softLimitBytes).toBe(512)
    budget.reset()
    expect(budget.snapshot(150)).toMatchObject({ ledgerEntries: 0, bufferedBytes: 0, softLimitBytes: 512 })
  })
})
