<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto, LogicalButtonDto } from '@shared/gamepad/contract'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import SettingModalShell from './SettingModalShell.vue'
import { events } from '../../services/events'

interface Props {
  open: boolean
  scopeId: string
  snapshot: GamepadRuntimeSnapshotDto | null
}

const props = defineProps<Props>()

const emit = defineEmits<{
  (event: 'close'): void
}>()

const lastPadSnapshot = ref<LogicalPadSnapshotDto | null>(null)
let lastPadSnapshotAt = 0
let disposeGamepadPadSnapshot: EventUnsubscribe | undefined

const LOGICAL_BUTTON_LABEL: Array<{ key: keyof LogicalPadSnapshotDto['state']['buttons'], label: string, logical: LogicalButtonDto }> = [
  { key: 'south', label: 'A', logical: 'south' },
  { key: 'east', label: 'B', logical: 'east' },
  { key: 'west', label: 'X', logical: 'west' },
  { key: 'north', label: 'Y', logical: 'north' },
  { key: 'l1', label: 'LB', logical: 'l1' },
  { key: 'r1', label: 'RB', logical: 'r1' },
  { key: 'l2', label: 'LT', logical: 'l2' },
  { key: 'r2', label: 'RT', logical: 'r2' },
  { key: 'l3', label: 'LS', logical: 'l3' },
  { key: 'r3', label: 'RS', logical: 'r3' },
  { key: 'view', label: 'View', logical: 'view' },
  { key: 'menu', label: 'Menu', logical: 'menu' },
  { key: 'home', label: 'Xbox', logical: 'home' },
  { key: 'dpadUp', label: 'DPad Up', logical: 'dpad-up' },
  { key: 'dpadDown', label: 'DPad Down', logical: 'dpad-down' },
  { key: 'dpadLeft', label: 'DPad Left', logical: 'dpad-left' },
  { key: 'dpadRight', label: 'DPad Right', logical: 'dpad-right' },
]

const pressedButtons = computed(() => {
  const snapshot = lastPadSnapshot.value
  if (snapshot === null) {
    return []
  }
  return LOGICAL_BUTTON_LABEL
    .map((item) => {
      const value = snapshot.state.buttons[item.key]
      if (value <= 0.5) {
        return null
      }
      return {
        label: item.label,
        logical: item.logical,
        value,
      }
    })
    .filter(item => item !== null)
})

const activeSlotText = computed(() => {
  if (lastPadSnapshot.value === null) {
    return '未检测到实时输入'
  }
  return `当前槽位：${lastPadSnapshot.value.slot}`
})

function handleClose(): void {
  emit('close')
}

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
  }
  event.preventDefault()
  event.stopPropagation()
  handleClose()
}

function attachSnapshotListener(): void {
  if (disposeGamepadPadSnapshot !== undefined) {
    return
  }
  disposeGamepadPadSnapshot = events.on('gamepad.slotSnapshot', (snapshot) => {
    const now = Date.now()
    if (now - lastPadSnapshotAt < 100) {
      return
    }
    lastPadSnapshotAt = now
    lastPadSnapshot.value = snapshot
  })
}

function detachSnapshotListener(): void {
  if (disposeGamepadPadSnapshot !== undefined) {
    disposeGamepadPadSnapshot()
    disposeGamepadPadSnapshot = undefined
  }
}

watch(
  () => props.open,
  (open) => {
    window.removeEventListener('keydown', handleEscapeKeydown, true)
    if (open) {
      window.addEventListener('keydown', handleEscapeKeydown, true)
      attachSnapshotListener()
      return
    }
    detachSnapshotListener()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleEscapeKeydown, true)
  detachSnapshotListener()
})
</script>

