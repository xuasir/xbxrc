<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

const props = defineProps<{
  id: string
  scopeId: string
  label: string
  description?: string
  value?: string
  effectLabel?: string
  needsRestart?: boolean
  disabled?: boolean
  busy?: boolean
  select?: boolean
}>()
const emit = defineEmits<{ click: [] }>()
</script>

<template>
  <Focusable
    :id="props.id" as="button" type="button" class="setting-row"
    :scope-id="props.scopeId" :disabled="props.disabled" :busy="props.busy"
    :aria-labelledby="`${props.id}.label ${props.id}.value`"
    :aria-describedby="props.description ? `${props.id}.description` : undefined"
    @click="emit('click')"
  >
    <span class="setting-row__copy">
      <span :id="`${props.id}.label`" class="setting-row__label">{{ props.label }}</span>
      <span v-if="props.description" :id="`${props.id}.description`" class="setting-row__desc">{{ props.description }}</span>
      <span v-if="props.effectLabel" class="setting-row__effect">{{ props.effectLabel }}</span>
    </span>
    <span :id="`${props.id}.value`" class="setting-row__value"><slot>{{ props.value }}</slot><span v-if="props.select" class="setting-row__chevron" aria-hidden="true">›</span></span>
  </Focusable>
</template>

<style scoped src="./setting-row.css"></style>
