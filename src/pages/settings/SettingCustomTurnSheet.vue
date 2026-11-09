<script setup lang="ts">
import type { CustomTurnDraft } from './setting-validation'
import { useNavigationButtonLabels } from '@shared/gamepad/navigation-hints'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingConfirmSheet from '../../components/settings/SettingConfirmSheet.vue'
import SettingModalShell from '../../components/settings/SettingModalShell.vue'
import SettingTextField from '../../components/settings/SettingTextField.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'
import { rpc } from '../../services/rpc'
import { validateCustomTurn } from './setting-validation'

const props = defineProps<{ open: boolean, config: CustomTurnDraft }>()
const emit = defineEmits<{ close: [], saved: [] }>()
const { t } = useI18n()
const buttons = useNavigationButtonLabels()
const scope = 'setting.custom-turn'
const draft = ref<CustomTurnDraft>({ ...props.config })
const enabled = ref(false)
const saving = ref(false)
const error = ref('')
const confirming = ref<'discard' | 'reset' | null>(null)
const storedEnabled = computed(() => !!props.config.server_url && !!props.config.server_username && !!props.config.server_credential)
const dirty = computed(() => enabled.value !== storedEnabled.value || JSON.stringify(draft.value) !== JSON.stringify(props.config))
const validation = computed(() => enabled.value ? validateCustomTurn(draft.value) : null)
watch(() => props.open, (open) => {
  if (open) {
    draft.value = { ...props.config }
    enabled.value = storedEnabled.value
    error.value = ''
    confirming.value = null
  }
})
function close() {
  if (saving.value)
    return
  if (dirty.value)
    confirming.value = 'discard'
  else emit('close')
}
async function save(reset = false) {
  if (saving.value || (!reset && validation.value))
    return
  saving.value = true
  error.value = ''
  try {
    const patch = enabled.value && !reset
      ? { server_url: draft.value.server_url.trim(), server_username: draft.value.server_username.trim(), server_credential: draft.value.server_credential.trim() }
      : { server_url: '', server_username: '', server_credential: '' }
    await rpc.config.set({ patch })
    confirming.value = null
    emit('saved')
    emit('close')
  }
  catch {
    error.value = t('setting.experience.saveFailed')
    confirming.value = null
  }
  finally { saving.value = false }
}
function confirm() {
  if (confirming.value === 'reset') {
    void save(true)
  }
  else {
    confirming.value = null
    emit('close')
  }
}
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="scope" :default-focus-id="`${scope}.enabled`" :title="t('setting.experience.turn.title')" :hint="t('setting.experience.turn.hint')" :busy="saving" @close="close">
    <div class="setting-turn-form">
      <SettingToggleRow :id="`${scope}.enabled`" :scope-id="scope" :label="t('setting.experience.turn.enable')" :enabled="enabled" :busy="saving" @confirm="enabled = !enabled" />
      <template v-if="enabled">
        <p class="setting-panel__status">
          {{ t('setting.experience.input.controllerHint', { confirm: buttons.south }) }}
        </p>
        <SettingTextField :id="`${scope}.url`" v-model="draft.server_url" :scope-id="scope" :label="t('setting.experience.turn.url')" placeholder="turn:relay.example.com:3478" :busy="saving" />
        <SettingTextField :id="`${scope}.username`" v-model="draft.server_username" :scope-id="scope" :label="t('setting.experience.turn.username')" :busy="saving" />
        <SettingTextField :id="`${scope}.credential`" v-model="draft.server_credential" :scope-id="scope" :label="t('setting.experience.turn.credential')" :busy="saving" sensitive />
      </template>
      <p class="setting-panel__status">
        {{ t(enabled ? 'setting.experience.effects.nextConnection' : 'setting.experience.turn.default') }}
      </p>
      <p v-if="validation" class="setting-panel__status setting-panel__status--error" role="status">
        {{ t(`setting.experience.turn.${validation === 'url' ? 'urlError' : validation}`) }}
      </p>
      <p v-if="error" class="setting-panel__status setting-panel__status--error" role="alert">
        {{ error }}
      </p>
      <Focusable v-if="props.config.server_url || props.config.server_username || props.config.server_credential" :id="`${scope}.reset`" as="button" :scope-id="scope" class="setting-panel__action" :busy="saving" @click="confirming = 'reset'">
        {{ t('setting.experience.turn.reset') }}
      </Focusable>
    </div>
    <template #footer>
      <div class="setting-panel__actions">
        <Focusable :id="`${scope}.cancel`" as="button" :scope-id="scope" class="setting-panel__action" :busy="saving" @click="close">
          {{ t('setting.editor.cancel') }}
        </Focusable>
        <Focusable :id="`${scope}.apply`" as="button" :scope-id="scope" class="setting-panel__action setting-panel__action--primary" :busy="saving" :disabled="!!validation || !dirty" @click="save()">
          {{ t(saving ? 'setting.experience.saving' : 'gamepadSettings.apply') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
  <SettingConfirmSheet :open="!!confirming" scope-id="setting.custom-turn.confirm" :title="t(confirming === 'reset' ? 'setting.experience.turn.reset' : 'setting.experience.discardTitle')" :message="t(confirming === 'reset' ? 'setting.experience.turn.resetHint' : 'setting.experience.discardHint')" :busy="saving" @close="confirming = null" @confirm="confirm" />
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.setting-turn-form { display: flex; flex-direction: column; gap: var(--ui-settings-form-gap); }
</style>
