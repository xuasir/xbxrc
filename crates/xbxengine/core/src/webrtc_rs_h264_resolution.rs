const NAL_TYPE_MASK: u8 = 0x1F;
const NAL_TYPE_SPS: u8 = 7;
const NAL_TYPE_STAP_A: u8 = 24;
const NAL_TYPE_FU_A: u8 = 28;
const MAX_SPS_NAL_SIZE: usize = 4096;

#[derive(Debug, Default)]
pub(crate) struct H264ResolutionTracker {
    fu_timestamp: Option<u32>,
    fu_sps_buffer: Vec<u8>,
    latest_resolution: Option<(u32, u32)>,
}

impl H264ResolutionTracker {
    pub(crate) fn ingest_rtp_payload(
        &mut self,
        rtp_timestamp: u32,
        payload: &[u8],
    ) -> Option<(u32, u32)> {
        let resolution = extract_resolution_from_rtp_payload(self, rtp_timestamp, payload)?;
        if self.latest_resolution != Some(resolution) {
            self.latest_resolution = Some(resolution);
            return Some(resolution);
        }
        None
    }

    fn reset_fu_buffer(&mut self) {
        self.fu_timestamp = None;
        self.fu_sps_buffer.clear();
    }
}

fn extract_resolution_from_rtp_payload(
    tracker: &mut H264ResolutionTracker,
    rtp_timestamp: u32,
    payload: &[u8],
) -> Option<(u32, u32)> {
    let first_byte = *payload.first()?;
    let nal_type = first_byte & NAL_TYPE_MASK;
    match nal_type {
        1..=23 => {
            tracker.reset_fu_buffer();
            parse_sps_dimensions_from_nal(payload)
        }
        NAL_TYPE_STAP_A => {
            tracker.reset_fu_buffer();
            parse_sps_dimensions_from_stap_a(payload)
        }
        NAL_TYPE_FU_A => parse_sps_dimensions_from_fu_a(tracker, rtp_timestamp, payload),
        _ => {
            tracker.reset_fu_buffer();
            None
        }
    }
}

fn parse_sps_dimensions_from_stap_a(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() < 3 {
        return None;
    }
    let mut offset = 1usize;
    while offset + 2 <= payload.len() {
        let nal_size = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
        offset += 2;
        if nal_size == 0 || offset + nal_size > payload.len() {
            return None;
        }
        if let Some(resolution) = parse_sps_dimensions_from_nal(&payload[offset..offset + nal_size])
        {
            return Some(resolution);
        }
        offset += nal_size;
    }
    None
}

fn parse_sps_dimensions_from_fu_a(
    tracker: &mut H264ResolutionTracker,
    rtp_timestamp: u32,
    payload: &[u8],
) -> Option<(u32, u32)> {
    if payload.len() < 3 {
        tracker.reset_fu_buffer();
        return None;
    }
    let fu_indicator = payload[0];
    let fu_header = payload[1];
    let start = fu_header & 0x80 != 0;
    let end = fu_header & 0x40 != 0;
    let nal_type = fu_header & NAL_TYPE_MASK;
    if nal_type != NAL_TYPE_SPS {
        if start || end {
            tracker.reset_fu_buffer();
        }
        return None;
    }

    if start {
        tracker.fu_timestamp = Some(rtp_timestamp);
        tracker.fu_sps_buffer.clear();
        tracker
            .fu_sps_buffer
            .push((fu_indicator & 0xE0) | NAL_TYPE_SPS);
        tracker.fu_sps_buffer.extend_from_slice(&payload[2..]);
        if tracker.fu_sps_buffer.len() > MAX_SPS_NAL_SIZE {
            tracker.reset_fu_buffer();
            return None;
        }
        if end {
            let resolution = parse_sps_dimensions_from_nal(&tracker.fu_sps_buffer);
            tracker.reset_fu_buffer();
            return resolution;
        }
        return None;
    }

    if tracker.fu_timestamp != Some(rtp_timestamp) || tracker.fu_sps_buffer.is_empty() {
        return None;
    }
    tracker.fu_sps_buffer.extend_from_slice(&payload[2..]);
    if tracker.fu_sps_buffer.len() > MAX_SPS_NAL_SIZE {
        tracker.reset_fu_buffer();
        return None;
    }
    if end {
        let resolution = parse_sps_dimensions_from_nal(&tracker.fu_sps_buffer);
        tracker.reset_fu_buffer();
        return resolution;
    }
    None
}

