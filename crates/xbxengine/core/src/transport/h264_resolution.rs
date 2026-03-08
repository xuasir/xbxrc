const NAL_TYPE_MASK: u8 = 0x1F;
const NAL_TYPE_SPS: u8 = 7;

pub(crate) fn parse_sps_dimensions_from_nal(nal: &[u8]) -> Option<(u32, u32)> {
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
