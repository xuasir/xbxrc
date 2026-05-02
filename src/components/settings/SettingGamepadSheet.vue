<script setup lang="ts">
import SettingGamepadSection from '../../pages/settings/SettingGamepadSection.vue'
import SettingModalShell from './SettingModalShell.vue'

interface Props {
  open: boolean
  scopeId: string
  navNodeBaseId: string
}

const props = defineProps<Props>()

const emit = defineEmits<{
  (event: 'close'): void
}>()

function handleClose(): void {
  emit('close')
}
</script>

<template>
  <SettingModalShell
    :open="props.open"
    :scope-id="props.scopeId"
    title="手柄设置"
    hint="使用手柄在 UI 中导航，或自定义按钮映射。"
    width="min(100%, 860px)"
    max-height="min(90vh, 860px)"
    @close="handleClose"
  >
    <div class="setting-gamepad-sheet__body">
      <SettingGamepadSection
        :scope-id="props.scopeId"
        :nav-node-base-id="props.navNodeBaseId"
        embedded
      />
    </div>
  </SettingModalShell>
</template>

<style scoped>
.setting-gamepad-sheet__body {
  overflow: auto;
  overscroll-behavior: contain;
  padding-right: 6px;
  margin-right: -6px;
}
</style>