<template>
  <SettingModalShell
    :open="props.open"
    :scope-id="props.scopeId"
    title="输入调试视图"
    hint="展示当前按下按钮及其布局含义，并实时显示摇杆/扳机值。"
    width="min(100%, 760px)"
    max-height="min(90vh, 860px)"
    @close="handleClose"
  >
    <div class="setting-input-debug-sheet__scroll">
      <p class="setting-input-debug-sheet__slot">
        {{ activeSlotText }}
      </p>

      <section class="setting-input-debug-sheet__section">
        <p class="setting-input-debug-sheet__section-title">
          按下按钮
        </p>
        <div v-if="pressedButtons.length > 0" class="setting-input-debug-sheet__pressed-list">
          <article
            v-for="item in pressedButtons"
            :key="item.logical"
            class="setting-input-debug-sheet__pressed-item"
          >
            <span class="setting-input-debug-sheet__pressed-label">{{ item.label }}</span>
            <span class="setting-input-debug-sheet__pressed-logical">{{ item.logical }}</span>
            <span class="setting-input-debug-sheet__pressed-value">{{ item.value.toFixed(2) }}</span>
          </article>
        </div>
        <p v-else class="setting-input-debug-sheet__empty">
          当前无按下按钮
        </p>
      </section>

      <section v-if="lastPadSnapshot !== null" class="setting-input-debug-sheet__section">
        <p class="setting-input-debug-sheet__section-title">
          摇杆 / 扳机实时值
        </p>
        <div class="setting-input-debug-sheet__metrics">
          <article class="setting-input-debug-sheet__metric">
            <span class="setting-input-debug-sheet__metric-key">LS</span>
            <span class="setting-input-debug-sheet__metric-value">
              ({{ lastPadSnapshot.state.leftStick.x.toFixed(2) }}, {{ lastPadSnapshot.state.leftStick.y.toFixed(2) }})
            </span>
          </article>
          <article class="setting-input-debug-sheet__metric">
            <span class="setting-input-debug-sheet__metric-key">RS</span>
            <span class="setting-input-debug-sheet__metric-value">
              ({{ lastPadSnapshot.state.rightStick.x.toFixed(2) }}, {{ lastPadSnapshot.state.rightStick.y.toFixed(2) }})
            </span>
          </article>
          <article class="setting-input-debug-sheet__metric">
            <span class="setting-input-debug-sheet__metric-key">LT</span>
            <span class="setting-input-debug-sheet__metric-value">{{ lastPadSnapshot.state.leftTrigger.toFixed(2) }}</span>
          </article>
          <article class="setting-input-debug-sheet__metric">
            <span class="setting-input-debug-sheet__metric-key">RT</span>
            <span class="setting-input-debug-sheet__metric-value">{{ lastPadSnapshot.state.rightTrigger.toFixed(2) }}</span>
          </article>
        </div>
      </section>
    </div>
  </SettingModalShell>
</template>

<style scoped>
.setting-input-debug-sheet__scroll {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: min(70vh, 640px);
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding: 16px 10px;
}

.setting-input-debug-sheet__slot {
  margin: 0;
  font-size: 13px;
  color: var(--color-text-secondary);
}

.setting-input-debug-sheet__section {
  border: 1px solid var(--ui-border-subtle);
  border-radius: 10px;
  background: color-mix(in srgb, var(--ui-surface-panel), transparent 8%);
  padding: 12px;
}

.setting-input-debug-sheet__section-title {
  margin: 0 0 8px;
  font-size: 13px;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-input-debug-sheet__pressed-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.setting-input-debug-sheet__pressed-item {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: var(--ui-surface-overlay);
  padding: 8px 10px;
}

.setting-input-debug-sheet__pressed-label {
  font-weight: var(--ui-font-weight-bold);
}

.setting-input-debug-sheet__pressed-logical {
  color: var(--color-text-secondary);
  font-family: var(--ui-font-mono, monospace);
  font-size: 12px;
}

.setting-input-debug-sheet__pressed-value {
  font-family: var(--ui-font-mono, monospace);
  font-size: 12px;
}

.setting-input-debug-sheet__empty {
  margin: 0;
  font-size: 13px;
  color: var(--color-text-tertiary);
}

.setting-input-debug-sheet__metrics {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.setting-input-debug-sheet__metric {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: var(--ui-surface-overlay);
  padding: 8px 10px;
}

.setting-input-debug-sheet__metric-key {
  font-weight: var(--ui-font-weight-bold);
}

.setting-input-debug-sheet__metric-value {
  font-family: var(--ui-font-mono, monospace);
  color: var(--color-text-secondary);
}

::global(html[data-ui-density='narrow']) .setting-input-debug-sheet__pressed-list,
::global(html[data-ui-density='narrow']) .setting-input-debug-sheet__metrics {
  grid-template-columns: 1fr;
}
</style>

