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
    pub raw: Vec<u8>,
    pub parsed: SeqParameterSet,
}

#[derive(Clone, Debug)]
pub struct H264PicParameterSet {
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
    pub has_vcl: bool,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub has_aud: bool,
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

    pub fn bootstrap_parameter_sets(&self) -> Option<&H264ParameterSets> {
        self.parameter_sets.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H264BootstrapRejectReason {
    NoVcl,
    MissingSps,
    MissingPps,
    NonIdrVcl,
    InvalidSliceHeader,
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
                        .is_none_or(|committed| committed.raw != parsed.raw)
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
                    .is_none_or(|committed| committed.raw != parsed.raw)
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
        let mut has_aud = false;
        let mut slice_headers_valid = true;
        let mut first_vcl_seen = false;

        for (index, nal) in nals.iter().enumerate() {
            let header = parse_nal_header(nal.bytes, index)?;
            match header.nal_unit_type() {
                UnitType::AccessUnitDelimiter => {
                    has_aud = true;
                }
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

        let parameter_sets = match (parsed_sps, parsed_pps) {
            (Some(sps), Some(pps)) => Some(H264ParameterSets { sps, pps }),
            _ => None,
        };

        let bootstrap_reject_reason = if !has_vcl {
            Some(H264BootstrapRejectReason::NoVcl)
        } else if !seen_sps {
            Some(H264BootstrapRejectReason::MissingSps)
        } else if !seen_pps {
            Some(H264BootstrapRejectReason::MissingPps)
        } else if !is_idr {
            Some(H264BootstrapRejectReason::NonIdrVcl)
        } else if !slice_headers_valid {
            Some(H264BootstrapRejectReason::InvalidSliceHeader)
        } else if has_non_idr_vcl {
            Some(H264BootstrapRejectReason::NonIdrVcl)
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
            has_vcl,
            has_inband_sps: seen_sps,
            has_inband_pps: seen_pps,
            has_aud,
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
}
