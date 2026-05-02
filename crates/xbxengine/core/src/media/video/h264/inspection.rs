use std::ops::Range;
use std::sync::{Arc, Mutex};

use h264_reader::{
    nal::{
        pps::PicParameterSet, slice::SliceHeader, sps::SeqParameterSet, Nal, NalHeader,
        NalHeaderError, UnitType,
    },
    Context,
};

#[derive(Clone, Debug)]
pub struct H264SeqParameterSet {
    #[allow(dead_code)]
    pub raw: Vec<u8>,
    pub parsed: SeqParameterSet,
}

#[derive(Clone, Debug)]
pub struct H264PicParameterSet {
    #[allow(dead_code)]
    pub raw: Vec<u8>,
    pub parsed: PicParameterSet,
}

#[derive(Clone, Debug)]
pub struct H264ParameterSets {
    pub sps: H264SeqParameterSet,
    pub pps: H264PicParameterSet,
}

#[derive(Clone, Debug)]
pub struct H264NalUnit {
    #[allow(dead_code)]
    pub range: Range<usize>,
    pub unit_type: UnitType,
}

#[derive(Clone, Debug)]
pub struct H264AccessUnitInspection {
    pub nals: Vec<H264NalUnit>,
    pub parameter_sets: Option<H264ParameterSets>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_idr: bool,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub slice_headers_valid: bool,
    pub parameter_sets_changed: bool,
    pub config_changed: bool,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<H264BootstrapRejectReason>,
    pub(crate) commit_state: Arc<Mutex<H264AccessUnitInspectorState>>,
}

impl H264AccessUnitInspection {
    pub fn commit(&self) {
        let mut state = self
            .commit_state
            .lock()
            .expect("h264 commit state poisoned");
        if let Some(parameter_sets) = self.parameter_sets.as_ref() {
            let committed_dimensions = state.committed_dimensions;
            state.committed_dimensions = self.width.zip(self.height).or(committed_dimensions);
            state.committed_sps = Some(parameter_sets.sps.clone());
            state.committed_pps = Some(parameter_sets.pps.clone());
            return;
        }

        if let Some(width) = self.width {
            if let Some(height) = self.height {
                state.committed_dimensions = Some((width, height));
            }
        }
    }

    #[allow(dead_code)]
    pub fn build_avcc_payload(&self, payload: &[u8]) -> Vec<u8> {
        let mut avcc_payload = Vec::with_capacity(payload.len() + self.nals.len() * 4);
        for nal in &self.nals {
            if matches!(
                nal.unit_type,
                UnitType::SeqParameterSet
                    | UnitType::PicParameterSet
                    | UnitType::AccessUnitDelimiter
            ) {
                continue;
            }
            let nal_bytes = &payload[nal.range.clone()];
            if nal_bytes.is_empty() {
                continue;
            }
            let len = nal_bytes.len() as u32;
            avcc_payload.extend_from_slice(&len.to_be_bytes());
            avcc_payload.extend_from_slice(nal_bytes);
        }
        avcc_payload
    }

    #[allow(dead_code)]
    pub fn effective_parameter_sets(&self) -> Option<H264ParameterSets> {
        if let Some(parameter_sets) = self.parameter_sets.clone() {
            return Some(parameter_sets);
        }
        let state = self
            .commit_state
            .lock()
            .expect("h264 commit state poisoned");
        state
            .committed_sps
            .clone()
            .zip(state.committed_pps.clone())
            .map(|(sps, pps)| H264ParameterSets { sps, pps })
    }

