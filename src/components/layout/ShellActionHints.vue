<script setup lang="ts">
import { useNavigationButtonLabels } from '@shared/gamepad/navigation-hints'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

defineProps<{
  regionLabel?: string
  status?: string
  error?: boolean
  overlay?: boolean
  available?: boolean
  exclusive?: boolean
  busy?: boolean
  back?: boolean
}>()
const emit = defineEmits<{ resize: [height: number] }>()
const { t } = useI18n()
const buttons = useNavigationButtonLabels()
const root = ref<HTMLElement | null>(null)
let resizeObserver: ResizeObserver | undefined

onMounted(() => {
  if (!root.value)
    return
  const updateHeight = () => emit('resize', Math.ceil(root.value?.getBoundingClientRect().height ?? 0))
  updateHeight()
  resizeObserver = new ResizeObserver(updateHeight)
  resizeObserver.observe(root.value)
})
onBeforeUnmount(() => resizeObserver?.disconnect())
</script>

<template>
  <footer ref="root" class="shell-hints" :aria-label="t('shell.hints.label')">
    <div v-if="available && !exclusive && !busy" class="shell-hints__actions">
      <span class="shell-hints__action"><kbd>{{ buttons.south }}</kbd>{{ t('shell.hints.confirm') }}</span>
      <span v-if="overlay || back" class="shell-hints__action"><kbd>{{ buttons.east }}</kbd>{{ t(overlay ? 'shell.hints.close' : 'shell.hints.back') }}</span>
      <template v-if="!overlay">
        <span class="shell-hints__action"><kbd>{{ buttons.l1 }}</kbd><kbd>{{ buttons.r1 }}</kbd>{{ t('shell.hints.pages') }}</span>
        <span v-if="regionLabel" class="shell-hints__action"><kbd>{{ buttons.l2 }}</kbd><kbd>{{ buttons.r2 }}</kbd>{{ regionLabel }}</span>
      </template>
    </div>
    <span v-else class="shell-hints__ownership">
      {{ t(busy ? 'shell.hints.busy' : exclusive ? 'shell.hints.exclusive' : 'shell.hints.inactive') }}
    </span>
    <p class="shell-hints__status" :class="{ 'shell-hints__status--error': error }" :role="error ? 'alert' : 'status'">
      {{ status }}
    </p>
  </footer>
</template>

<style scoped>
.shell-hints { grid-column: 2; grid-row: 3; position: relative; z-index: calc(var(--z-modal) + 1); display: flex; align-items: center; justify-content: space-between; gap: var(--space-16); min-width: 0; min-height: var(--ui-shell-hint-area); padding: var(--space-8) var(--space-8) var(--space-12); color: var(--color-text-secondary); background: transparent; pointer-events: none; font-size: 12px; line-height: 18px; }
.shell-hints__actions { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-16); }
.shell-hints__action { display: inline-flex; align-items: center; gap: var(--space-8); white-space: nowrap; }
.shell-hints kbd { display: inline-flex; align-items: center; justify-content: center; min-width: 24px; min-height: 22px; padding: 1px var(--space-4); border: 1px solid var(--ui-border-hover); border-radius: var(--ui-radius-md); background: var(--ui-keycap-bg); color: var(--ui-icon-text); font: inherit; font-size: 11px; line-height: 16px; box-shadow: inset 0 1px 0 var(--ui-material-edge); }
/* 暂时隐藏重复的视觉提示，继续通过 status / alert 提供辅助技术反馈。 */
.shell-hints__status { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
.shell-hints__status--error { color: var(--color-danger); }
@media(max-width:700px) {
  .shell-hints {
    grid-area: auto;
    position: absolute;
    left: 50%;
    bottom: calc(var(--ui-shell-bottom-nav-height) + var(--space-12));
    transform: translateX(-50%);
    width: max-content;
    max-width: calc(100% - 24px);
    min-height: 0;
    padding: 6px 10px;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-8) var(--space-16);
    border: 1px solid var(--ui-border-subtle);
    border-radius: var(--ui-radius-lg);
    background: color-mix(in srgb, var(--ui-surface-overlay) 90%, transparent);
    backdrop-filter: blur(24px) saturate(140%);
    box-shadow: inset 0 1px 0 var(--ui-material-edge), var(--ui-shadow-low);
  }
  .shell-hints__actions { justify-content: center; gap: var(--space-8) var(--space-16); }
}
@media(max-width:600px) {
  /* 隐藏悬浮面板并释放布局空间，状态提示继续供辅助技术读取。 */
  .shell-hints { display: contents; }
  .shell-hints__actions, .shell-hints__ownership { display: none; }
}
</style>
