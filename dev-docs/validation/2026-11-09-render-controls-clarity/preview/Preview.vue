<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import SettingDisplayOptionsSheet from '@/components/settings/SettingDisplayOptionsSheet.vue'
import StreamExperiencePanel from '@/components/stream/StreamExperiencePanel.vue'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { useSessionRenderControls } from '@/streaming/render-controls'
import { renderControlsLabel } from '@/shared/config/render-controls'
const { t } = useI18n()
const params = new URLSearchParams(location.search)
const scope = ref(params.get('scope') === 'default' ? 'default' : 'session')
const open = ref(params.get('open') === '1')
const visible = ref(true)
const controls = useSessionRenderControls()
controls.initialize({ render_mode: params.get('mode') ?? 'video', display_options: { sharpness: params.get('sharpness') === 'soft' ? 1 : 3, brightness: 100, contrast: 100, saturation: 100 } })
const { current: currentValue, active: activeValue, source } = controls
const fallback = ref(params.get('fallback') ?? '')
const actual = computed(() => ({ mode: fallback.value ? 'video' : activeValue.value.mode, sharpening: activeValue.value.displayOptions.sharpness <= 2 ? 'soft' : 'clear', pending: false, fallbackReason: fallback.value || undefined }))
const model = { observationState: 'fresh', status: '稳定', resolution: '2560 × 1440', rtt: '16 ms', jit: '1 ms', recvFps: '60', decodeFps: '60', presentFps: '59.9', packetLoss: '0.0%', videoBitrate: '20.0 Mbps', totalBitrate: '20.1 Mbps', relayNotice: false, recoveringNotice: false, displaySupplyNotice: false, noVideoNotice: false }
function close() { controls.clearTransient(); open.value = false }
function submit(value, saveDefault) { saveDefault ? controls.acceptSavedDefault(value) : controls.apply(value); open.value = false }
function restore() { controls.restoreDefaults(); open.value = false }
function toggleTheme() { document.documentElement.dataset.theme = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark' }
</script>
<template>
  <main class="preview-stream">
    <StreamExperiencePanel :visible="visible" :mount="{ phase: 'mounted' }" :model="model" :render-state="actual" />
    <p class="preview-center">串流画面</p>
    <FocusScope id="preview.page" :active="!open" :trap="false">
      <div class="preview-controls">
        <Focusable id="preview.open" as="button" @click="open = true">画面设置</Focusable>
        <Focusable id="preview.metrics" as="button" @click="visible = !visible">指标栏开关</Focusable>
        <Focusable id="preview.fallback" as="button" @click="fallback = fallback ? '' : 'performance'">模拟回退</Focusable>
        <Focusable id="preview.theme" as="button" @click="toggleTheme">切换主题</Focusable>
        <p>已应用：{{ renderControlsLabel(currentValue, t) }} · {{ source === 'default' ? '全局默认' : '本次调整' }}</p>
      </div>
    </FocusScope>
    <SettingDisplayOptionsSheet :open="open" scope-id="preview.render" :title="t('setting.renderControls.title')" :hint="t(scope === 'session' ? 'setting.renderControls.sessionHint' : 'setting.renderControls.defaultHint')" :current-value="currentValue" :configuration-scope="scope" :value-source="source" @change="controls.preview" @close="close" @submit="submit" @restore-defaults="restore" />
  </main>
</template>
<style scoped>
.preview-stream { position: relative; width: 100%; height: 100%; background: radial-gradient(ellipse at 65% 30%, #253847, transparent 60%), linear-gradient(135deg, #171a1c, #090d10); color: white; }
.preview-center { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); opacity: .35; font-size: 24px; }
.preview-controls { position: absolute; bottom: 24px; left: 24px; display: flex; flex-wrap: wrap; gap: 16px; max-width: calc(100% - 48px); }
.preview-controls button { padding: 10px 16px; border: 0; border-radius: 8px; background: #27343d; color: white; }
.preview-controls p { width: 100%; }
</style>
