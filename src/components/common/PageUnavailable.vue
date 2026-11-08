<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

const props = withDefaults(defineProps<{ id: string, title: string, body: string, actionLabel: string, busy?: boolean, error?: boolean }>(), { busy: false, error: false })
const emit = defineEmits<{ retry: [] }>()

function retry(): void {
  // 请求期间保留焦点入口，并合并重复确认。
  if (!props.busy)
    emit('retry')
}
</script>

<template>
  <section class="page-unavailable" :aria-busy="busy">
    <h1 class="ui-page-title">
      {{ title }}
    </h1>
    <p class="ui-page-body" :role="error ? 'alert' : 'status'">
      {{ body }}
    </p>
    <Focusable :id="id" as="button" type="button" scope-id="app.shell" class="page-unavailable__action ui-action-button ui-action-button--brand" :aria-busy="busy" @click="retry">
      {{ actionLabel }}
    </Focusable>
  </section>
</template>

<style scoped>
.page-unavailable { display: flex; flex: 1 1 auto; flex-direction: column; justify-content: center; align-items: center; gap: 18px; width: min(100%, var(--ui-login-content-width)); min-height: var(--ui-xcloud-state-min-height); margin: auto; text-align: center; }
.page-unavailable .ui-page-title, .page-unavailable .ui-page-body { width: min(100%, var(--ui-login-desc-width)); overflow-wrap: anywhere; }
.page-unavailable__action { min-width: min(100%, var(--ui-login-signin-width)); min-height: var(--ui-xcloud-action-min-height); margin: 0 auto; cursor: pointer; }
.page-unavailable__action[data-focused='true'] { transform: none; box-shadow: var(--shadow-xbox-focus); }
.page-unavailable__action[aria-busy='true'] { opacity: .62; cursor: progress; }
</style>
