<script setup lang="ts">
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable } from '@/navigation/core/vue'

const props = defineProps<{ src: string, busy?: boolean }>()
const emit = defineEmits<{ error: [], operationError: [message: string] }>()
type WebkitDocument = Document & { webkitFullscreenElement?: Element | null, webkitExitFullscreen?: () => Promise<void> | void }
type WebkitElement = HTMLElement & { webkitRequestFullscreen?: () => Promise<void> | void }
const scopeId = 'album.preview.video'
const owner = Symbol(scopeId)
const container = ref<HTMLElement | null>(null)
const video = ref<HTMLVideoElement | null>(null)
const playing = ref(false)
const playBusy = ref(false)
const currentTime = ref(0)
const duration = ref(0)
const volume = ref(1)
const muted = ref(false)
const fullscreenMode = ref<'inline' | 'expanded' | 'native'>('inline')
const fullscreenBusy = ref(false)
const expanded = computed(() => fullscreenMode.value !== 'inline')
const canSeek = computed(() => duration.value > 0)
let active = true
let playRevision = 0
let fullscreenRevision = 0
let scopeRevision = 0
const fullscreenDocument = () => document as WebkitDocument
const nativeContainer = () => document.fullscreenElement ?? fullscreenDocument().webkitFullscreenElement
function time(value: number) {
  const seconds = Math.floor(Math.max(0, value))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
function sync(event?: Event) {
  const element = video.value
  if (!element || (event && event.currentTarget !== element))
    return
  playing.value = !element.paused && !element.ended
  currentTime.value = Number.isFinite(element.currentTime) ? Math.max(0, element.currentTime) : 0
  duration.value = Number.isFinite(element.duration) ? Math.max(0, element.duration) : 0
  volume.value = element.volume
  muted.value = element.muted
}
function mediaError(event: Event) {
  if (event.currentTarget === video.value)
    emit('error')
}
async function togglePlayback() {
  const element = video.value
  if (!element || props.busy || playBusy.value)
    return
  if (!element.paused) {
    element.pause()
    sync()
    return
  }
  const request = ++playRevision
  playBusy.value = true
  try {
    await element.play()
    if (active && request === playRevision && element === video.value)
      sync()
    else element.pause()
  }
  catch {
    if (active && request === playRevision && element === video.value)
      emit('operationError', '录像暂时无法播放，请刷新资源后重试')
  }
  finally {
    if (active && request === playRevision)
      playBusy.value = false
  }
}
function seek(delta: number) {
  const element = video.value
  if (!element || props.busy || !canSeek.value)
    return
  try {
    element.currentTime = Math.min(duration.value, Math.max(0, currentTime.value + delta))
    sync()
  }
  catch { emit('operationError', '录像进度暂时无法调整，请刷新资源后重试') }
}
function adjustVolume(delta: number) {
  const element = video.value
  if (!element || props.busy)
    return
  element.volume = Math.round(Math.min(1, Math.max(0, element.volume + delta)) * 100) / 100
  if (delta > 0)
    element.muted = false
  sync()
}
function toggleMute() {
  if (video.value && !props.busy) {
    video.value.muted = !video.value.muted
    sync()
  }
}
async function exitNative(element: HTMLElement | null) {
  if (element && nativeContainer() === element) {
    if (document.exitFullscreen)
      await document.exitFullscreen()
    else await fullscreenDocument().webkitExitFullscreen?.()
  }
}
async function restoreFullscreenFocus() {
  const request = fullscreenRevision
  await nextTick()
  requestAnimationFrame(() => {
    const source = document.getElementById('album.preview.fullscreen')
    // 浏览器退出全屏后会再次恢复原生焦点；下一帧只同步当前来源，保留用户的新位置。
    if (active && request === fullscreenRevision && !expanded.value && source && navigationEngine.getCurrentFocus() === source)
      navigationEngine.focusElement(source, false, true)
  })
}
function fullscreenChanged() {
  if (active && container.value && nativeContainer() === container.value) {
    fullscreenMode.value = 'native'
  }
  else if (fullscreenMode.value === 'native') {
    fullscreenMode.value = 'inline'
    void restoreFullscreenFocus()
  }
}
async function leaveFullscreen() {
  const element = container.value
  const request = ++fullscreenRevision
  fullscreenBusy.value = true
  try {
    await exitNative(element)
    if (active && request === fullscreenRevision) {
      fullscreenMode.value = 'inline'
      void restoreFullscreenFocus()
    }
  }
  catch {
    if (active && request === fullscreenRevision)
      emit('operationError', '全屏状态暂时无法切换，请重试')
  }
  finally {
    if (active && request === fullscreenRevision)
      fullscreenBusy.value = false
  }
}
async function toggleFullscreen() {
  const element = container.value as WebkitElement | null
  if (!element)
    return
  if (expanded.value) {
    await leaveFullscreen()
    return
  }
  if (props.busy || fullscreenBusy.value)
    return
  const request = ++fullscreenRevision
  fullscreenBusy.value = true
  try {
    // 手柄 A 的合成事件缺少浏览器用户激活，窗口内全屏继续保留操作区与 B 返回。
    const canRequestNative = navigator.userActivation?.isActive !== false && (document.fullscreenEnabled || element.webkitRequestFullscreen)
    if (canRequestNative) {
      try {
        if (element.requestFullscreen)
          await element.requestFullscreen()
        else await element.webkitRequestFullscreen?.()
      }
      catch { /* 用户激活或 WebView 能力限制时，采用同一播放器的窗口内全屏。 */ }
    }
    if (active && request === fullscreenRevision)
      fullscreenMode.value = nativeContainer() === element ? 'native' : 'expanded'
    else await exitNative(element)
  }
  catch {
    if (active && request === fullscreenRevision)
      emit('operationError', '全屏状态暂时无法切换，请重试')
  }
  finally {
    if (active && request === fullscreenRevision)
      fullscreenBusy.value = false
  }
}
function back(event: Event) {
  if (expanded.value) {
    event.preventDefault()
    event.stopPropagation()
    void leaveFullscreen()
  }
}
function escape(event: KeyboardEvent) {
  if (event.key === 'Escape' && !event.defaultPrevented && navigationEngine.isTopmostScope(scopeId))
    back(event)
}
watch(expanded, async (open) => {
  const request = ++scopeRevision
  if (!open) {
    navigationEngine.updateActiveScope(scopeId, false, { owner })
    return
  }
  await nextTick()
  if (active && request === scopeRevision && expanded.value)
    navigationEngine.updateActiveScope(scopeId, true, { owner, element: container.value })
})
watch(() => props.src, () => {
  playRevision++
  video.value?.pause()
  playBusy.value = playing.value = false
  currentTime.value = duration.value = 0
})
function release() {
  active = false
  playRevision++
  fullscreenRevision++
  scopeRevision++
  video.value?.pause()
  playBusy.value = fullscreenBusy.value = playing.value = false
  fullscreenMode.value = 'inline'
  navigationEngine.updateActiveScope(scopeId, false, { owner })
  void exitNative(container.value).catch(() => {})
}
onActivated(() => active = true)
onDeactivated(release)
onMounted(() => {
  document.addEventListener('fullscreenchange', fullscreenChanged)
  document.addEventListener('webkitfullscreenchange', fullscreenChanged)
  window.addEventListener('keydown', escape)
})
onBeforeUnmount(() => {
  release()
  document.removeEventListener('fullscreenchange', fullscreenChanged)
  document.removeEventListener('webkitfullscreenchange', fullscreenChanged)
  window.removeEventListener('keydown', escape)
})
</script>

<template>
  <div :id="scopeId" ref="container" class="album-video" :class="{ 'album-video--expanded': expanded }" :data-nav-zone="scopeId" @nav-back="back">
    <!-- 所有播放操作由下方的几何导航控件统一提供，鼠标和手柄共用同一操作区。 -->
    <video :key="props.src" ref="video" class="album-video__media" :src="props.src" playsinline preload="metadata" @loadedmetadata="sync" @durationchange="sync" @timeupdate="sync" @volumechange="sync" @play="sync" @pause="sync" @ended="sync" @error="mediaError" />
    <div class="album-video__status library-caption">
      <span>进度 {{ time(currentTime) }} / {{ time(duration) }}</span><span>音量 {{ muted ? '静音' : `${Math.round(volume * 100)}%` }}</span>
    </div>
    <div class="library-actions album-video__actions" aria-label="录像操作">
      <Focusable id="album.preview.play" as="button" class="library-control" :busy="props.busy || playBusy" @click="togglePlayback">
        {{ playing ? '暂停录像' : '播放录像' }}
      </Focusable>
      <Focusable id="album.preview.rewind" as="button" class="library-control" :disabled="!canSeek" :busy="props.busy" @click="seek(-10)">
        后退 10 秒
      </Focusable>
      <Focusable id="album.preview.forward" as="button" class="library-control" :disabled="!canSeek" :busy="props.busy" @click="seek(10)">
        前进 10 秒
      </Focusable>
      <Focusable id="album.preview.volume-down" as="button" class="library-control" :busy="props.busy" @click="adjustVolume(-0.1)">
        音量 −
      </Focusable>
      <Focusable id="album.preview.volume-up" as="button" class="library-control" :busy="props.busy" @click="adjustVolume(0.1)">
        音量 +
      </Focusable>
      <Focusable id="album.preview.mute" as="button" class="library-control" :busy="props.busy" :aria-pressed="muted" @click="toggleMute">
        {{ muted ? '取消静音' : '静音' }}
      </Focusable>
      <Focusable id="album.preview.fullscreen" as="button" class="library-control" :busy="(props.busy && !expanded) || fullscreenBusy" :aria-pressed="expanded" @click="toggleFullscreen">
        {{ expanded ? '退出全屏' : '全屏查看' }}
      </Focusable>
    </div>
  </div>
</template>

<style scoped>
.album-video { display: flex; flex-direction: column; gap: var(--ui-space-md); }
.album-video__media { display: block; width: 100%; max-height: 60dvh; min-height: 0; object-fit: contain; border-radius: var(--ui-radius-md); }
.album-video__status { display: flex; justify-content: space-between; gap: var(--ui-space-md); flex-wrap: wrap; font-variant-numeric: tabular-nums; }
.album-video--expanded, .album-video:fullscreen { position: fixed; inset: 0; z-index: calc(var(--z-modal) + 1); padding: var(--ui-space-lg); background: var(--ui-surface-overlay); overflow: auto; }
.album-video--expanded .album-video__media, .album-video:fullscreen .album-video__media { flex: 1 1 0; max-height: none; }
.album-video__actions { flex-shrink: 0; }
</style>
