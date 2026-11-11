<script setup lang="ts">
import type { DiagnosticRecord, DiagnosticSaveParams, RuntimeTraceStatus } from '@shared/rpc/runtimeTrace'
import { computed, onActivated, onDeactivated, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { useDiagnosticExport } from '../../composables/useDiagnosticExport'
import { rpc } from '../../services/rpc'

const props = defineProps<{ scopeId: string }>()
const { t, locale } = useI18n()
const status = ref<RuntimeTraceStatus | null>(null)
const loading = ref(false)
const mutating = ref(false)
const error = ref('')
const toolsOpen = ref(false)
const exporting = useDiagnosticExport()
const busy = computed(() => mutating.value || exporting.busy.value)
const capture = computed(() => status.value?.diagnostics.capture)
const activeCapture = computed(() => capture.value?.state === 'active')
const records = computed(() => status.value?.diagnostics.records ?? [])
const now = ref(Date.now())
const remaining = computed(() => {
  const seconds = Math.max(0, Math.ceil(((capture.value?.expiresAtMs ?? now.value) - now.value) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
})
let active = true
let disposed = false
let timer: ReturnType<typeof setTimeout> | undefined
let generation = 0

async function refresh() {
  if (loading.value || disposed || !active || document.hidden)
    return
  const revision = generation
  loading.value = true
  try {
    const result = await rpc.runtimeTrace.getStatus()
    if (revision === generation && active && !disposed) {
      status.value = result
      now.value = Date.now()
      if (error.value === t('diagnostics.statusFailed'))
        error.value = ''
    }
  }
  catch {
    if (revision === generation && !disposed)
      error.value = t('diagnostics.statusFailed')
  }
  finally { loading.value = false }
}
function stopPolling() {
  generation++
  clearTimeout(timer)
}
function poll() {
  clearTimeout(timer)
  if (disposed || !active || document.hidden)
    return
  void refresh().finally(() => {
    if (active && !disposed && !document.hidden)
      timer = setTimeout(poll, 2000)
  })
}
function visibility() {
  stopPolling()
  poll()
}
onMounted(() => {
  document.addEventListener('visibilitychange', visibility)
  poll()
})
onActivated(() => {
  active = true
  poll()
})
onDeactivated(() => {
  active = false
  stopPolling()
})
onUnmounted(() => {
  disposed = true
  stopPolling()
  document.removeEventListener('visibilitychange', visibility)
})

function selection(record: DiagnosticRecord): DiagnosticSaveParams {
  return record.kind === 'capture' ? { captureId: record.id } : { issueId: record.id }
}
async function primary() {
  if (busy.value)
    return
  if (!status.value) {
    await refresh()
    return
  }
  mutating.value = true
  error.value = ''
  let stoppedId: string | undefined
  try {
    if (activeCapture.value && capture.value) {
      const id = capture.value.id
      await rpc.runtimeTrace.stopCapture({ captureId: id })
      stoppedId = id
    }
    else { await rpc.runtimeTrace.startCapture() }
  }
  catch { error.value = t('diagnostics.actionFailed') }
  await refresh()
  if (stoppedId)
    await exporting.save({ captureId: stoppedId })
  mutating.value = false
}
async function setMode(lightweight: boolean) {
  if (busy.value || !status.value || status.value.diagnostics.lightweight === lightweight)
    return
  mutating.value = true
  error.value = ''
  try {
    await rpc.config.set({ patch: { diagnostics_lightweight: lightweight } })
  }
  catch { error.value = t('diagnostics.actionFailed') }
  finally { mutating.value = false }
  await refresh()
}
async function tool(kind: 'openDevTools' | 'openLogDirectory') {
  if (busy.value)
    return
  mutating.value = true
  error.value = ''
  try {
    await rpc.runtimeTrace[kind]()
  }
  catch { error.value = t('diagnostics.actionFailed') }
  finally { mutating.value = false }
}
function time(value: number) {
  return new Date(value).toLocaleString(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' })
}
function range(record: DiagnosticRecord) {
  const start = new Date(record.startedAtMs)
  const end = new Date(record.endedAtMs)
  const ending = start.toDateString() === end.toDateString()
    ? end.toLocaleTimeString(locale.value, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : time(record.endedAtMs)
  return `${time(record.startedAtMs)} — ${ending}`
}
function operation(record: DiagnosticRecord) {
  if (record.kind === 'capture')
    return t('diagnostics.manualRecord')
  const key = record.operation === 'data.getHosts' ? 'hostsOperation' : record.operation?.startsWith('auth.') || record.operation === 'frontend.login' ? 'loginOperation' : record.operation?.startsWith('config.') || record.operation === 'frontend.settings' ? 'settingsOperation' : record.operation?.startsWith('updater.') || record.operation === 'frontend.update' ? 'updateOperation' : record.source === 'media' || record.operation?.startsWith('streaming.') || record.operation === 'frontend.stream' ? 'playbackOperation' : record.source === 'input' ? 'inputOperation' : 'dataOperation'
  return t(`diagnostics.${key}`)
}
</script>

<template>
  <section class="setting-panel__section diagnostic-panel" :aria-label="t('diagnostics.title')">
    <div v-if="status?.diagnostics.effectiveMode === 'production'" class="diagnostic-panel__mode-row">
      <div class="diagnostic-panel__heading">
        <h2>{{ t('diagnostics.backgroundMode') }}</h2>
        <div class="diagnostic-panel__choices" role="radiogroup" :aria-label="t('diagnostics.backgroundMode')" aria-describedby="diagnostic-mode-help">
          <Focusable v-for="choice in [{ key: 'regular', lightweight: false }, { key: 'lightweight', lightweight: true }]" :id="`setting.diagnostics.mode.${choice.key}`" :key="choice.key" as="button" type="button" :scope-id="props.scopeId" class="diagnostic-panel__choice" role="radio" :aria-checked="status.diagnostics.lightweight === choice.lightweight" :busy="busy" @click="setMode(choice.lightweight)">
            {{ t(`diagnostics.${choice.key}`) }}
          </Focusable>
        </div>
      </div>
      <p id="diagnostic-mode-help" class="diagnostic-panel__copy">
        {{ t(status.diagnostics.lightweight ? 'diagnostics.lightweightHelp' : 'diagnostics.regularHelp') }}
      </p>
    </div>
    <div v-else-if="status" class="diagnostic-panel__mode-row">
      <h2>{{ t('diagnostics.development') }}</h2>
      <p class="diagnostic-panel__copy">
        {{ t('diagnostics.help') }}
      </p>
    </div>
    <div class="diagnostic-panel__controls">
      <p class="diagnostic-panel__status" role="status">
        {{ !status ? t('diagnostics.loading') : !status.diagnostics.recordingAvailable ? t('diagnostics.unavailable') : activeCapture ? t('diagnostics.capturing', { remaining }) : t('diagnostics.ready') }}
      </p>
      <div class="setting-panel__actions">
        <Focusable id="setting.diagnostics.capture" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" :busy="busy || loading" :disabled="!!status && !status.diagnostics.recordingAvailable" @click="primary">
          {{ t(activeCapture ? 'diagnostics.stopExport' : status ? 'diagnostics.start' : 'diagnostics.retry') }}
        </Focusable>
        <Focusable id="setting.diagnostics.export" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action" :busy="busy" @click="exporting.save(activeCapture && capture ? { captureId: capture.id } : records[0] ? selection(records[0]) : {})">
          {{ t(exporting.busy.value ? 'diagnostics.exporting' : 'diagnostics.export') }}
        </Focusable>
      </div>
    </div>
    <p v-if="error || exporting.feedback.value" class="setting-panel__status" :class="{ 'setting-panel__status--error': error || exporting.failed.value }" :role="error || exporting.failed.value ? 'alert' : 'status'">
      {{ error || exporting.feedback.value }}
    </p>
    <div class="diagnostic-panel__history">
      <div class="diagnostic-panel__heading">
        <h3>{{ t('diagnostics.records') }}</h3>
        <span class="diagnostic-panel__copy">{{ records.length }}</span>
      </div>
      <p class="diagnostic-panel__copy">
        {{ t('diagnostics.retention') }}
      </p>
      <p v-if="!records.length" class="diagnostic-panel__copy">
        {{ t('diagnostics.emptyRecords') }}
      </p>
      <article v-for="record in records" :key="record.id" class="diagnostic-panel__record">
        <div class="diagnostic-panel__record-copy">
          <strong>{{ operation(record) }}<span v-if="record.count > 1"> · {{ t('diagnostics.repeated', { count: record.count }) }}</span></strong>
          <span>{{ range(record) }} · {{ t(`diagnostics.recordState.${record.state}`) }}</span>
          <span v-if="!record.available">{{ t('diagnostics.recordUnavailable') }}</span>
        </div>
        <Focusable :id="`setting.diagnostics.record.${record.id}`" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action" :busy="busy" :disabled="!record.available" @click="exporting.save(selection(record))">
          {{ t(record.state === 'active' ? 'diagnostics.exportCurrent' : 'diagnostics.export') }}
        </Focusable>
      </article>
    </div>
    <template v-if="status?.diagnostics.effectiveMode === 'development'">
      <Focusable id="setting.diagnostics.tools" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action" :aria-expanded="toolsOpen" @click="toolsOpen = !toolsOpen">
        {{ t('diagnostics.developerTools') }}
      </Focusable>
      <div v-if="toolsOpen" class="setting-panel__actions">
        <Focusable id="setting.diagnostics.devtools" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action" :busy="busy" @click="tool('openDevTools')">
          {{ t('diagnostics.devtools') }}
        </Focusable>
        <Focusable id="setting.diagnostics.logDirectory" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action" :busy="busy" @click="tool('openLogDirectory')">
          {{ t('diagnostics.logDirectory') }}
        </Focusable>
      </div>
    </template>
  </section>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.diagnostic-panel { display: grid; gap: var(--ui-space-lg); padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.diagnostic-panel__mode-row { display: grid; gap: var(--ui-space-sm); min-width: 0; }
.diagnostic-panel__heading { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--ui-space-md); }
.diagnostic-panel h2, .diagnostic-panel h3 { margin: 0; font-size: var(--ui-settings-row-label-size); color: var(--color-text-primary); }
.diagnostic-panel__copy { margin: 0; font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); color: var(--color-text-secondary); }
.diagnostic-panel__controls { display: flex; flex-wrap: wrap; align-items: center; gap: var(--ui-space-md); }
.diagnostic-panel__status { flex: 1 1 180px; margin: 0; color: var(--color-text-secondary); font-size: var(--ui-settings-row-description-size); }
.diagnostic-panel__controls .setting-panel__actions { margin: 0; }
.diagnostic-panel .setting-panel__action { justify-self: start; min-width: 0; min-height: 34px; padding: 6px 12px; font-size: var(--ui-settings-row-description-size); font-weight: var(--ui-font-weight-medium); line-height: 1.4; }
.diagnostic-panel__controls .setting-panel__action { min-width: 96px; }
.diagnostic-panel__choices { display: inline-flex; gap: 2px; padding: 2px; border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); }
.diagnostic-panel__choice { min-height: 30px; padding: 4px 12px; color: var(--color-text-secondary); border: 1px solid transparent; border-radius: var(--ui-radius-sm); background: transparent; font-size: var(--ui-settings-row-description-size); font-weight: var(--ui-font-weight-medium); line-height: 1.4; }
.diagnostic-panel__choice[aria-checked='true'] { color: var(--color-text-primary); border-color: var(--ui-border-subtle); background: var(--ui-material-selected); }
.diagnostic-panel__choice[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.diagnostic-panel__history { display: grid; gap: var(--ui-space-sm); padding-top: var(--ui-space-lg); border-top: 1px solid var(--ui-border-subtle); min-width: 0; }
.diagnostic-panel__record { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--ui-space-md); padding-block: var(--ui-space-sm); }
.diagnostic-panel__record-copy { display: grid; flex: 1 1 160px; gap: var(--ui-space-xs); min-width: 0; }
.diagnostic-panel__record-copy strong { color: var(--color-text-primary); font-size: var(--ui-settings-row-description-size); }
.diagnostic-panel__record-copy span { color: var(--color-text-secondary); font-size: var(--ui-settings-effect-size); overflow-wrap: anywhere; }
</style>
