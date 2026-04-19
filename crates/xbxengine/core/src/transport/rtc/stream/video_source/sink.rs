use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stream::packet_router::{RtcMediaRouteLabel, RtcPayloadRouteMap};
use crate::transport::rtc::stream::packet_types::{
    RtcMediaIngressPacket, RtcRtpPacketMeta, RtcVideoIngressKind, RtcVideoRepairMetadata,
    RtcVideoRtpPacket,
};
use crate::transport::rtc::stream::sink::RtcMediaSink;
use crate::{XbxEngineVideoFrameDropObservation, XbxEngineVideoRtxReinjectObservation};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::FrameBoundaryTracker;

const DEFAULT_PRIORITY_BACKLOG_LIMIT: usize = 32;
const MAX_BEST_EFFORT_BACKLOG_LIMIT: usize = 8;
const MIN_REPAIR_BACKLOG_LIMIT: usize = 5;
const MAX_REPAIR_BURST_BEFORE_BEST_EFFORT: u8 = 4;
const SCHEDULED_FLUSH_TICK_MS: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IngressBackpressureClass {
    PriorityPrimary,
    PriorityRepair,
    BestEffort,
}

pub(crate) struct RtcVideoSourceSink {
    tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
    pub(super) payload_route_map: Option<RtcPayloadRouteMap>,
    pub(super) runtime_stats: RuntimeStatsSink,
    pending_priority_primary: VecDeque<RtcVideoRtpPacket>,
    pending_repair: VecDeque<RtcVideoRtpPacket>,
    pending_best_effort: VecDeque<RtcVideoRtpPacket>,
    priority_backlog_limit: usize,
    repair_backlog_limit: usize,
    best_effort_backlog_limit: usize,
    repair_burst_streak: u8,
    next_drop_observation_id: u64,
    flush_tick: Duration,
    next_flush_due_at: Option<Instant>,
    frame_boundary: Arc<Mutex<FrameBoundaryTracker>>,
}

impl RtcVideoSourceSink {
    pub(super) fn new(
        tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
        runtime_stats: RuntimeStatsSink,
        frame_boundary: Arc<Mutex<FrameBoundaryTracker>>,
    ) -> Self {
        let priority_backlog_limit = tx.max_capacity().clamp(4, DEFAULT_PRIORITY_BACKLOG_LIMIT);
        let repair_backlog_limit = (priority_backlog_limit / 2)
            .clamp(MIN_REPAIR_BACKLOG_LIMIT, DEFAULT_PRIORITY_BACKLOG_LIMIT);
        let best_effort_backlog_limit =
            (priority_backlog_limit / 4).clamp(1, MAX_BEST_EFFORT_BACKLOG_LIMIT);
        Self {
            tx,
            payload_route_map: None,
            runtime_stats,
            pending_priority_primary: VecDeque::new(),
            pending_repair: VecDeque::new(),
            pending_best_effort: VecDeque::new(),
            priority_backlog_limit,
            repair_backlog_limit,
            best_effort_backlog_limit,
            repair_burst_streak: 0,
            next_drop_observation_id: 0,
            flush_tick: Duration::from_millis(SCHEDULED_FLUSH_TICK_MS),
            next_flush_due_at: None,
            frame_boundary,
        }
    }

    fn flush_pending(&mut self, now: Instant) {
        loop {
            if let Some(packet) = self.pending_priority_primary.pop_front() {
                if self.try_send_packet(packet.clone()) {
                    self.repair_burst_streak = 0;
                    continue;
                }
                self.pending_priority_primary.push_front(packet);
                break;
            }

            if self.repair_burst_streak >= MAX_REPAIR_BURST_BEFORE_BEST_EFFORT {
                if let Some(packet) = self.pending_best_effort.pop_front() {
                    if self.try_send_packet(packet.clone()) {
                        self.repair_burst_streak = 0;
                        continue;
                    }
                    self.pending_best_effort.push_front(packet);
                    break;
                }
            }

            if let Some(packet) = self.pending_repair.pop_front() {
                if self.try_send_packet(packet.clone()) {
                    self.repair_burst_streak = self.repair_burst_streak.saturating_add(1);
                    continue;
                }
                self.pending_repair.push_front(packet);
                break;
            }

            if let Some(packet) = self.pending_best_effort.pop_front() {
                if self.try_send_packet(packet.clone()) {
                    self.repair_burst_streak = 0;
                    continue;
                }
                self.pending_best_effort.push_front(packet);
                break;
            }

            break;
        }
        self.update_flush_schedule_after_attempt(now);
    }

