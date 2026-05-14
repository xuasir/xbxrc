import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto, LogicalPadStateDto } from '@shared/gamepad/contract'
import type { StreamErrorKind } from './types'
import { businessInputArbiter } from '@shared/gamepad/business-input-arbiter'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

type BrowserTimeout = number

export type StreamPageActiveSheet = 'none' | 'menu' | 'diagnosticsMenu' | 'display' | 'audio' | 'text'
export type StreamPageOverlayState = 'none' | 'loading' | 'error' | 'connecting'

interface UseXStreamPageUiOptions {
  getIsConnected: () => boolean
  getIsLoading: () => boolean
  getHasError: () => boolean
  getErrorKind: () => StreamErrorKind
  getWarningVisible: () => boolean
}

const CHROME_HIDE_DELAY_MS = 2_000
const REVEAL_EVENTS: Array<keyof WindowEventMap> = [
  'mousemove',
  'mousedown',
  'touchstart',
  'touchmove',
  'keydown',
]

function isLogicalPadStateNeutral(state: LogicalPadStateDto): boolean {
  return (
    Object.values(state.buttons).every(value => value === 0)
    && state.leftStick.x === 0
    && state.leftStick.y === 0
    && state.rightStick.x === 0
    && state.rightStick.y === 0
    && state.leftTrigger === 0
    && state.rightTrigger === 0
  )
}

function areAllSlotsNeutral(slots: readonly LogicalPadSnapshotDto[]): boolean {
  return slots.every(slot => isLogicalPadStateNeutral(slot.state))
}

function syncStreamUiInputMode(enabled: boolean, overlayOpen: boolean): void {
  window.dispatchEvent(
    new CustomEvent('stream-ui-input-mode', {
      detail: { enabled, overlayOpen },
    }),
  )
}

/**
 * XStream 页面自己的临时 UI 状态，不放到 streaming 域内继续扩散。
 */
