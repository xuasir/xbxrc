<script setup lang="ts">
import type { GamepadTool } from '../settings/useGamepadPanelSession'
import { useRouter } from 'vue-router'
import { requestPageFocus } from '@/navigation/core/usePageFocus'
import SettingGamepadEditor from './SettingGamepadEditor.vue'

const props = defineProps<{ tool: GamepadTool | 'live' }>()
const router = useRouter()
function close(): void {
  requestPageFocus(false)
  void router.push({ name: 'controller', query: { tab: props.tool === 'live' ? 'connection' : 'settings' } })
}
</script>

<template>
  <div class="controller-tool-page">
    <SettingGamepadEditor :tool="tool === 'live' ? 'test' : tool" :live="tool === 'live'" @close="close" />
  </div>
</template>

<style scoped>
.controller-tool-page { height: 100%; min-width: 0; min-height: 0; }
</style>
