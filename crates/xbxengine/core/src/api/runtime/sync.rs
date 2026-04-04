use xbxengine_protocol::{
    XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto, XbxEngineTransportStateDto,
    XbxEngineVideoTrackStatusDto,
};

use super::{XbxEngineEventSink, XbxEngineHostBridge, XbxEngineRuntime};
use crate::{
    XbxEngineInputStatus, XbxEngineMediaBackend, XbxEngineMediaNegotiation,
    XbxEngineMediaRuntimeStats,
};

impl<THostBridge, TEventSink, TMediaBackend>
    XbxEngineRuntime<THostBridge, TEventSink, TMediaBackend>
where
    THostBridge: XbxEngineHostBridge,
    TEventSink: XbxEngineEventSink,
    TMediaBackend: XbxEngineMediaBackend,
{
    pub(super) fn record_media_ready(&mut self, negotiation: &XbxEngineMediaNegotiation) {
        self.snapshot.surface_id = Some(negotiation.surface_id.clone());
        if let Some(viewport) = self.snapshot.viewport.as_ref() {
            if let Err(error) = self
                .host_bridge
                .attach_viewport(viewport, Some(&negotiation.surface_id))
            {
                self.emit_error("attachViewportFailed", error.to_string());
            }
        }
        self.health.reset_video_epoch();
        self.snapshot.video_size = Some((negotiation.video_width, negotiation.video_height));
        self.snapshot.first_frame_packet_arrival_time_ms =
            negotiation.first_frame_packet_arrival_time_ms;
        self.snapshot.frame_decoded_time_ms = negotiation.frame_decoded_time_ms;
        self.snapshot.frame_rendered_time_ms = negotiation.frame_rendered_time_ms;
        self.snapshot.latest_video_track_status = Some(XbxEngineVideoTrackStatusDto {
            state: "negotiated".to_string(),
            video_width: Some(negotiation.video_width),
            video_height: Some(negotiation.video_height),
            mime_type: None,
            transport_state: XbxEngineTransportStateDto::New,
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: 0,
            observed_at_ms: super::now_ms_f64(),
        });
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::MediaSurfaceReady {
                surface_id: negotiation.surface_id.clone(),
            });
        // 这里的 ready 只表示协商完成并拿到 video 尺寸，不代表第一帧已经进入解码/渲染。
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::MediaVideoReady {
                width: negotiation.video_width,
                height: negotiation.video_height,
            });
        if let Some(status) = self.snapshot.latest_video_track_status.clone() {
            self.event_sink
                .emit(XbxEngineRuntimeEventDto::MediaVideoTrackStatusChanged { status });
        }
    }

    pub(super) fn present_latest_render_frame(&mut self) {
        let Some(viewport) = self.snapshot.viewport.clone() else {
            return;
        };
        let frame = match self.media_backend.take_latest_render_frame() {
            Ok(frame) => frame,
            Err(error) => {
                self.emit_error("takeLatestRenderFrameFailed", error.to_string());
                return;
            }
        };
        let Some(frame) = frame else {
            return;
        };
        if let Err(error) =
            self.host_bridge
                .present_frame(&viewport, self.snapshot.surface_id.as_deref(), &frame)
        {
            self.emit_error("presentFrameFailed", error.to_string());
            return;
        }
        if let Err(error) = self
            .media_backend
            .acknowledge_latest_render_frame(frame.frame_seq)
        {
            self.emit_error("acknowledgeLatestRenderFrameFailed", error.to_string());
        }
        self.snapshot.video_size = Some((frame.width, frame.height));
        self.snapshot.frame_rendered_time_ms = Some(frame.rendered_at_ms);
    }

    pub(super) fn record_input_status(&mut self, status: &XbxEngineInputStatus) {
        self.snapshot.input_device_count = status.device_count;
        self.snapshot.input_pad_count = status.pad_count;
        self.snapshot.input_route_attached = status.route_attached;
    }

    pub(super) fn emit_phase(&mut self, phase: XbxEngineRuntimePhaseDto) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase });
    }

    pub(super) fn emit_transport_state(&mut self, state: XbxEngineTransportStateDto) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state });
    }

    pub(super) fn emit_error(&mut self, code: impl Into<String>, message: impl Into<String>) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::ErrorReported {
                code: code.into(),
                message: message.into(),
            });
    }

    pub(super) fn sync_runtime_activity_snapshot(&mut self) {
        crate::xbx_log_debug!("[xbxengine][runtime][sync] sync_runtime_activity_snapshot enter");
        let Ok(runtime_stats) = self.media_backend.snapshot_runtime_stats() else {
            crate::xbx_log_warn!(
                "[xbxengine][runtime][sync] sync_runtime_activity_snapshot snapshot_runtime_stats_failed"
            );
            return;
        };
        crate::xbx_log_debug!(
            "[xbxengine][runtime][sync] sync_runtime_activity_snapshot got_stats transport_state={:?}",
            runtime_stats.transport_state
        );
        self.sync_recovery_snapshot(&runtime_stats);
        self.sync_transport_state(&runtime_stats);
        self.sync_video_track_status(&runtime_stats);
        self.sync_video_packet_stats(&runtime_stats);
        self.sync_video_frame_stats(&runtime_stats);
        crate::xbx_log_debug!("[xbxengine][runtime][sync] sync_runtime_activity_snapshot exit");
    }

    pub(super) fn sync_recovery_snapshot(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        // 这里取当前快照与后端统计的较大值，避免测试后端或延迟统计回写把本地计数冲掉。
        self.snapshot.recovery_keyframe_request_count = self
            .snapshot
            .recovery_keyframe_request_count
            .max(stats.video_pli_request_count_total);
        self.snapshot.recovery_decoder_reset_count = self
            .snapshot
            .recovery_decoder_reset_count
            .max(stats.video_decoder_reset_count);
    }

    pub(super) fn sync_transport_state(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let now_ms = super::now_ms_f64();
        if !self
            .health
            .sync_transport_state(&stats.transport_state, now_ms)
        {
            return;
        }
        self.emit_transport_state(stats.transport_state.clone());
    }

    pub(super) fn sync_video_frame_stats(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let Some(frame) = stats.latest_video_frame.as_ref() else {
            return;
        };
        let had_advanced_frame = frame.frame_seq > self.health.last_frame_seq;
        let video_size_changed = self.health.record_video_frame(
            frame.width,
            frame.height,
            frame.frame_seq,
            frame.rendered_at_ms,
        );
        if !had_advanced_frame {
            return;
        }
        self.snapshot.video_size = Some((frame.width, frame.height));
        self.snapshot.frame_rendered_time_ms = Some(frame.rendered_at_ms);

        if self.snapshot.first_frame_packet_arrival_time_ms.is_none() {
            self.snapshot.first_frame_packet_arrival_time_ms = Some(frame.rendered_at_ms);
        }
        if self.snapshot.frame_decoded_time_ms.is_none() {
            self.snapshot.frame_decoded_time_ms = Some(frame.rendered_at_ms);
        }

        if video_size_changed.is_some() {
            self.event_sink
                .emit(XbxEngineRuntimeEventDto::MediaVideoReady {
                    width: frame.width,
                    height: frame.height,
                });
        }

        self.event_sink
            .emit(XbxEngineRuntimeEventDto::StatsVideoFrameRendered {
                first_frame_packet_arrival_time_ms: self
                    .snapshot
                    .first_frame_packet_arrival_time_ms
                    .unwrap_or(frame.rendered_at_ms),
                frame_decoded_time_ms: self
                    .snapshot
                    .frame_decoded_time_ms
                    .unwrap_or(frame.rendered_at_ms),
                renderer_frame_time_ms: frame.rendered_at_ms,
            });
    }

    pub(super) fn sync_video_packet_stats(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        self.sync_recovery_snapshot(stats);
        let Some(arrived_at_ms) = stats.latest_video_packet_arrival_time_ms else {
            return;
        };
        self.health
            .record_video_packet_activity(stats.inbound_video_packet_count_total, arrived_at_ms);
        if self.snapshot.first_frame_packet_arrival_time_ms.is_none() {
            self.snapshot.first_frame_packet_arrival_time_ms = Some(arrived_at_ms);
        }
        self.sync_video_track_status(stats);
    }

    pub(super) fn sync_video_track_status(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let Some(status) = stats.latest_video_track_status.as_ref() else {
            return;
        };
        let status = XbxEngineVideoTrackStatusDto {
            state: status.state.clone(),
            video_width: status.video_width,
            video_height: status.video_height,
            mime_type: status.mime_type.clone(),
            transport_state: status.transport_state.clone(),
            video_bytes_total: status.video_bytes_total,
            video_packet_count_total: status.video_packet_count_total,
            audio_bytes_total: status.audio_bytes_total,
            observed_at_ms: status.observed_at_ms,
        };
        if self.snapshot.latest_video_track_status.as_ref() == Some(&status) {
            return;
        }
        self.snapshot.latest_video_track_status = Some(status.clone());
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::MediaVideoTrackStatusChanged { status });
    }
}