export function useXStreamPageUi(options: UseXStreamPageUiOptions) {
  const activeSheet = ref<StreamPageActiveSheet>('none')
  const chromeVisible = ref(true)
  const chromeTimer = ref<BrowserTimeout | null>(null)
  const cleanupFns: Array<() => void> = []
  const latestSlots = new Map<string, LogicalPadSnapshotDto>()
  const pendingResumeStream = ref(false)
  const hasKnownRuntimeSnapshot = ref(false)

  const showFailedSheet = computed(
    () => options.getHasError() && options.getErrorKind() === 'connectionFailed',
  )
  const showWarningSheet = computed(
    () => options.getWarningVisible() && !options.getHasError(),
  )
  const overlayState = computed<StreamPageOverlayState>(() => {
    if (options.getIsLoading()) {
      return 'loading'
    }
    if (options.getHasError() && !showFailedSheet.value) {
      return 'error'
    }
    if (!options.getIsConnected()) {
      return 'connecting'
    }
    return 'none'
  })
  const hasOverlay = computed(
    () =>
      overlayState.value !== 'none'
      || showFailedSheet.value
      || showWarningSheet.value
      || activeSheet.value !== 'none',
  )
  const shouldShowChrome = computed(
    () => !options.getIsConnected() || hasOverlay.value || chromeVisible.value,
  )
  const shouldEnableSpatialInput = computed(
    // 只要 stream 页 chrome 处于可交互态，就切回 UI 输入模式。
    // 否则会出现 topbar 已显示，但 spatial navigation 仍被关掉的错位。
    () => !options.getIsConnected() || hasOverlay.value || shouldShowChrome.value,
  )

  function clearChromeTimer(): void {
    if (chromeTimer.value !== null) {
      window.clearTimeout(chromeTimer.value)
      chromeTimer.value = null
    }
  }

  function scheduleChromeHide(): void {
    clearChromeTimer()
    if (!options.getIsConnected() || hasOverlay.value) {
      chromeVisible.value = true
      return
    }

    chromeTimer.value = window.setTimeout(() => {
      chromeVisible.value = false
    }, CHROME_HIDE_DELAY_MS)
  }

  function revealChrome(): void {
    chromeVisible.value = true
    scheduleChromeHide()
  }

  function openSheet(sheet: Exclude<StreamPageActiveSheet, 'none'>): void {
    revealChrome()
    pendingResumeStream.value = false
    activeSheet.value = sheet
  }

  function flushPendingResumeIfNeutral(): void {
    if (!pendingResumeStream.value) {
      return
    }
    if (!hasKnownRuntimeSnapshot.value) {
      return
    }
    if (!areAllSlotsNeutral(Array.from(latestSlots.values()))) {
      return
    }
    pendingResumeStream.value = false
    businessInputArbiter.applyActionOutcome({ kind: 'resume-stream' })
  }

  function requestResumeStreamAfterNeutral(): void {
    pendingResumeStream.value = true
    flushPendingResumeIfNeutral()
  }

  function closeSheet(sheet?: Exclude<StreamPageActiveSheet, 'none'>): void {
    if (sheet !== undefined && activeSheet.value !== sheet) {
      return
    }
    activeSheet.value = 'none'
    requestResumeStreamAfterNeutral()
  }

  function applyRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    hasKnownRuntimeSnapshot.value = true
    latestSlots.clear()
    for (const slot of snapshot.slots) {
      latestSlots.set(slot.slot, slot)
    }
    flushPendingResumeIfNeutral()
  }

  function applySlotSnapshot(snapshot: LogicalPadSnapshotDto): void {
    latestSlots.set(snapshot.slot, snapshot)
    flushPendingResumeIfNeutral()
  }

  async function refreshRuntimeSnapshot(): Promise<void> {
    try {
      applyRuntimeSnapshot(await rpc.gamepad.getRuntimeSnapshot())
    }
    catch {
      // runtime snapshot 拉取失败不阻断 overlay 逻辑；后续增量事件仍可推进 neutral 检测。
    }
  }

  watch(
    () => [options.getIsConnected(), hasOverlay.value] as const,
    ([connected]) => {
      if (!connected) {
        clearChromeTimer()
        chromeVisible.value = true
        return
      }

      scheduleChromeHide()
    },
    { immediate: true },
  )

  watch(
    () => ({
      spatial: shouldEnableSpatialInput.value,
      overlay: hasOverlay.value,
    }),
    ({ spatial, overlay }) => {
      syncStreamUiInputMode(spatial, overlay)
    },
    { immediate: true },
  )

  onMounted(() => {
    for (const eventName of REVEAL_EVENTS) {
      window.addEventListener(eventName, revealChrome)
    }
    const disposeRuntime = events.on('gamepad.runtimeSnapshot', applyRuntimeSnapshot)
    const disposeSlot = events.on('gamepad.slotSnapshot', applySlotSnapshot)
    cleanupFns.push(disposeRuntime, disposeSlot)
    void refreshRuntimeSnapshot()
  })

  onBeforeUnmount(() => {
    clearChromeTimer()
    syncStreamUiInputMode(true, false)
    for (const eventName of REVEAL_EVENTS) {
      window.removeEventListener(eventName, revealChrome)
    }
    pendingResumeStream.value = false
    hasKnownRuntimeSnapshot.value = false
    latestSlots.clear()
    for (const cleanup of cleanupFns) {
      cleanup()
    }
    cleanupFns.length = 0
  })

  return {
    state: {
      overlayState,
      showFailedSheet,
      showWarningSheet,
      shouldShowChrome,
      isMenuSheetOpen: computed(() => activeSheet.value === 'menu'),
      isDiagnosticsMenuSheetOpen: computed(() => activeSheet.value === 'diagnosticsMenu'),
      isDisplaySheetOpen: computed(() => activeSheet.value === 'display'),
      isAudioSheetOpen: computed(() => activeSheet.value === 'audio'),
      isTextSheetOpen: computed(() => activeSheet.value === 'text'),
    },
    actions: {
      revealChrome,
      openSheet,
      closeSheet,
    },
  }
}