    /// 为硬解 backend 构造 AVCC access unit。
    /// 当关键帧/配置变化时，把当前生效的 SPS/PPS 一并前置，避免硬解器只拿到裸 VCL。
    #[allow(dead_code)]
    pub fn build_decoder_avcc_payload(
        &self,
        payload: &[u8],
        prepend_parameter_sets: bool,
    ) -> Vec<u8> {
        let mut avcc_payload = Vec::with_capacity(payload.len() + self.nals.len() * 4 + 128);
        if prepend_parameter_sets {
            if let Some(parameter_sets) = self.effective_parameter_sets() {
                push_avcc_nal(&mut avcc_payload, &parameter_sets.sps.raw);
                push_avcc_nal(&mut avcc_payload, &parameter_sets.pps.raw);
            }
        }
        for nal in &self.nals {
            if nal.range.is_empty() {
                continue;
            }
            if nal.unit_type == UnitType::AccessUnitDelimiter {
                continue;
            }
            if prepend_parameter_sets
                && matches!(
                    nal.unit_type,
                    UnitType::SeqParameterSet | UnitType::PicParameterSet
                )
            {
                continue;
            }
            push_avcc_nal(&mut avcc_payload, &payload[nal.range.clone()]);
        }
        avcc_payload
    }

    #[allow(dead_code)]
    pub fn bootstrap_parameter_sets(&self) -> Option<&H264ParameterSets> {
        self.parameter_sets.as_ref()
    }

    pub fn committed_sps_present(&self) -> bool {
        self.commit_state
            .lock()
            .expect("h264 commit state poisoned")
            .committed_sps
            .is_some()
    }

    pub fn committed_pps_present(&self) -> bool {
        self.commit_state
            .lock()
            .expect("h264 commit state poisoned")
            .committed_pps
            .is_some()
    }

    pub fn delta_continuation_ready(&self) -> bool {
        let state = self
            .commit_state
            .lock()
            .expect("h264 commit state poisoned");
        self.slice_headers_valid
            && !self.is_idr
            && self.nals.iter().any(|nal| is_vcl_unit(nal.unit_type))
            && state.committed_sps.is_some()
            && state.committed_pps.is_some()
    }

    pub fn nal_type_labels(&self) -> Vec<String> {
        self.nals
            .iter()
            .map(|nal| format!("{:?}", nal.unit_type))
            .collect()
    }
}

impl H264SeqParameterSet {
    /// 比较两份 SPS 是否会改变当前可继续承接的解码上下文。
    ///
    /// `seq_parameter_set_id` 也必须纳入比较。即便语义字段一致，只要 id 变了，
    /// 后续 slice 对参数集的引用关系就已经变化，不能继续沿用旧的 bootstrap/session。
    pub(crate) fn same_decoder_configuration(&self, other: &Self) -> bool {
        self.parsed.seq_parameter_set_id == other.parsed.seq_parameter_set_id
            && self.parsed.profile_idc == other.parsed.profile_idc
            && self.parsed.constraint_flags == other.parsed.constraint_flags
            && self.parsed.level_idc == other.parsed.level_idc
            && self.parsed.chroma_info == other.parsed.chroma_info
            && self.parsed.log2_max_frame_num_minus4 == other.parsed.log2_max_frame_num_minus4
            && self.parsed.pic_order_cnt == other.parsed.pic_order_cnt
            && self.parsed.max_num_ref_frames == other.parsed.max_num_ref_frames
            && self.parsed.gaps_in_frame_num_value_allowed_flag
                == other.parsed.gaps_in_frame_num_value_allowed_flag
            && self.parsed.pic_width_in_mbs_minus1 == other.parsed.pic_width_in_mbs_minus1
            && self.parsed.pic_height_in_map_units_minus1
                == other.parsed.pic_height_in_map_units_minus1
            && self.parsed.frame_mbs_flags == other.parsed.frame_mbs_flags
            && self.parsed.direct_8x8_inference_flag == other.parsed.direct_8x8_inference_flag
            && self.parsed.frame_cropping == other.parsed.frame_cropping
            && self.parsed.vui_parameters == other.parsed.vui_parameters
    }
}

