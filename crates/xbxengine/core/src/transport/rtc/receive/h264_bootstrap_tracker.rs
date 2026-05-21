use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
    H264InspectionError,
};

/// H264 参数集缓存与 IDR bootstrap（receiver-local）。
#[derive(Default)]
pub struct H264BootstrapTracker {
    inspector: H264AccessUnitInspector,
}

impl H264BootstrapTracker {
    pub fn inspect_access_unit(
        &mut self,
        payload: &[u8],
    ) -> Result<H264AccessUnitInspection, H264InspectionError> {
        self.inspector.inspect_access_unit(payload)
    }

    pub fn committed_sps_present(&self) -> bool {
        self.inspector.committed_sps_present()
    }

    pub fn committed_pps_present(&self) -> bool {
        self.inspector.committed_pps_present()
    }

    pub fn seed_committed_parameter_sets_if_absent(
        &mut self,
        sps: &[u8],
        pps: &[u8],
    ) -> Result<bool, H264InspectionError> {
        self.inspector
            .seed_committed_parameter_sets_if_absent(sps, pps)
    }

    pub fn try_prepend_committed_parameter_sets(
        &self,
        inspection: &H264AccessUnitInspection,
        payload: &[u8],
    ) -> Option<Vec<u8>> {
        if inspection.bootstrap_ready {
            return None;
        }
        let reject = inspection.bootstrap_reject_reason.as_ref()?;
        if !matches!(
            reject,
            H264BootstrapRejectReason::MissingSps | H264BootstrapRejectReason::MissingPps
        ) {
            return None;
        }
        if !inspection.is_idr || !inspection.slice_headers_valid {
            return None;
        }
        if inspection.parameter_sets_changed || inspection.config_changed {
            return None;
        }
        if !inspection.committed_sps_present() || !inspection.committed_pps_present() {
            return None;
        }
        let mut prefix = self.inspector.committed_parameter_set_annex_b_prefix()?;
        let mut out = Vec::with_capacity(prefix.len() + payload.len());
        out.append(&mut prefix);
        out.extend_from_slice(payload);
        Some(out)
    }
}