fn parse_sps_dimensions_from_nal(nal: &[u8]) -> Option<(u32, u32)> {
    if nal.len() < 2 || (nal[0] & NAL_TYPE_MASK) != NAL_TYPE_SPS {
        return None;
    }
    parse_sps_dimensions(&nal[1..])
}

fn parse_sps_dimensions(sps_payload: &[u8]) -> Option<(u32, u32)> {
    let rbsp = unescape_rbsp(sps_payload);
    let mut bits = BitReader::new(&rbsp);

    let profile_idc = bits.read_u8()?;
    let _constraint_flags_and_reserved = bits.read_u8()?;
    let _level_idc = bits.read_u8()?;
    let _seq_parameter_set_id = bits.read_ue()?;

    let mut chroma_format_idc = 1u32;
    let mut separate_colour_plane_flag = false;
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format_idc = bits.read_ue()?;
        if chroma_format_idc == 3 {
            separate_colour_plane_flag = bits.read_bit()?;
        }
        let _bit_depth_luma_minus8 = bits.read_ue()?;
        let _bit_depth_chroma_minus8 = bits.read_ue()?;
        let _qpprime_y_zero_transform_bypass_flag = bits.read_bit()?;
        let seq_scaling_matrix_present_flag = bits.read_bit()?;
        if seq_scaling_matrix_present_flag {
            let scaling_list_count = if chroma_format_idc != 3 { 8 } else { 12 };
            for index in 0..scaling_list_count {
                let seq_scaling_list_present_flag = bits.read_bit()?;
                if seq_scaling_list_present_flag {
                    skip_scaling_list(&mut bits, if index < 6 { 16 } else { 64 })?;
                }
            }
        }
    }

    let _log2_max_frame_num_minus4 = bits.read_ue()?;
    let pic_order_cnt_type = bits.read_ue()?;
    if pic_order_cnt_type == 0 {
        let _log2_max_pic_order_cnt_lsb_minus4 = bits.read_ue()?;
    } else if pic_order_cnt_type == 1 {
        let _delta_pic_order_always_zero_flag = bits.read_bit()?;
        let _offset_for_non_ref_pic = bits.read_se()?;
        let _offset_for_top_to_bottom_field = bits.read_se()?;
        let num_ref_frames_in_pic_order_cnt_cycle = bits.read_ue()?;
        for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
            let _offset_for_ref_frame = bits.read_se()?;
        }
    }

    let _max_num_ref_frames = bits.read_ue()?;
    let _gaps_in_frame_num_value_allowed_flag = bits.read_bit()?;
    let pic_width_in_mbs_minus1 = bits.read_ue()?;
    let pic_height_in_map_units_minus1 = bits.read_ue()?;
    let frame_mbs_only_flag = bits.read_bit()?;
    if !frame_mbs_only_flag {
        let _mb_adaptive_frame_field_flag = bits.read_bit()?;
    }
    let _direct_8x8_inference_flag = bits.read_bit()?;
    let frame_cropping_flag = bits.read_bit()?;

    let (
        frame_crop_left_offset,
        frame_crop_right_offset,
        frame_crop_top_offset,
        frame_crop_bottom_offset,
    ) = if frame_cropping_flag {
        (
            bits.read_ue()?,
            bits.read_ue()?,
            bits.read_ue()?,
            bits.read_ue()?,
        )
    } else {
        (0, 0, 0, 0)
    };

    let frame_mbs_only_factor = if frame_mbs_only_flag { 1 } else { 0 };
    let mut width = (pic_width_in_mbs_minus1 + 1) * 16;
    let mut height = (pic_height_in_map_units_minus1 + 1) * 16 * (2 - frame_mbs_only_factor);

    let effective_chroma_format_idc = if separate_colour_plane_flag {
        0
    } else {
        chroma_format_idc
    };
    let (sub_width_c, sub_height_c) = match effective_chroma_format_idc {
        0 => (1, 1),
        1 => (2, 2),
        2 => (2, 1),
        3 => (1, 1),
        _ => return None,
    };
    let crop_unit_x = sub_width_c;
    let crop_unit_y = sub_height_c * (2 - frame_mbs_only_factor);
    let crop_w = (frame_crop_left_offset + frame_crop_right_offset).saturating_mul(crop_unit_x);
    let crop_h = (frame_crop_top_offset + frame_crop_bottom_offset).saturating_mul(crop_unit_y);

    if crop_w >= width || crop_h >= height {
        return None;
    }
    width -= crop_w;
    height -= crop_h;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