#[allow(dead_code)]
fn push_avcc_nal(out: &mut Vec<u8>, nal_bytes: &[u8]) {
    if nal_bytes.is_empty() {
        return;
    }
    let len = nal_bytes.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(nal_bytes);
}

impl H264PicParameterSet {
    /// 比较两份 PPS 是否会改变当前可继续承接的解码上下文。
    ///
    /// `pic_parameter_set_id` / `seq_parameter_set_id` 同样必须参与比较，
    /// 否则会把“参数集引用图已切换”误判成“同一 preset”。
    #[allow(dead_code)]
    pub(crate) fn same_decoder_configuration(&self, other: &Self) -> bool {
        let slice_groups_equal = match (&self.parsed.slice_groups, &other.parsed.slice_groups) {
            (None, None) => true,
            (Some(left), Some(right)) => format!("{left:?}") == format!("{right:?}"),
            _ => false,
        };
        let extension_equal = match (&self.parsed.extension, &other.parsed.extension) {
            (None, None) => true,
            (Some(left), Some(right)) => format!("{left:?}") == format!("{right:?}"),
            _ => false,
        };

        self.parsed.pic_parameter_set_id == other.parsed.pic_parameter_set_id
            && self.parsed.seq_parameter_set_id == other.parsed.seq_parameter_set_id
            && self.parsed.entropy_coding_mode_flag == other.parsed.entropy_coding_mode_flag
            && self.parsed.bottom_field_pic_order_in_frame_present_flag
                == other.parsed.bottom_field_pic_order_in_frame_present_flag
            && slice_groups_equal
            && self.parsed.num_ref_idx_l0_default_active_minus1
                == other.parsed.num_ref_idx_l0_default_active_minus1
            && self.parsed.num_ref_idx_l1_default_active_minus1
                == other.parsed.num_ref_idx_l1_default_active_minus1
            && self.parsed.weighted_pred_flag == other.parsed.weighted_pred_flag
            && self.parsed.weighted_bipred_idc == other.parsed.weighted_bipred_idc
            && self.parsed.pic_init_qp_minus26 == other.parsed.pic_init_qp_minus26
            && self.parsed.pic_init_qs_minus26 == other.parsed.pic_init_qs_minus26
            && self.parsed.chroma_qp_index_offset == other.parsed.chroma_qp_index_offset
            && self.parsed.deblocking_filter_control_present_flag
                == other.parsed.deblocking_filter_control_present_flag
            && self.parsed.constrained_intra_pred_flag == other.parsed.constrained_intra_pred_flag
            && self.parsed.redundant_pic_cnt_present_flag
                == other.parsed.redundant_pic_cnt_present_flag
            && extension_equal
    }
}

impl H264ParameterSets {
    /// 比较一组参数集是否会改变当前解码配置。
    #[allow(dead_code)]
    pub(crate) fn same_decoder_configuration(&self, other: &Self) -> bool {
        self.sps.same_decoder_configuration(&other.sps)
            && self.pps.same_decoder_configuration(&other.pps)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H264BootstrapRejectReason {
    NoVcl,
    MissingSps,
    MissingPps,
    BootstrapMissingIdr,
    MixedIdrWithTrailingDelta,
    #[allow(dead_code)] // 兼容旧测试夹具；inspector 不再产出该语义。
    NonIdrVcl,
    InvalidSliceHeader,
}

impl H264BootstrapRejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoVcl => "NoVcl",
            Self::MissingSps => "bootstrapMissingSps",
            Self::MissingPps => "bootstrapMissingPps",
            Self::BootstrapMissingIdr => "bootstrapMissingIdr",
            Self::MixedIdrWithTrailingDelta => "mixedIdrWithTrailingDelta",
            Self::NonIdrVcl => "NonIdrVcl",
            Self::InvalidSliceHeader => "InvalidSliceHeader",
        }
    }
}

#[derive(Debug)]
pub enum H264InspectionError {
    EmptyAccessUnit,
    NalHeader { index: usize, message: String },
    Sps { index: usize, message: String },
    Pps { index: usize, message: String },
}

