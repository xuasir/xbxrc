use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::negotiation::{
    AudioChannels, BitratePreference, Codec, CodecPreference, NegotiationPlan,
};
use crate::policy::runtime::RuntimeMode;
use crate::policy::session::ResolutionPreference;
use crate::policy::types::Target;

pub fn compile_negotiation(
    config: &Config,
    context: &Context,
    runtime_mode: RuntimeMode,
) -> NegotiationPlan {
    let prefer_ipv6 = match context.target {
        Target::Home => config.negotiation.home_prefer_ipv6,
        Target::Cloud => config.negotiation.cloud_prefer_ipv6,
    };
    let codec = compile_codec(config.negotiation.video_codec.clone());
    let video_bitrate_kbps = compile_video_bitrate(config, context.target);
    let audio_bitrate_kbps = compile_audio_bitrate(config.negotiation.audio_bitrate);
    let stereo_audio = match config.negotiation.audio_channels {
        AudioChannels::Auto | AudioChannels::Stereo => true,
        AudioChannels::Mono => false,
    };
    let offer_profile = config
        .negotiation
        .offer_profile
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| match runtime_mode {
            RuntimeMode::WebRtcDirect => "browser".to_string(),
            RuntimeMode::RustOwned => "rust-owned".to_string(),
        });
    let inject_console_addrs =
        context.target.is_home() && !context.remote_play.console_addrs.is_empty();

    NegotiationPlan {
        prefer_ipv6,
        codec,
        video_bitrate_kbps,
        audio_bitrate_kbps,
        stereo_audio,
        offer_profile,
        normalize_end_of_candidates: true,
        inject_console_addrs,
        console_addrs: if inject_console_addrs {
            context.remote_play.console_addrs.clone()
        } else {
            Vec::new()
        },
    }
}

pub fn compile_codec(preference: CodecPreference) -> Option<Codec> {
    match preference {
        CodecPreference::Auto => None,
        CodecPreference::H264Low => Some(Codec {
            mime_type: "video/H264".to_string(),
            profiles: vec!["420".to_string()],
        }),
        CodecPreference::H264Normal => Some(Codec {
            mime_type: "video/H264".to_string(),
            profiles: vec!["42e".to_string()],
        }),
        CodecPreference::H264Main => Some(Codec {
            mime_type: "video/H264".to_string(),
            profiles: vec!["4d".to_string()],
        }),
        CodecPreference::H264High => Some(Codec {
            mime_type: "video/H264".to_string(),
            // 对齐 better-xcloud / Xbox 云端协商口径：
            // high 档在标准 SDP 排序里优先 Main family(4d)。
            profiles: vec!["4d".to_string()],
        }),
        CodecPreference::MimeType { mime_type } => Some(Codec {
            mime_type,
            profiles: Vec::new(),
        }),
    }
}

pub fn compile_bitrate(preference: BitratePreference) -> Option<u32> {
    match preference {
        BitratePreference::Auto => None,
        BitratePreference::CustomKbps { kbps } if kbps > 0 => Some(kbps),
        BitratePreference::CustomKbps { .. } => None,
    }
}

fn compile_video_bitrate(config: &Config, target: Target) -> Option<u32> {
    let preference = match target {
        Target::Home => config.negotiation.home_video_bitrate,
        Target::Cloud => config.negotiation.cloud_video_bitrate,
    };

    compile_bitrate(preference).or_else(|| {
        let resolution = match target {
            Target::Home => config.session.home_resolution,
            Target::Cloud => config.session.cloud_resolution,
        };
        Some(default_video_bitrate_kbps(target, resolution))
    })
}

fn compile_audio_bitrate(preference: BitratePreference) -> Option<u32> {
    compile_bitrate(preference).or(Some(128))
}

fn default_video_bitrate_kbps(target: Target, resolution: ResolutionPreference) -> u32 {
    match (target, resolution) {
        (Target::Cloud, ResolutionPreference::P720) => 10_000,
        (Target::Cloud, ResolutionPreference::P1080) => 20_000,
        (Target::Cloud, ResolutionPreference::P1080Hq) => 35_000,
        (Target::Cloud, ResolutionPreference::P1440) => 50_000,
        (Target::Cloud, ResolutionPreference::Auto) => 20_000,
        (Target::Home, ResolutionPreference::P720) => 20_000,
        (Target::Home, ResolutionPreference::P1080) => 35_000,
        (Target::Home, ResolutionPreference::P1080Hq) => 50_000,
        (Target::Home, ResolutionPreference::P1440) => 65_000,
        (Target::Home, ResolutionPreference::Auto) => 35_000,
    }
}