fn skip_scaling_list(bits: &mut BitReader<'_>, size: usize) -> Option<()> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = bits.read_se()?;
            next_scale = (last_scale + delta_scale + 256) % 256;
        }
        last_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
    }
    Some(())
}

fn unescape_rbsp(sps_payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(sps_payload.len());
    let mut zero_count = 0u8;
    for &byte in sps_payload {
        if zero_count >= 2 && byte == 0x03 {
            zero_count = 0;
            continue;
        }
        rbsp.push(byte);
        if byte == 0 {
            zero_count = zero_count.saturating_add(1);
        } else {
            zero_count = 0;
        }
    }
    rbsp
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<bool> {
        if self.bit_pos >= self.data.len().saturating_mul(8) {
            return None;
        }
        let byte = self.data[self.bit_pos / 8];
        let bit_shift = 7 - (self.bit_pos % 8);
        self.bit_pos = self.bit_pos.saturating_add(1);
        Some(((byte >> bit_shift) & 0x01) == 1)
    }

    fn read_bits(&mut self, bit_count: usize) -> Option<u32> {
        if bit_count == 0 || bit_count > 32 {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..bit_count {
            value <<= 1;
            if self.read_bit()? {
                value |= 1;
            }
        }
        Some(value)
    }

    fn read_u8(&mut self) -> Option<u8> {
        self.read_bits(8).map(|value| value as u8)
    }

    fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zero_bits = 0u32;
        while !self.read_bit()? {
            leading_zero_bits = leading_zero_bits.saturating_add(1);
            if leading_zero_bits > 31 {
                return None;
            }
        }
        if leading_zero_bits == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zero_bits as usize)?;
        Some(((1u32 << leading_zero_bits) - 1).saturating_add(suffix))
    }

    fn read_se(&mut self) -> Option<i32> {
        let code_num = self.read_ue()? as i32;
        if code_num % 2 == 0 {
            Some(-(code_num / 2))
        } else {
            Some((code_num + 1) / 2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::H264ResolutionTracker;

    #[test]
    fn parse_resolution_from_single_sps_nal() {
        let mut tracker = H264ResolutionTracker::default();
        let sps_1280x720 = vec![
            0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00,
            0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc8, 0xf1, 0x83, 0x19, 0x60,
        ];
        let resolution = tracker.ingest_rtp_payload(9_000, &sps_1280x720);
        assert_eq!(resolution, Some((1280, 720)));
    }

    #[test]
    fn parse_resolution_from_stap_a() {
        let mut tracker = H264ResolutionTracker::default();
        let sps_1280x720 = vec![
            0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00,
            0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc8, 0xf1, 0x83, 0x19, 0x60,
        ];
        let pps = vec![0x68, 0xee, 0x3c, 0x80];
        let mut stap_a = vec![0x78];
        stap_a.extend_from_slice(&(sps_1280x720.len() as u16).to_be_bytes());
        stap_a.extend_from_slice(&sps_1280x720);
        stap_a.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        stap_a.extend_from_slice(&pps);

        let resolution = tracker.ingest_rtp_payload(9_000, &stap_a);
        assert_eq!(resolution, Some((1280, 720)));
    }

    #[test]
    fn parse_resolution_from_fu_a_sps() {
        let mut tracker = H264ResolutionTracker::default();
        let sps_1280x720_payload = vec![
            0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00, 0x03,
            0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc8, 0xf1, 0x83, 0x19, 0x60,
        ];

        let mut first = vec![0x7c, 0x87];
        first.extend_from_slice(&sps_1280x720_payload[..12]);
        let mut second = vec![0x7c, 0x47];
        second.extend_from_slice(&sps_1280x720_payload[12..]);

        assert_eq!(tracker.ingest_rtp_payload(9_000, &first), None);
        assert_eq!(
            tracker.ingest_rtp_payload(9_000, &second),
            Some((1280, 720))
        );
    }
}