impl std::fmt::Display for H264InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            H264InspectionError::EmptyAccessUnit => f.write_str("empty H264 access unit"),
            H264InspectionError::NalHeader { index, message } => {
                write!(f, "nal[{index}] header error: {message}")
            }
            H264InspectionError::Sps { index, message } => {
                write!(f, "nal[{index}] sps parse error: {message}")
            }
            H264InspectionError::Pps { index, message } => {
                write!(f, "nal[{index}] pps parse error: {message}")
            }
        }
    }
}

impl std::error::Error for H264InspectionError {}

#[derive(Default, Debug)]
pub(crate) struct H264AccessUnitInspectorState {
    committed_sps: Option<H264SeqParameterSet>,
    committed_pps: Option<H264PicParameterSet>,
    committed_dimensions: Option<(u32, u32)>,
}

#[derive(Clone, Default)]
pub struct H264AccessUnitInspector {
    state: Arc<Mutex<H264AccessUnitInspectorState>>,
}

impl H264AccessUnitInspector {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(H264AccessUnitInspectorState::default())),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_commit_state() -> Arc<Mutex<H264AccessUnitInspectorState>> {
        Arc::new(Mutex::new(H264AccessUnitInspectorState::default()))
    }

    /// 供其它模块单测构造与 inspector 共享的 `commit_state`（与 `H264AccessUnitInspection` 对齐）。
    #[cfg(test)]
    pub(crate) fn shared_commit_state(&self) -> Arc<Mutex<H264AccessUnitInspectorState>> {
        Arc::clone(&self.state)
    }

    pub fn committed_sps_present(&self) -> bool {
        self.state
            .lock()
            .expect("h264 inspector state poisoned")
            .committed_sps
            .is_some()
    }

    pub fn committed_pps_present(&self) -> bool {
        self.state
            .lock()
            .expect("h264 inspector state poisoned")
            .committed_pps
            .is_some()
    }

    pub fn seed_committed_parameter_sets_if_absent(
        &self,
        sps_bytes: &[u8],
        pps_bytes: &[u8],
    ) -> Result<bool, H264InspectionError> {
        let mut state = self.state.lock().expect("h264 inspector state poisoned");
        if state.committed_sps.is_some() && state.committed_pps.is_some() {
            return Ok(false);
        }

        let sps = parse_sps(sps_bytes, 0)?;
        let mut ctx = Context::new();
        ctx.put_seq_param_set(sps.parsed.clone());
        let pps = parse_pps(&ctx, pps_bytes, 1)?;
        let dimensions = sps.parsed.pixel_dimensions().ok();

        state.committed_sps = Some(sps);
        state.committed_pps = Some(pps);
        state.committed_dimensions = dimensions;
        Ok(true)
    }

    pub fn inspect_access_unit(
        &self,
        payload: &[u8],
    ) -> Result<H264AccessUnitInspection, H264InspectionError> {
        let nals = split_annex_b_nals(payload);
        if nals.is_empty() {
            return Err(H264InspectionError::EmptyAccessUnit);
        }

        let state = self.state.lock().expect("h264 inspector state poisoned");
        let mut working_ctx = build_context(&state);
        let committed_sps = state.committed_sps.clone();
        let committed_pps = state.committed_pps.clone();
        let mut parsed_sps: Option<H264SeqParameterSet> = None;
        let mut parsed_pps: Option<H264PicParameterSet> = None;
        let mut seen_sps = false;
        let mut seen_pps = false;
        let mut parameter_sets_changed = false;
        let mut width_height = state.committed_dimensions;

        for (index, nal) in nals.iter().enumerate() {
            let header = parse_nal_header(nal.bytes, index)?;
            match header.nal_unit_type() {
                UnitType::SeqParameterSet => {
                    seen_sps = true;
                    let parsed = parse_sps(nal.bytes, index)?;
                    width_height = parsed.parsed.pixel_dimensions().ok();
                    if state
                        .committed_sps
                        .as_ref()
                        .is_none_or(|committed| !committed.same_decoder_configuration(&parsed))
                    {
                        parameter_sets_changed = true;
                    }
                    working_ctx.put_seq_param_set(parsed.parsed.clone());
                    parsed_sps = Some(parsed);
                }
                _ => {}
            }
        }

        for (index, nal) in nals.iter().enumerate() {
            let header = parse_nal_header(nal.bytes, index)?;
            if header.nal_unit_type() == UnitType::PicParameterSet {
                seen_pps = true;
                let parsed = parse_pps(&working_ctx, nal.bytes, index)?;
                if state
                    .committed_pps
                    .as_ref()
                    .is_none_or(|committed| !committed.same_decoder_configuration(&parsed))
                {
                    parameter_sets_changed = true;
                }
                working_ctx.put_pic_param_set(parsed.parsed.clone());
                parsed_pps = Some(parsed);
            }
        }

        let mut is_idr = false;
        let mut has_vcl = false;
        let mut has_non_idr_vcl = false;
        let mut slice_headers_valid = true;
        let mut first_vcl_seen = false;

        for (index, nal) in nals.iter().enumerate() {
            let header = parse_nal_header(nal.bytes, index)?;
            match header.nal_unit_type() {
                UnitType::AccessUnitDelimiter => {}
                unit_type if is_vcl_unit(unit_type) => {
                    has_vcl = true;
                    if !first_vcl_seen {
                        first_vcl_seen = true;
                        is_idr = matches!(unit_type, UnitType::SliceLayerWithoutPartitioningIdr);
                    } else if !matches!(unit_type, UnitType::SliceLayerWithoutPartitioningIdr) {
                        has_non_idr_vcl = true;
                    }

                    let nal_ref = h264_reader::nal::RefNal::new(nal.bytes, &[], true);
                    let mut rbsp_bits = nal_ref.rbsp_bits();
                    let slice_result = SliceHeader::from_bits(&working_ctx, &mut rbsp_bits, header);
                    if slice_result.is_err() {
                        slice_headers_valid = false;
                    }
                }
                _ => {}
            }
        }

        let parameter_sets = parsed_sps
            .clone()
            .or_else(|| committed_sps.clone())
            .zip(parsed_pps.clone().or_else(|| committed_pps.clone()))
            .map(|(sps, pps)| H264ParameterSets { sps, pps });

        let effective_sps_present = seen_sps || committed_sps.is_some();
        let effective_pps_present = seen_pps || committed_pps.is_some();

        let bootstrap_reject_reason = if !has_vcl {
            Some(H264BootstrapRejectReason::NoVcl)
        } else if !effective_sps_present {
            Some(H264BootstrapRejectReason::MissingSps)
        } else if !effective_pps_present {
            Some(H264BootstrapRejectReason::MissingPps)
        } else if !slice_headers_valid {
            Some(H264BootstrapRejectReason::InvalidSliceHeader)
        } else if !is_idr {
            Some(H264BootstrapRejectReason::BootstrapMissingIdr)
        } else if has_non_idr_vcl {
            Some(H264BootstrapRejectReason::MixedIdrWithTrailingDelta)
        } else {
            None
        };

        let bootstrap_ready = bootstrap_reject_reason.is_none();
        let committed_dimensions = state.committed_dimensions;
        let config_changed = parameter_sets_changed || width_height != committed_dimensions;
        let nals = nals
            .iter()
            .enumerate()
            .map(|(index, nal)| {
                let header = parse_nal_header(nal.bytes, index)?;
                Ok(H264NalUnit {
                    range: nal.range.clone(),
                    unit_type: header.nal_unit_type(),
                })
            })
            .collect::<Result<Vec<_>, H264InspectionError>>()?;

        Ok(H264AccessUnitInspection {
            nals,
            parameter_sets,
            width: width_height.map(|(width, _)| width),
            height: width_height.map(|(_, height)| height),
            is_idr,
            has_inband_sps: seen_sps,
            has_inband_pps: seen_pps,
            slice_headers_valid,
            parameter_sets_changed,
            config_changed,
            bootstrap_ready,
            bootstrap_reject_reason,
            commit_state: Arc::clone(&self.state),
        })
    }
}

