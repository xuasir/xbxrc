//! 设置到领域偏好的纯映射。
use super::*;

pub(super) fn apply_streaming_preferences(
    config: &mut DomainStreamingConfig,
    snapshot: &StreamingConfigSnapshot,
) {
    // 这里只做原始设置到 crate 偏好的搬运，不在 tauri 层解释执行策略。
    config.negotiation.video_codec = parse_codec_preference(&snapshot.codec);
    config.negotiation.home_video_bitrate =
        parse_bitrate_preference(&snapshot.xhome_bitrate_mode, snapshot.xhome_bitrate);
    config.negotiation.cloud_video_bitrate =
        parse_bitrate_preference(&snapshot.xcloud_bitrate_mode, snapshot.xcloud_bitrate);
    config.negotiation.audio_bitrate =
        parse_audio_bitrate_preference(&snapshot.audio_bitrate_mode, snapshot.audio_bitrate);
    config.negotiation.audio_channels = AudioChannels::Auto;

    config.input.polling_rate_hz = snapshot.polling_rate.clamp(1, u16::MAX as i64) as u16;
    config.input.vibration = snapshot.vibration;
    config.session.power_on = snapshot.power_on;

    config.runtime.mode = parse_runtime_preference(&snapshot.stream_runtime_mode);
    config.runtime.custom_turn = resolve_custom_turn(snapshot);
    config.runtime.home_fallback_turn = snapshot.xhome_turn_fallback;

    config.render.enable_audio_control = snapshot.enable_audio_control;
    config.render.video_format = normalize_optional(&snapshot.video_format);
    config.render.display_options = DomainDisplayOptions {
        sharpness: snapshot.display_options.sharpness,
        saturation: snapshot.display_options.saturation,
        contrast: snapshot.display_options.contrast,
        brightness: snapshot.display_options.brightness,
    };
    config.render.super_resolution_preference = if snapshot.super_resolution_experimental {
        SuperResolutionRenderPreference::Fsr1Experimental
    } else {
        SuperResolutionRenderPreference::Off
    };
}

pub(super) fn parse_bitrate_preference(mode: &str, bitrate_mbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_mbps <= 0 {
        return BitratePreference::Auto;
    }

    let kbps = bitrate_mbps.saturating_mul(1000);
    BitratePreference::CustomKbps {
        kbps: kbps.clamp(1, u32::MAX as i64) as u32,
    }
}

pub(super) fn parse_audio_bitrate_preference(mode: &str, bitrate_kbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_kbps <= 0 {
        return BitratePreference::Auto;
    }

    BitratePreference::CustomKbps {
        kbps: bitrate_kbps.clamp(1, u32::MAX as i64) as u32,
    }
}

pub(super) fn parse_codec_preference(value: &str) -> CodecPreference {
    match value.trim() {
        "" => CodecPreference::Auto,
        "video/H264-420" => CodecPreference::H264Low,
        "video/H264-42e" => CodecPreference::H264Normal,
        "video/H264-4d" => CodecPreference::H264Main,
        "video/H264-64" => CodecPreference::H264High,
        mime_type => CodecPreference::MimeType {
            mime_type: mime_type.to_string(),
        },
    }
}

pub(super) fn parse_runtime_preference(value: &str) -> RuntimePreference {
    match value.trim() {
        "webrtc-direct" => RuntimePreference::WebRtcDirect,
        "rust-owned" => RuntimePreference::RustOwned,
        _ => RuntimePreference::Auto,
    }
}

pub(super) fn resolve_custom_turn(snapshot: &StreamingConfigSnapshot) -> Option<TurnServer> {
    Some(TurnServer {
        url: normalize_optional(&snapshot.server_url)?,
        username: normalize_optional(&snapshot.server_username)?,
        credential: normalize_optional(&snapshot.server_credential)?,
    })
}