    #[cfg(test)]
    pub(crate) fn test_flush_pending(&mut self) {
        self.flush_pending(Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn test_repair_backlog_limit(&self) -> usize {
        self.repair_backlog_limit
    }

    #[cfg(test)]
    pub(crate) fn test_has_scheduled_flush(&self) -> bool {
        self.next_flush_due_at.is_some()
    }

    #[cfg(test)]
    pub(crate) fn test_force_flush_due_now(&mut self) {
        self.next_flush_due_at = Some(Instant::now());
    }

    fn enqueue_local_backpressure(
        &mut self,
        packet: RtcVideoRtpPacket,
        class: IngressBackpressureClass,
    ) {
        match class {
            IngressBackpressureClass::PriorityPrimary => {
                if self.pending_priority_primary.len() >= self.priority_backlog_limit {
                    // 统一策略：基于 timestamp 判断新旧，优先保留当前帧
                    if let Some(oldest) = self.pending_priority_primary.front() {
                        if is_packet_newer(&packet, oldest) {
                            // 新包的 timestamp 更新，丢弃最旧的包
                            if let Some(evicted) = self.pending_priority_primary.pop_front() {
                                self.record_local_backpressure_drop(
                                    &evicted,
                                    "localBackpressurePriorityOverflow",
                                    "priorityQueueDropOldest",
                                );
                            }
                        } else {
                            // 新包的 timestamp 更旧，丢弃新包
                            self.record_local_backpressure_drop(
                                &packet,
                                "localBackpressurePriorityOverflow",
                                "priorityQueueDropStale",
                            );
                            return;
                        }
                    } else {
                        // 队列为空但长度达到限制（不应该发生）
                        self.record_local_backpressure_drop(
                            &packet,
                            "localBackpressurePriorityOverflow",
                            "priorityQueueFull",
                        );
                        return;
                    }
                }
                self.pending_priority_primary.push_back(packet);
            }
            IngressBackpressureClass::PriorityRepair => {
                if self.pending_repair.len() >= self.repair_backlog_limit {
                    // 统一策略：基于 timestamp 判断新旧
                    if let Some(oldest) = self.pending_repair.front() {
                        if is_packet_newer(&packet, oldest) {
                            if let Some(evicted) = self.pending_repair.pop_front() {
                                self.record_local_backpressure_drop(
                                    &evicted,
                                    "localBackpressureRepairOverflow",
                                    "repairQueueDropOldest",
                                );
                            }
                        } else {
                            self.record_local_backpressure_drop(
                                &packet,
                                "localBackpressureRepairOverflow",
                                "repairQueueDropStale",
                            );
                            return;
                        }
                    }
                }
                self.pending_repair.push_back(packet);
            }
            IngressBackpressureClass::BestEffort => {
                if self.pending_best_effort.len() >= self.best_effort_backlog_limit {
                    // 统一策略：基于 timestamp 判断新旧
                    if let Some(oldest) = self.pending_best_effort.front() {
                        if is_packet_newer(&packet, oldest) {
                            if let Some(replaced) = self.pending_best_effort.pop_front() {
                                self.record_local_backpressure_drop(
                                    &replaced,
                                    "localBackpressureBestEffortOverflow",
                                    "bestEffortQueueDropOldest",
                                );
                            }
                        } else {
                            self.record_local_backpressure_drop(
                                &packet,
                                "localBackpressureBestEffortOverflow",
                                "bestEffortQueueDropStale",
                            );
                            return;
                        }
                    }
                }
                self.pending_best_effort.push_back(packet);
            }
        }
    }

    fn record_local_backpressure_drop(
        &mut self,
        packet: &RtcVideoRtpPacket,
        reason: &str,
        detail: &str,
    ) {
        self.next_drop_observation_id = self.next_drop_observation_id.saturating_add(1);
        let class = classify_backpressure_class(packet, &self.frame_boundary).label();
        self.runtime_stats
            .record_video_frame_drop(XbxEngineVideoFrameDropObservation {
                observation_id: self.next_drop_observation_id,
                reason: reason.to_string(),
                stage: Some("ingress".to_string()),
                action: Some("drop".to_string()),
                detail: Some(format!("{detail}:{class}")),
                frame_rtp_timestamp: Some(packet.meta.timestamp),
                frame_seq: None,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: Some("localBackpressure".to_string()),
                frame_budget: None,
                observed_at_ms: crate::transport::rtc::stream::video_source::now_ms_f64(),
                width: 0,
                height: 0,
                is_keyframe: is_priority_primary_packet(packet),
                queue_depth: self.pending_depth_estimate(),
            });
    }

    fn pending_depth_estimate(&self) -> usize {
        let sender_depth = self.tx.max_capacity().saturating_sub(self.tx.capacity());
        sender_depth
            + self.pending_priority_primary.len()
            + self.pending_repair.len()
            + self.pending_best_effort.len()
    }

    fn has_pending_packets(&self) -> bool {
        !(self.pending_priority_primary.is_empty()
            && self.pending_repair.is_empty()
            && self.pending_best_effort.is_empty())
    }

    fn schedule_flush_if_needed(&mut self, now: Instant) {
        if !self.has_pending_packets() || self.next_flush_due_at.is_some() {
            return;
        }
        self.next_flush_due_at = Some(now + self.flush_tick);
    }

    fn update_flush_schedule_after_attempt(&mut self, now: Instant) {
        if !self.has_pending_packets() {
            self.next_flush_due_at = None;
            return;
        }
        if self.next_flush_due_at.is_none() {
            self.next_flush_due_at = Some(now + self.flush_tick);
        }
    }

    fn try_send_packet(&mut self, packet: RtcVideoRtpPacket) -> bool {
        match self.tx.try_send(packet.clone()) {
            Ok(()) => {
                if let Some(observation) =
                    build_reinject_queued_observation(&packet, self.pending_depth_estimate())
                {
                    self.runtime_stats.record_video_rtx_reinject(observation);
                }
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => false,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                crate::xbx_log_warn!("[xbxengine][rtc] video source sink channel closed");
                self.record_local_backpressure_drop(
                    &packet,
                    "localIngressChannelClosed",
                    "sourceChannelClosed",
                );
                true
            }
        }
    }
}

impl RtcMediaSink for RtcVideoSourceSink {
    fn apply_payload_route_map(&mut self, payload_route_map: Option<RtcPayloadRouteMap>) {
        self.payload_route_map = payload_route_map;
    }

    fn on_raw_packet(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        let now = Instant::now();
        let Some(normalized) = normalize_video_packet(
            packet,
            route_label,
            rtp_meta,
            self.payload_route_map.as_ref(),
        ) else {
            return;
        };

        // 时效性过滤：检查包是否属于已完成的旧帧
        let is_primary = matches!(normalized.ingress_kind, RtcVideoIngressKind::Primary);
        let is_stale = if let Ok(tracker) = self.frame_boundary.lock() {
            tracker.is_packet_stale(
                normalized.meta.timestamp,
                normalized.meta.sequence_number,
                is_primary,
            )
        } else {
            false
        };

        if is_stale {
            self.record_local_backpressure_drop(
                &normalized,
                "localBackpressureStaleRtx",
                "staleFrameRtxFiltered",
            );
            return;
        }

        if self.try_send_packet(normalized.clone()) {
            return;
        }
        let class = classify_backpressure_class(&normalized, &self.frame_boundary);
        crate::xbx_log_warn!(
            "[xbxengine][rtc] video source sink backpressure class={} seq={} ts={}",
            class.label(),
            normalized.meta.sequence_number,
            normalized.meta.timestamp
        );
        self.enqueue_local_backpressure(normalized, class);
        self.schedule_flush_if_needed(now);
        self.flush_pending(now);
    }

    /// 由外部 tick task 定期调用（约每 4ms），在包稀疏场景下主动排空背压队列。
    fn on_tick(&mut self, now: Instant) {
        if self.next_flush_due_at.is_some_and(|due| now >= due) {
            self.next_flush_due_at = None;
            self.flush_pending(now);
        }
    }
}

fn normalize_video_packet(
    packet: &RtcMediaIngressPacket,
    route_label: RtcMediaRouteLabel,
    rtp_meta: Option<&RtcRtpPacketMeta>,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    let (Some(meta), Some(payload)) = (rtp_meta, packet.rtp_payload.as_ref()) else {
        return None;
    };

    match route_label {
        RtcMediaRouteLabel::PrimaryVideo => Some(RtcVideoRtpPacket {
            payload: payload.clone(),
            meta: meta.clone(),
            ingress_kind: RtcVideoIngressKind::Primary,
        }),
        RtcMediaRouteLabel::RepairVideo => {
            normalize_repair_video_packet(meta, payload, payload_route_map)
        }
        _ => None,
    }
}

fn normalize_repair_video_packet(
    meta: &RtcRtpPacketMeta,
    payload: &[u8],
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    if is_rtx_payload(meta.payload_type, payload_route_map) {
        return unpack_rtx_packet(meta, payload, payload_route_map);
    }

    if is_primary_video_payload(meta.payload_type, payload_route_map) {
        let primary_ssrc = payload_route_map.and_then(|map| map.primary_ssrc_for_repair(meta.ssrc));
        let Some(primary_ssrc) = primary_ssrc else {
            crate::xbx_log_debug!(
                "[RtcVideoSourceSink] dropping repair-route primary payload without FID mapping pt={} ssrc={} seq={}",
                meta.payload_type,
                meta.ssrc,
                meta.sequence_number
            );
            return None;
        };
        crate::xbx_log_debug!(
            "[RtcVideoSourceSink] repair route carried primary video payload pt={} seq={}",
            meta.payload_type,
            meta.sequence_number
        );
        let mut normalized_meta = meta.clone();
        normalized_meta.ssrc = primary_ssrc;
        return Some(RtcVideoRtpPacket {
            payload: payload.to_vec(),
            meta: normalized_meta,
            ingress_kind: RtcVideoIngressKind::RepairPrimaryPassThrough {
                repair: repair_metadata(meta),
            },
        });
    }

    crate::xbx_log_debug!(
        "[RtcVideoSourceSink] ignoring unsupported repair payload pt={} len={}",
        meta.payload_type,
        payload.len()
    );
    None
}

fn is_rtx_payload(payload_type: u8, payload_route_map: Option<&RtcPayloadRouteMap>) -> bool {
    payload_route_map
        .map(|map| map.is_rtx_payload_type(payload_type))
        .unwrap_or(false)
}

fn is_primary_video_payload(
    payload_type: u8,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> bool {
    payload_route_map
        .map(|map| map.is_primary_video_payload_type(payload_type))
        .unwrap_or(false)
}

fn unpack_rtx_packet(
    meta: &RtcRtpPacketMeta,
    payload: &[u8],
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    if payload.len() < 2 {
        crate::xbx_log_debug!(
            "[RtcVideoSourceSink] truncated RTX payload pt={} seq={} len={}",
            meta.payload_type,
            meta.sequence_number,
            payload.len()
        );
        return None;
    }
    let original_sequence = u16::from_be_bytes([payload[0], payload[1]]);
    let mut normalized_meta = meta.clone();
    normalized_meta.sequence_number = original_sequence;
    let payload_route_map = payload_route_map?;
    let primary_payload_type = payload_route_map.primary_payload_type_for_rtx(meta.payload_type)?;
    if !payload_route_map.is_primary_video_payload_type(primary_payload_type) {
        crate::xbx_log_debug!(
            "[RtcVideoSourceSink] dropping RTX packet with non-primary apt target rtx_pt={} apt={} seq={}",
            meta.payload_type,
            primary_payload_type,
            meta.sequence_number
        );
        return None;
    }
    let primary_ssrc = payload_route_map.primary_ssrc_for_repair(meta.ssrc)?;
    normalized_meta.payload_type = primary_payload_type;
    normalized_meta.ssrc = primary_ssrc;
    Some(RtcVideoRtpPacket {
        payload: payload[2..].to_vec(),
        meta: normalized_meta,
        ingress_kind: RtcVideoIngressKind::RtxReinject {
            repair: repair_metadata(meta),
        },
    })
}

fn repair_metadata(meta: &RtcRtpPacketMeta) -> RtcVideoRepairMetadata {
    RtcVideoRepairMetadata {
        native_ssrc: meta.ssrc,
        native_payload_type: meta.payload_type,
        native_sequence_number: meta.sequence_number,
    }
}

fn classify_backpressure_class(
    packet: &RtcVideoRtpPacket,
    frame_boundary: &Arc<Mutex<FrameBoundaryTracker>>,
) -> IngressBackpressureClass {
    match packet.ingress_kind {
        RtcVideoIngressKind::Primary => {
            // 首包是高优先级 → PriorityPrimary
            if is_priority_primary_packet(packet) {
                return IngressBackpressureClass::PriorityPrimary;
            }

            // 检查是否属于高优先级帧（继承优先级）
            if let Ok(tracker) = frame_boundary.lock() {
                if let Some(super::FramePriority::High) =
                    tracker.get_frame_priority(packet.meta.timestamp)
                {
                    return IngressBackpressureClass::PriorityPrimary;
                }
            }

            IngressBackpressureClass::BestEffort
        }
        RtcVideoIngressKind::RepairPrimaryPassThrough { .. } => {
            // repair 路由上的 primary payload 也需要检查 NAL type
            // IDR/SPS/PPS 应进 PriorityPrimary，其余进 PriorityRepair
            if is_priority_primary_packet(packet) {
                return IngressBackpressureClass::PriorityPrimary;
            }

            // 检查是否属于高优先级帧（继承优先级）
            if let Ok(tracker) = frame_boundary.lock() {
                if let Some(super::FramePriority::High) =
                    tracker.get_frame_priority(packet.meta.timestamp)
                {
                    return IngressBackpressureClass::PriorityPrimary;
                }
            }

            IngressBackpressureClass::PriorityRepair
        }
        RtcVideoIngressKind::RtxReinject { .. } => IngressBackpressureClass::PriorityRepair,
    }
}

impl IngressBackpressureClass {
    fn label(self) -> &'static str {
        match self {
            Self::PriorityPrimary => "priority-primary",
            Self::PriorityRepair => "priority-repair",
            Self::BestEffort => "best-effort",
        }
    }
}

fn is_packet_newer(packet: &RtcVideoRtpPacket, oldest: &RtcVideoRtpPacket) -> bool {
    // 使用 RTP timestamp 的回绕安全比较
    // 如果 packet.timestamp - oldest.timestamp < 2^31，则认为 packet 更新
    const UINT32SIZE_HALF: u32 = 0x8000_0000;
    packet.meta.timestamp.wrapping_sub(oldest.meta.timestamp) < UINT32SIZE_HALF
}

fn is_priority_primary_packet(packet: &RtcVideoRtpPacket) -> bool {
    is_likely_h264_recovery_priority(&packet.payload)
}

pub(super) fn is_likely_h264_recovery_priority(payload: &[u8]) -> bool {
    let Some(&first) = payload.first() else {
        return false;
    };
    let nal_type = first & 0x1f;
    match nal_type {
        5 | 7 | 8 => true,
        24 => payload
            .get(1)
            .map(|byte| matches!(byte & 0x1f, 5 | 7 | 8))
            .unwrap_or(false),
        28 => payload
            .get(1)
            .map(|byte| (byte & 0x80) != 0 && matches!(byte & 0x1f, 5 | 7 | 8))
            .unwrap_or(false),
        _ => false,
    }
}

fn build_reinject_queued_observation(
    packet: &RtcVideoRtpPacket,
    pending_queue_len: usize,
) -> Option<XbxEngineVideoRtxReinjectObservation> {
    let (repair, primary_ssrc) = match packet.ingress_kind {
        RtcVideoIngressKind::Primary => return None,
        RtcVideoIngressKind::RepairPrimaryPassThrough { repair } => (repair, packet.meta.ssrc),
        RtcVideoIngressKind::RtxReinject { repair } => (repair, packet.meta.ssrc),
    };
    Some(XbxEngineVideoRtxReinjectObservation {
        stage: "queued".to_string(),
        primary_ssrc,
        repair_ssrc: repair.native_ssrc,
        sequence_number: packet.meta.sequence_number,
        rtp_timestamp: packet.meta.timestamp,
        pending_queue_len,
        native_sequence_number: Some(repair.native_sequence_number),
        matched_head_gap: false,
        matched_nack_range: false,
        matched_pending_gap: false,
        matched_gap_sequence: None,
        matched_nack_first_sequence: None,
        matched_nack_last_sequence: None,
        observed_at_ms: crate::transport::rtc::stream::video_source::now_ms_f64(),
    })
}

#[cfg(test)]
#[path = "sink.test.rs"]
mod tests;