fn build_context(state: &H264AccessUnitInspectorState) -> Context {
    let mut ctx = Context::new();
    if let Some(sps) = state.committed_sps.as_ref() {
        ctx.put_seq_param_set(sps.parsed.clone());
    }
    if let Some(pps) = state.committed_pps.as_ref() {
        ctx.put_pic_param_set(pps.parsed.clone());
    }
    ctx
}

struct AnnexBNal<'a> {
    bytes: &'a [u8],
    range: Range<usize>,
}

fn split_annex_b_nals(data: &[u8]) -> Vec<AnnexBNal<'_>> {
    let mut nals = Vec::new();
    let mut i = 0usize;
    while i + 3 < data.len() {
        let start_len = if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            3
        } else if i + 4 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };

        let nal_start = i + start_len;
        let mut nal_end = data.len();
        let mut j = nal_start;
        while j + 3 < data.len() {
            let has_three = data[j] == 0 && data[j + 1] == 0 && data[j + 2] == 1;
            let has_four = j + 4 < data.len()
                && data[j] == 0
                && data[j + 1] == 0
                && data[j + 2] == 0
                && data[j + 3] == 1;
            if has_three || has_four {
                nal_end = j;
                break;
            }
            j += 1;
        }

        if nal_start < nal_end {
            nals.push(AnnexBNal {
                bytes: &data[nal_start..nal_end],
                range: nal_start..nal_end,
            });
        }
        i = nal_end;
    }
    nals
}

