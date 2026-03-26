use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::transport::rtc::stream::runtime_state::RtcMediaIngressSnapshot;

pub(crate) fn merge_media_snapshot_into_runtime_stats(
    stats: &mut XbxEngineMediaRuntimeStats,
    media_snapshot: &RtcMediaIngressSnapshot,
    now_ms: f64,
) {
    let had_video_before_merge = stats.inbound_video_bytes_total > 0;
    let inbound_video_packet_count_total = media_snapshot
        .inbound_primary_video_count
        .saturating_add(media_snapshot.inbound_repair_video_count);
    let inbound_video_bytes_total = media_snapshot
        .inbound_primary_video_bytes
        .saturating_add(media_snapshot.inbound_repair_video_bytes);

    stats.inbound_audio_bytes_total = stats
        .inbound_audio_bytes_total
        .max(media_snapshot.inbound_audio_bytes);
    stats.inbound_primary_video_bytes_total = stats
        .inbound_primary_video_bytes_total
        .max(media_snapshot.inbound_primary_video_bytes);
    stats.inbound_video_packet_count_total = stats
        .inbound_video_packet_count_total
        .max(inbound_video_packet_count_total);
    stats.inbound_video_bytes_total = stats
        .inbound_video_bytes_total
        .max(inbound_video_bytes_total);
    stats.inbound_bytes_total = stats.inbound_video_bytes_total + stats.inbound_audio_bytes_total;
    let video_width = stats
        .latest_video_frame
        .as_ref()
        .map(|frame| frame.width)
        .or(stats.latest_video_stream_width)
        .or_else(|| {
            stats
                .latest_video_track_status
                .as_ref()
                .and_then(|status| status.video_width)
        });
    let video_height = stats
        .latest_video_frame
        .as_ref()
        .map(|frame| frame.height)
        .or(stats.latest_video_stream_height)
        .or_else(|| {
            stats
                .latest_video_track_status
                .as_ref()
                .and_then(|status| status.video_height)
        });
    let mime_type = stats
        .latest_video_track_status
        .as_ref()
        .and_then(|status| status.mime_type.clone());

    // 首次观测到主视频 RTP 时先打 primaryVideoRtpStarted，下一轮再升级为 remoteTrackAttached，
    // 避免同一轮 merge 内状态被覆盖导致前者在运行时不可观测。
    stats.latest_video_track_status = if stats.inbound_video_bytes_total > 0 {
        let state = if had_video_before_merge {
            "remoteTrackAttached"
        } else {
            "primaryVideoRtpStarted"
        };
        let (video_bytes_total, video_packet_count_total) = if state == "remoteTrackAttached" {
            (
                stats.inbound_video_bytes_total,
                stats.inbound_video_packet_count_total,
            )
        } else {
            (
                stats.inbound_primary_video_bytes_total,
                media_snapshot.inbound_primary_video_count,
            )
        };
        Some(crate::XbxEngineVideoTrackStatus {
            state: state.to_string(),
            video_width,
            video_height,
            mime_type,
            transport_state: stats.transport_state.clone(),
            video_bytes_total,
            video_packet_count_total,
            audio_bytes_total: stats.inbound_audio_bytes_total,
            observed_at_ms: now_ms,
        })
    } else if stats.inbound_audio_bytes_total > 0 {
        Some(crate::XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type,
            transport_state: stats.transport_state.clone(),
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: stats.inbound_audio_bytes_total,
            observed_at_ms: now_ms,
        })
    } else {
        stats.latest_video_track_status.clone()
    };
}
