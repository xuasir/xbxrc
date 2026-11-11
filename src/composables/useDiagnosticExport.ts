import type { DiagnosticSaveParams } from '@shared/rpc/runtimeTrace'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { diagnosticExportBusy, saveDiagnosticBundle } from '../services/diagnostics'

export function useDiagnosticExport() {
  const { t } = useI18n()
  const feedback = ref('')
  const failed = ref(false)
  async function save(params: DiagnosticSaveParams = {}) {
    if (diagnosticExportBusy.value)
      return false
    feedback.value = ''
    failed.value = false
    try {
      const result = await saveDiagnosticBundle(params)
      feedback.value = t(result.canceled ? 'diagnostics.saveCanceled' : 'diagnostics.saved')
      return result.saved
    }
    catch {
      failed.value = true
      feedback.value = t('diagnostics.saveFailed')
      return false
    }
  }
  return { busy: diagnosticExportBusy, feedback, failed, save }
}
