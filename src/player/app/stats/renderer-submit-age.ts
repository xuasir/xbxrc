import type { RendererObservationSnapshot } from '../../infra/render/render-observation'

/** 新鲜 Worker 快照的提交年龄在生产者采样点计算，快照交付年龄独立保留。 */
export function rendererSubmitAge(renderer: RendererObservationSnapshot | undefined, atMs: number) {
  const lastDraw = renderer?.resources.lastDrawAtMs
  if (lastDraw === undefined)
    return {}
  const execution = renderer?.resources.execution
  const worker = execution?.mode && execution.mode !== 'main'
  const sampleAt = worker ? execution.observedAtMs : atMs
  if (sampleAt === undefined)
    return {}
  const observedAtMs = atMs - sampleAt <= 250 ? sampleAt : atMs
  return { ageMs: Math.max(0, observedAtMs - lastDraw), observedAtMs }
}