fn parse_nal_header(bytes: &[u8], index: usize) -> Result<NalHeader, H264InspectionError> {
    let Some(&header_byte) = bytes.first() else {
        return Err(H264InspectionError::NalHeader {
            index,
            message: "empty nal".to_string(),
        });
    };
    NalHeader::new(header_byte).map_err(|err| H264InspectionError::NalHeader {
        index,
        message: format_nal_header_error(err),
    })
}

fn parse_sps(bytes: &[u8], index: usize) -> Result<H264SeqParameterSet, H264InspectionError> {
    let nal = h264_reader::nal::RefNal::new(bytes, &[], true);
    let parsed =
        SeqParameterSet::from_bits(nal.rbsp_bits()).map_err(|err| H264InspectionError::Sps {
            index,
            message: format!("{err:?}"),
        })?;
    Ok(H264SeqParameterSet {
        raw: bytes.to_vec(),
        parsed,
    })
}

fn parse_pps(
    ctx: &Context,
    bytes: &[u8],
    index: usize,
) -> Result<H264PicParameterSet, H264InspectionError> {
    let nal = h264_reader::nal::RefNal::new(bytes, &[], true);
    let parsed = PicParameterSet::from_bits(ctx, nal.rbsp_bits()).map_err(|err| {
        H264InspectionError::Pps {
            index,
            message: format!("{err:?}"),
        }
    })?;
    Ok(H264PicParameterSet {
        raw: bytes.to_vec(),
        parsed,
    })
}

fn is_vcl_unit(unit_type: UnitType) -> bool {
    matches!(
        unit_type,
        UnitType::SliceLayerWithoutPartitioningNonIdr
            | UnitType::SliceDataPartitionALayer
            | UnitType::SliceDataPartitionBLayer
            | UnitType::SliceDataPartitionCLayer
            | UnitType::SliceLayerWithoutPartitioningIdr
            | UnitType::SliceLayerWithoutPartitioningAux
            | UnitType::SliceExtension
            | UnitType::SliceExtensionViewComponent
    )
}

