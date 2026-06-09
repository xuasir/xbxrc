import type { UiCaptureReason, UiReleaseReason } from '../pages/stream/stream-input-route-controller'
import type { StreamErrorKind } from './types'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { streamInputRouteController } from '../pages/stream/stream-input-route-controller'

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

function syncStreamUiInputMode(enabled: boolean, overlayOpen: boolean): void {
  window.dispatchEvent(
    new CustomEvent('stream-ui-input-mode', {
      detail: { enabled, overlayOpen },
    }),
  )
}

function captureReasonForSheet(sheet: Exclude<StreamPageActiveSheet, 'none'>): UiCaptureReason {
  if (sheet === 'menu') {
    return 'menu'
  }
  if (sheet === 'diagnosticsMenu') {
    return 'diagnostics'
  }
  return 'sheet'
}

function releaseReasonForSheet(sheet: Exclude<StreamPageActiveSheet, 'none'>): UiReleaseReason {
  if (sheet === 'menu') {
    return 'menu-close'
  }
  if (sheet === 'diagnosticsMenu') {
    return 'diagnostics-close'
  }
  return 'sheet-close'
}

/**
 * XStream 页面自己的临时 UI 状态，不放到 streaming 域内继续扩散。
 */
export function useXStreamPageUi(options: UseXStreamPageUiOptions) {
  const activeSheet = ref<StreamPageActiveSheet>('none')
  const chromeVisible = ref(true)
  const chromeTimer = ref<BrowserTimeout | null>(null)
  const lastOpenedSheet = ref<Exclude<StreamPageActiveSheet, 'none'> | null>(null)

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
    const previous = activeSheet.value
    const previousCapture = previous !== 'none' && previous !== sheet
      ? captureReasonForSheet(previous)
      : null
    lastOpenedSheet.value = sheet
    activeSheet.value = sheet
    if (previousCapture !== null) {
      streamInputRouteController.replaceUiCapture(previousCapture, captureReasonForSheet(sheet))
      return
    }
    streamInputRouteController.captureUiInput(captureReasonForSheet(sheet))
  }

  function closeSheet(sheet?: Exclude<StreamPageActiveSheet, 'none'>): void {
    if (sheet !== undefined && activeSheet.value !== sheet) {
      return
    }
    const closing = sheet
      ?? (activeSheet.value === 'none' ? lastOpenedSheet.value : activeSheet.value)
    activeSheet.value = 'none'
    lastOpenedSheet.value = null
    if (closing !== null) {
      void streamInputRouteController.releaseUiInputAfterNeutral(releaseReasonForSheet(closing))
    }
  }

  function syncOverlayCapture(
    visible: boolean,
    wasVisible: boolean | undefined,
    captureReason: UiCaptureReason,
    releaseReason: UiReleaseReason,
  ): void {
    if (visible) {
      streamInputRouteController.captureUiInput(captureReason)
      return
    }
    if (wasVisible) {
      void streamInputRouteController.releaseUiInputAfterNeutral(releaseReason)
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

  watch(showFailedSheet, (visible, wasVisible) =>
    syncOverlayCapture(visible, wasVisible, 'failed', 'failed-close'))

  watch(showWarningSheet, (visible, wasVisible) =>
    syncOverlayCapture(visible, wasVisible, 'warning', 'warning-close'))

  onMounted(() => {
    for (const eventName of REVEAL_EVENTS) {
      window.addEventListener(eventName, revealChrome)
    }
  })

  onBeforeUnmount(() => {
    clearChromeTimer()
    syncStreamUiInputMode(true, false)
    for (const eventName of REVEAL_EVENTS) {
      window.removeEventListener(eventName, revealChrome)
    }
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