fn format_nal_header_error(err: NalHeaderError) -> String {
    format!("{err:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_inspector() -> H264AccessUnitInspector {
        H264AccessUnitInspector::new()
    }

    #[test]
    fn bootstrap_ready_requires_idr_sps_pps_and_valid_slices() {
        let inspector = make_inspector();
        let payload = hex_literal::hex!(
            "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
             01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
             4E 7F 87 DF"
        );

        let inspection = inspector.inspect_access_unit(&payload).expect("inspection");
        assert!(inspection.bootstrap_ready);
        assert!(inspection.is_idr);
        assert!(inspection.has_inband_sps);
        assert!(inspection.has_inband_pps);
        assert!(inspection.slice_headers_valid);
        assert_eq!(inspection.bootstrap_reject_reason, None);
        assert!(inspection.parameter_sets.is_some());

        let avcc = inspection.build_avcc_payload(&payload);
        assert!(avcc.windows(1).any(|window| window == [0x65]));
    }

    #[test]
    fn missing_parameter_sets_block_bootstrap() {
        let inspector = make_inspector();
        let payload = hex_literal::hex!("00 00 00 01 65 88 81 00 05");

        let inspection = inspector.inspect_access_unit(&payload).expect("inspection");
        assert!(!inspection.bootstrap_ready);
        assert_eq!(
            inspection.bootstrap_reject_reason,
            Some(H264BootstrapRejectReason::MissingSps)
        );
        assert!(inspection.parameter_sets.is_none());
    }

    #[test]
    fn committed_parameter_sets_allow_idr_bootstrap_without_inband_sets() {
        let inspector = make_inspector();
        let bootstrap_payload = hex_literal::hex!(
            "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
             01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
             4E 7F 87 DF"
        );
        let bootstrap = inspector
            .inspect_access_unit(&bootstrap_payload)
            .expect("bootstrap inspection");
        bootstrap.commit();

        let idr_without_sets = hex_literal::hex!("00 00 00 01 65 88 81 00 05 4E 7F 87 DF");
        let inspection = inspector
            .inspect_access_unit(&idr_without_sets)
            .expect("idr inspection");

        assert!(inspection.bootstrap_ready);
        assert_eq!(inspection.bootstrap_reject_reason, None);
        assert!(inspection.parameter_sets.is_some());
        assert!(!inspection.has_inband_sps);
        assert!(!inspection.has_inband_pps);
    }

    #[test]
    fn partial_inband_parameter_refresh_reuses_committed_counterpart() {
        let inspector = make_inspector();
        let bootstrap_payload = hex_literal::hex!(
            "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
             01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
             4E 7F 87 DF"
        );
        let bootstrap = inspector
            .inspect_access_unit(&bootstrap_payload)
            .expect("bootstrap inspection");
        let committed = bootstrap.parameter_sets.clone().expect("committed sets");
        bootstrap.commit();

        let refreshed_sps_only = hex_literal::hex!(
            "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 01
             65 88 81 00 05 4E 7F 87 DF"
        );
        let inspection = inspector
            .inspect_access_unit(&refreshed_sps_only)
            .expect("refresh inspection");
        let effective = inspection.parameter_sets.expect("effective parameter sets");

        assert_eq!(effective.pps.raw, committed.pps.raw);
        assert!(inspection.bootstrap_ready);
        assert_eq!(inspection.bootstrap_reject_reason, None);
    }

    #[test]
    fn avcc_payload_excludes_parameter_sets_and_aud() {
        let inspector = make_inspector();
        let payload = hex_literal::hex!(
            "00 00 00 01 09 F0
             00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80
             00 00 00 01 68 E8 43 8F 13 21 30
             00 00 00 01 65 88 81 00 05 4E 7F 87 DF"
        );

        let inspection = inspector.inspect_access_unit(&payload).expect("inspection");
        let avcc = inspection.build_avcc_payload(&payload);
        assert!(!avcc.windows(2).any(|window| window == [0x09, 0xF0]));
        assert!(!avcc.windows(1).any(|window| window == [0x67]));
        assert!(!avcc.windows(1).any(|window| window == [0x68]));
        assert!(avcc.windows(1).any(|window| window == [0x65]));
    }

    #[test]
    fn decoder_avcc_payload_can_prepend_committed_parameter_sets() {
        let inspector = make_inspector();
        let sps = hex_literal::hex!(
            "67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80"
        );
        let pps = hex_literal::hex!("68 E8 43 8F 13 21 30");
        let idr_only = hex_literal::hex!("00 00 00 01 65 88 81 00 05 4E 7F 87 DF");

        inspector
            .seed_committed_parameter_sets_if_absent(&sps, &pps)
            .expect("seed should succeed");
        let inspection = inspector
            .inspect_access_unit(&idr_only)
            .expect("inspection");

        let avcc = inspection.build_decoder_avcc_payload(&idr_only, true);
        let expected_sps_len = (sps.len() as u32).to_be_bytes();
        let expected_pps_len = (pps.len() as u32).to_be_bytes();

        assert!(avcc.starts_with(expected_sps_len.as_slice()));
        assert!(avcc[4..].starts_with(&sps));
        let pps_offset = 4 + sps.len();
        assert!(avcc[pps_offset..].starts_with(expected_pps_len.as_slice()));
        assert!(avcc[(pps_offset + 4)..].starts_with(&pps));
        assert!(avcc.windows(1).any(|window| window == [0x65]));
    }

    #[test]
    fn semantic_parameter_set_comparison_treats_id_refresh_as_configuration_change() {
        let inspector = make_inspector();
        let payload = hex_literal::hex!(
            "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
             01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
             4E 7F 87 DF"
        );

        let inspection = inspector.inspect_access_unit(&payload).expect("inspection");
        let original = inspection.parameter_sets.expect("parameter sets");
        let mut refreshed = original.clone();

        refreshed.sps.parsed.seq_parameter_set_id =
            h264_reader::nal::sps::SeqParamSetId::from_u32(7).expect("sps id");
        refreshed.pps.parsed.pic_parameter_set_id =
            h264_reader::nal::pps::PicParamSetId::from_u32(9).expect("pps id");
        refreshed.pps.parsed.seq_parameter_set_id =
            h264_reader::nal::sps::SeqParamSetId::from_u32(7).expect("sps id");

        assert!(!original.same_decoder_configuration(&refreshed));

        refreshed.sps.parsed.level_idc = refreshed.sps.parsed.level_idc.saturating_add(1);
        assert!(!original.same_decoder_configuration(&refreshed));
    }

    #[test]
    fn seeded_parameter_sets_allow_bootstrap_without_inband_sps_pps() {
        let inspector = make_inspector();
        let sps = hex_literal::hex!(
            "67 64 00 0A AC 72 84 44 26 84 00 00
             03 00 04 00 00 03 00 CA 3C 48 96 11 80"
        );
        let pps = hex_literal::hex!("68 E8 43 8F 13 21 30");
        let idr_only = hex_literal::hex!("00 00 00 01 65 88 81 00 05 4E 7F 87 DF");

        let seeded = inspector
            .seed_committed_parameter_sets_if_absent(&sps, &pps)
            .expect("seed should succeed");
        assert!(seeded);

        let inspection = inspector
            .inspect_access_unit(&idr_only)
            .expect("inspection");
        assert!(inspection.committed_sps_present());
        assert!(inspection.committed_pps_present());
        assert!(inspection.slice_headers_valid);
        assert!(inspection.is_idr);
        assert!(inspection.bootstrap_ready);
        assert_eq!(inspection.bootstrap_reject_reason, None);
    }
}
