use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use interceptor::stream_info::StreamInfo;
use interceptor::Error as InterceptorError;
use interceptor::{
    Attributes, Interceptor, InterceptorBuilder, RTCPReader, RTCPWriter, RTPReader, RTPWriter,
};
use rtp::packet::Packet;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::XbxEngineVideoRtxReinjectObservation;

const MAX_PENDING_REINJECT_PACKETS: usize = 512;
const MAX_RECENT_PACKET_KEYS: usize = 4096;

pub struct RtxReinjectInterceptorBuilder {
    remote_answer_sdp: Arc<Mutex<Option<String>>>,
    runtime_stats: RuntimeStatsSink,
}

impl RtxReinjectInterceptorBuilder {
    pub fn new(
        remote_answer_sdp: Arc<Mutex<Option<String>>>,
        runtime_stats: RuntimeStatsSink,
    ) -> Self {
        Self {
            remote_answer_sdp,
            runtime_stats,
        }
    }
}

impl InterceptorBuilder for RtxReinjectInterceptorBuilder {
    fn build(
        &self,
        _id: &str,
    ) -> std::result::Result<Arc<dyn Interceptor + Send + Sync>, InterceptorError> {
        Ok(Arc::new(RtxReinjectInterceptor {
            remote_answer_sdp: self.remote_answer_sdp.clone(),
            runtime_stats: self.runtime_stats.clone(),
            shared: Arc::new(Mutex::new(RtxReinjectShared::default())),
        }))
    }
}

struct RtxReinjectInterceptor {
    remote_answer_sdp: Arc<Mutex<Option<String>>>,
    runtime_stats: RuntimeStatsSink,
    shared: Arc<Mutex<RtxReinjectShared>>,
}

#[derive(Default)]
struct RtxReinjectShared {
    primary_streams: HashMap<u32, PrimaryStreamState>,
    cached_answer_sdp: Option<String>,
    cached_contracts: Vec<RtxRepairContract>,
}

struct PendingReinjectPacket {
    packet: Packet,
    repair_ssrc: u32,
}

struct PrimaryStreamState {
    pending_packets: VecDeque<PendingReinjectPacket>,
    pending_packet_keys: HashSet<u64>,
    delivered_packet_keys: HashSet<u64>,
    delivered_packet_order: VecDeque<u64>,
    primary_read_poll_count: u64,
    inner_read_count: u64,
}

impl Default for PrimaryStreamState {
    fn default() -> Self {
        Self {
            pending_packets: VecDeque::new(),
            pending_packet_keys: HashSet::new(),
            delivered_packet_keys: HashSet::new(),
            delivered_packet_order: VecDeque::new(),
            primary_read_poll_count: 0,
            inner_read_count: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RtxRepairContract {
    primary_ssrc: u32,
    repair_ssrc: u32,
    primary_payload_type: u8,
    repair_payload_type: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrimaryStreamDescriptor {
    stream_ssrc: u32,
    mime_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepairStreamDescriptor {
    repair_ssrc: u32,
    associated_primary_ssrc: Option<u32>,
}

#[async_trait]
impl Interceptor for RtxReinjectInterceptor {
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync> {
        reader
    }

    async fn bind_rtcp_writer(
        &self,
        writer: Arc<dyn RTCPWriter + Send + Sync>,
    ) -> Arc<dyn RTCPWriter + Send + Sync> {
        writer
    }

    async fn bind_local_stream(
        &self,
        _info: &StreamInfo,
        writer: Arc<dyn RTPWriter + Send + Sync>,
    ) -> Arc<dyn RTPWriter + Send + Sync> {
        writer
    }

    async fn unbind_local_stream(&self, _info: &StreamInfo) {}

    async fn bind_remote_stream(
        &self,
        info: &StreamInfo,
        reader: Arc<dyn RTPReader + Send + Sync>,
    ) -> Arc<dyn RTPReader + Send + Sync> {
        if let Some(primary) = classify_primary_stream(info) {
            if let Ok(mut shared) = self.shared.lock() {
                shared.ensure_primary_stream(primary.stream_ssrc);
            }
            crate::xbx_log_info!(
                "[xbxengine][rtx-reinject] bind primary stream ssrc={} mime={}",
                primary.stream_ssrc,
                primary.mime_type
            );
            return Arc::new(PrimaryStreamRtpReader {
                inner: reader,
                primary_ssrc: primary.stream_ssrc,
                runtime_stats: self.runtime_stats.clone(),
                shared: self.shared.clone(),
            });
        }

        let Some(repair) = classify_repair_stream(info) else {
            return reader;
        };
        crate::xbx_log_info!(
            "[xbxengine][rtx-reinject] bind repair stream repair_ssrc={} assoc_primary_ssrc={:?} mime={} pt={} fmtp={}",
            repair.repair_ssrc,
            repair.associated_primary_ssrc,
            info.mime_type,
            info.payload_type,
            info.sdp_fmtp_line
        );
        Arc::new(RepairStreamRtpReader {
            inner: reader,
            descriptor: repair,
            remote_answer_sdp: self.remote_answer_sdp.clone(),
            runtime_stats: self.runtime_stats.clone(),
            shared: self.shared.clone(),
            reinjected_count: 0.into(),
        })
    }

    async fn unbind_remote_stream(&self, _info: &StreamInfo) {}

    async fn close(&self) -> std::result::Result<(), InterceptorError> {
        Ok(())
    }
}

struct PrimaryStreamRtpReader {
    inner: Arc<dyn RTPReader + Send + Sync>,
    primary_ssrc: u32,
    runtime_stats: RuntimeStatsSink,
    shared: Arc<Mutex<RtxReinjectShared>>,
}

#[async_trait]
impl RTPReader for PrimaryStreamRtpReader {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> std::result::Result<(Packet, Attributes), InterceptorError> {
        loop {
            // 先把 repair stream 回灌进来的包交给主视频流，保持现有 adapter/sample-builder 主链不变。
            if let Ok(mut shared) = self.shared.lock() {
                let pending_queue_len = shared.pending_queue_len(self.primary_ssrc);
                if pending_queue_len > 0 {
                    let poll_count = shared.bump_primary_read_poll_count(self.primary_ssrc);
                    let match_context = self.runtime_stats.read(|stats| RtxReinjectMatchContext {
                        gap_sequence: stats
                            .latest_video_packet_gap
                            .as_ref()
                            .map(|gap| gap.expected_sequence),
                        nack_first_sequence: stats
                            .latest_video_nack_observation
                            .as_ref()
                            .map(|nack| nack.first_sequence),
                        nack_last_sequence: stats
                            .latest_video_nack_observation
                            .as_ref()
                            .map(|nack| nack.last_sequence),
                    });
                    let match_context = match_context.unwrap_or_default();
                    self.runtime_stats.record_video_rtx_reinject(
                        XbxEngineVideoRtxReinjectObservation {
                            stage: "primaryReadPoll".to_string(),
                            primary_ssrc: self.primary_ssrc,
                            repair_ssrc: 0,
                            sequence_number: 0,
                            rtp_timestamp: 0,
                            pending_queue_len,
                            native_sequence_number: None,
                            matched_head_gap: false,
                            matched_nack_range: false,
                            matched_pending_gap: false,
                            matched_gap_sequence: match_context.gap_sequence,
                            matched_nack_first_sequence: match_context.nack_first_sequence,
                            matched_nack_last_sequence: match_context.nack_last_sequence,
                            observed_at_ms: now_ms_f64(),
                        },
                    );
                    if poll_count == 1 || poll_count.is_power_of_two() {
                        crate::xbx_log_warn!(
                            "[xbxengine][rtx-reinject] primaryReadPoll primary_ssrc={} pending={} gap={:?} nack={:?}..{:?} count={}",
                            self.primary_ssrc,
                            pending_queue_len,
                            match_context.gap_sequence,
                            match_context.nack_first_sequence,
                            match_context.nack_last_sequence,
                            poll_count
                        );
                    }
                    if let Some(pending) = shared.pop_pending_packet(self.primary_ssrc) {
                        let matched_gap_sequence = match_context
                            .gap_sequence
                            .filter(|gap| *gap == pending.packet.header.sequence_number);
                        self.runtime_stats.record_video_rtx_reinject(
                            XbxEngineVideoRtxReinjectObservation {
                                stage: "deliveredPrimary".to_string(),
                                primary_ssrc: self.primary_ssrc,
                                repair_ssrc: pending.repair_ssrc,
                                sequence_number: pending.packet.header.sequence_number,
                                rtp_timestamp: pending.packet.header.timestamp,
                                pending_queue_len: pending_queue_len.saturating_sub(1),
                                native_sequence_number: None,
                                matched_head_gap: matched_gap_sequence.is_some(),
                                matched_nack_range: match_context
                                    .matches_nack_range(pending.packet.header.sequence_number),
                                matched_pending_gap: matched_gap_sequence.is_some()
                                    || match_context
                                        .matches_nack_range(pending.packet.header.sequence_number),
                                matched_gap_sequence,
                                matched_nack_first_sequence: match_context.nack_first_sequence,
                                matched_nack_last_sequence: match_context.nack_last_sequence,
                                observed_at_ms: now_ms_f64(),
                            },
                        );
                        return Ok((pending.packet, attributes.clone()));
                    }
                    // pending_queue_len > 0 但无法 pop，说明队列状态和索引已经不一致。
                    self.runtime_stats.record_video_rtx_reinject(
                        XbxEngineVideoRtxReinjectObservation {
                            stage: "pendingInconsistent".to_string(),
                            primary_ssrc: self.primary_ssrc,
                            repair_ssrc: 0,
                            sequence_number: 0,
                            rtp_timestamp: 0,
                            pending_queue_len,
                            native_sequence_number: None,
                            matched_head_gap: false,
                            matched_nack_range: false,
                            matched_pending_gap: false,
                            matched_gap_sequence: match_context.gap_sequence,
                            matched_nack_first_sequence: match_context.nack_first_sequence,
                            matched_nack_last_sequence: match_context.nack_last_sequence,
                            observed_at_ms: now_ms_f64(),
                        },
                    );
                    crate::xbx_log_warn!(
                        "[xbxengine][rtx-reinject] pendingInconsistent primary_ssrc={} pending={} gap={:?} nack={:?}..{:?}",
                        self.primary_ssrc,
                        pending_queue_len,
                        match_context.gap_sequence,
                        match_context.nack_first_sequence,
                        match_context.nack_last_sequence
                    );
                }
            }

            let (packet, next_attributes) = self.inner.read(buf, attributes).await?;
            let pending_queue_len = self
                .shared
                .lock()
                .ok()
                .map(|shared| shared.pending_queue_len(self.primary_ssrc))
                .unwrap_or(0);
            if pending_queue_len > 0 {
                let inner_read_count = self
                    .shared
                    .lock()
                    .ok()
                    .map(|mut shared| shared.bump_inner_read_count(self.primary_ssrc))
                    .unwrap_or(0);
                let match_context = self.runtime_stats.read(|stats| RtxReinjectMatchContext {
                    gap_sequence: stats
                        .latest_video_packet_gap
                        .as_ref()
                        .map(|gap| gap.expected_sequence),
                    nack_first_sequence: stats
                        .latest_video_nack_observation
                        .as_ref()
                        .map(|nack| nack.first_sequence),
                    nack_last_sequence: stats
                        .latest_video_nack_observation
                        .as_ref()
                        .map(|nack| nack.last_sequence),
                });
                let match_context = match_context.unwrap_or_default();
                self.runtime_stats.record_video_rtx_reinject(
                    XbxEngineVideoRtxReinjectObservation {
                        stage: "innerReadPrimary".to_string(),
                        primary_ssrc: self.primary_ssrc,
                        repair_ssrc: 0,
                        sequence_number: packet.header.sequence_number,
                        rtp_timestamp: packet.header.timestamp,
                        pending_queue_len,
                        native_sequence_number: Some(packet.header.sequence_number),
                        matched_head_gap: match_context
                            .gap_sequence
                            .is_some_and(|gap| gap == packet.header.sequence_number),
                        matched_nack_range: match_context
                            .matches_nack_range(packet.header.sequence_number),
                        matched_pending_gap: match_context
                            .gap_sequence
                            .is_some_and(|gap| gap == packet.header.sequence_number)
                            || match_context.matches_nack_range(packet.header.sequence_number),
                        matched_gap_sequence: match_context.gap_sequence,
                        matched_nack_first_sequence: match_context.nack_first_sequence,
                        matched_nack_last_sequence: match_context.nack_last_sequence,
                        observed_at_ms: now_ms_f64(),
                    },
                );
                if inner_read_count == 1 || inner_read_count.is_power_of_two() {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtx-reinject] innerReadPrimary primary_ssrc={} pending={} native_seq={} gap={:?} nack={:?}..{:?} count={}",
                        self.primary_ssrc,
                        pending_queue_len,
                        packet.header.sequence_number,
                        match_context.gap_sequence,
                        match_context.nack_first_sequence,
                        match_context.nack_last_sequence,
                        inner_read_count
                    );
                }
            }
            let packet_key = packet_key(&packet);
            let should_drop_duplicate = self.shared.lock().ok().is_some_and(|mut shared| {
                shared.mark_primary_packet_seen(self.primary_ssrc, packet_key)
            });
            if should_drop_duplicate {
                continue;
            }
            return Ok((packet, next_attributes));
        }
    }
}

struct RepairStreamRtpReader {
    inner: Arc<dyn RTPReader + Send + Sync>,
    descriptor: RepairStreamDescriptor,
    remote_answer_sdp: Arc<Mutex<Option<String>>>,
    runtime_stats: RuntimeStatsSink,
    shared: Arc<Mutex<RtxReinjectShared>>,
    reinjected_count: std::sync::atomic::AtomicU64,
}

#[async_trait]
impl RTPReader for RepairStreamRtpReader {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> std::result::Result<(Packet, Attributes), InterceptorError> {
        let (packet, next_attributes) = self.inner.read(buf, attributes).await?;
        // RTX PoC 只做 unwrap + reinject，不阻断 webrtc-rs 自己的 repair/TWCC 读路径。
        let contract = self.shared.lock().ok().and_then(|mut shared| {
            shared.refresh_contracts_if_needed(&self.remote_answer_sdp);
            shared.resolve_contract(
                self.descriptor.repair_ssrc,
                self.descriptor.associated_primary_ssrc,
            )
        });
        if let Some(contract) = contract {
            if let Some(primary_packet) = unwrap_rtx_packet(&packet, &contract) {
                let packet_key = packet_key(&primary_packet);
                let sequence_number = primary_packet.header.sequence_number;
                let rtp_timestamp = primary_packet.header.timestamp;
                let reinjected = self.shared.lock().ok().is_some_and(|mut shared| {
                    shared.enqueue_pending_packet(
                        contract.primary_ssrc,
                        contract.repair_ssrc,
                        primary_packet,
                    )
                });
                if reinjected {
                    let match_context = self.runtime_stats.read(|stats| RtxReinjectMatchContext {
                        gap_sequence: stats
                            .latest_video_packet_gap
                            .as_ref()
                            .map(|gap| gap.expected_sequence),
                        nack_first_sequence: stats
                            .latest_video_nack_observation
                            .as_ref()
                            .map(|nack| nack.first_sequence),
                        nack_last_sequence: stats
                            .latest_video_nack_observation
                            .as_ref()
                            .map(|nack| nack.last_sequence),
                    });
                    let match_context = match_context.unwrap_or_default();
                    let matched_gap_sequence = match_context
                        .gap_sequence
                        .filter(|gap| *gap == sequence_number);
                    let matched_nack_range = match_context.matches_nack_range(sequence_number);
                    let matched_pending_gap = matched_gap_sequence.is_some() || matched_nack_range;
                    self.runtime_stats.record_video_rtx_reinject(
                        XbxEngineVideoRtxReinjectObservation {
                            stage: "queued".to_string(),
                            primary_ssrc: contract.primary_ssrc,
                            repair_ssrc: contract.repair_ssrc,
                            sequence_number,
                            rtp_timestamp,
                            pending_queue_len: self
                                .shared
                                .lock()
                                .ok()
                                .map(|shared| shared.pending_queue_len(contract.primary_ssrc))
                                .unwrap_or(0),
                            native_sequence_number: None,
                            matched_head_gap: matched_gap_sequence.is_some(),
                            matched_nack_range,
                            matched_pending_gap,
                            matched_gap_sequence,
                            matched_nack_first_sequence: match_context.nack_first_sequence,
                            matched_nack_last_sequence: match_context.nack_last_sequence,
                            observed_at_ms: now_ms_f64(),
                        },
                    );
                    let reinjected_count = self
                        .reinjected_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                        + 1;
                    if reinjected_count == 1 || reinjected_count.is_power_of_two() {
                        crate::xbx_log_warn!(
                            "[xbxengine][rtx-reinject] queued primary packet primary_ssrc={} repair_ssrc={} primary_pt={} repair_pt={} seq={} match={} gap={:?} nack={:?}..{:?} count={} key={}",
                            contract.primary_ssrc,
                            contract.repair_ssrc,
                            contract.primary_payload_type,
                            contract.repair_payload_type,
                            sequence_number,
                            matched_pending_gap,
                            match_context.gap_sequence,
                            match_context.nack_first_sequence,
                            match_context.nack_last_sequence,
                            reinjected_count,
                            packet_key
                        );
                    }
                }
            }
        }
        Ok((packet, next_attributes))
    }
}

impl RtxReinjectShared {
    fn ensure_primary_stream(&mut self, primary_ssrc: u32) {
        self.primary_streams.entry(primary_ssrc).or_default();
    }

    fn enqueue_pending_packet(
        &mut self,
        primary_ssrc: u32,
        repair_ssrc: u32,
        packet: Packet,
    ) -> bool {
        let target_primary_ssrc = self.resolve_enqueue_primary_ssrc(primary_ssrc);
        let stream = self.primary_streams.entry(target_primary_ssrc).or_default();
        let pending_key = packet_key(&packet);
        if stream.pending_packet_keys.contains(&pending_key)
            || stream.delivered_packet_keys.contains(&pending_key)
        {
            return false;
        }
        if stream.pending_packets.len() >= MAX_PENDING_REINJECT_PACKETS {
            if let Some(dropped) = stream.pending_packets.pop_front() {
                stream
                    .pending_packet_keys
                    .remove(&packet_key(&dropped.packet));
            }
        }
        stream.pending_packet_keys.insert(pending_key);
        stream.pending_packets.push_back(PendingReinjectPacket {
            packet,
            repair_ssrc,
        });
        true
    }

    fn resolve_enqueue_primary_ssrc(&self, primary_ssrc: u32) -> u32 {
        if self.primary_streams.contains_key(&primary_ssrc) {
            return primary_ssrc;
        }
        // webrtc-rs 暴露给主流 reader 的 SSRC 可能和 answer/FID 里的 primary SSRC 不完全一致；
        // 单视频流 PoC 下优先退化到唯一主流，避免 repair 包永远堆在无人消费的队列里。
        if self.primary_streams.len() == 1 {
            if let Some(only_primary_ssrc) = self.primary_streams.keys().next().copied() {
                return only_primary_ssrc;
            }
        }
        primary_ssrc
    }

    fn pop_pending_packet(&mut self, primary_ssrc: u32) -> Option<PendingReinjectPacket> {
        let stream = self.primary_streams.get_mut(&primary_ssrc)?;
        while let Some(packet) = stream.pending_packets.pop_front() {
            let packet_key = packet_key(&packet.packet);
            stream.pending_packet_keys.remove(&packet_key);
            // 原生主流已经把同 seq 包送到了上层时，丢掉这类 stale reinject。
            if stream.delivered_packet_keys.contains(&packet_key) {
                continue;
            }
            remember_packet_key(stream, packet_key);
            return Some(packet);
        }
        None
    }

    fn pending_queue_len(&self, primary_ssrc: u32) -> usize {
        self.primary_streams
            .get(&primary_ssrc)
            .map(|stream| stream.pending_packets.len())
            .unwrap_or(0)
    }

    fn bump_primary_read_poll_count(&mut self, primary_ssrc: u32) -> u64 {
        let stream = self.primary_streams.entry(primary_ssrc).or_default();
        stream.primary_read_poll_count = stream.primary_read_poll_count.saturating_add(1);
        stream.primary_read_poll_count
    }

    fn bump_inner_read_count(&mut self, primary_ssrc: u32) -> u64 {
        let stream = self.primary_streams.entry(primary_ssrc).or_default();
        stream.inner_read_count = stream.inner_read_count.saturating_add(1);
        stream.inner_read_count
    }

    fn mark_primary_packet_seen(&mut self, primary_ssrc: u32, packet_key: u64) -> bool {
        let stream = self.primary_streams.entry(primary_ssrc).or_default();
        if stream.delivered_packet_keys.contains(&packet_key) {
            return true;
        }
        if stream.pending_packet_keys.remove(&packet_key) {
            remove_pending_packet_by_key(stream, packet_key);
        }
        remember_packet_key(stream, packet_key);
        false
    }

    fn refresh_contracts_if_needed(&mut self, remote_answer_sdp: &Arc<Mutex<Option<String>>>) {
        let latest_answer_sdp = remote_answer_sdp
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if self.cached_answer_sdp == latest_answer_sdp {
            return;
        }
        self.cached_contracts = latest_answer_sdp
            .as_deref()
            .map(parse_rtx_repair_contracts)
            .unwrap_or_default();
        self.cached_answer_sdp = latest_answer_sdp;
    }

    fn resolve_contract(
        &self,
        repair_ssrc: u32,
        associated_primary_ssrc: Option<u32>,
    ) -> Option<RtxRepairContract> {
        self.cached_contracts
            .iter()
            .find(|contract| {
                contract.repair_ssrc == repair_ssrc
                    || associated_primary_ssrc
                        .is_some_and(|primary_ssrc| contract.primary_ssrc == primary_ssrc)
            })
            .cloned()
    }
}

fn remember_packet_key(stream: &mut PrimaryStreamState, packet_key: u64) {
    if stream.delivered_packet_keys.insert(packet_key) {
        stream.delivered_packet_order.push_back(packet_key);
        if stream.delivered_packet_order.len() > MAX_RECENT_PACKET_KEYS {
            if let Some(oldest) = stream.delivered_packet_order.pop_front() {
                stream.delivered_packet_keys.remove(&oldest);
            }
        }
    }
}

fn remove_pending_packet_by_key(stream: &mut PrimaryStreamState, target_packet_key: u64) {
    if let Some(index) = stream
        .pending_packets
        .iter()
        .position(|pending| packet_key(&pending.packet) == target_packet_key)
    {
        stream.pending_packets.remove(index);
    }
}

fn classify_primary_stream(info: &StreamInfo) -> Option<PrimaryStreamDescriptor> {
    let mime_type = info.mime_type.trim().to_ascii_lowercase();
    if mime_type == "video/h264" && info.associated_stream.is_none() {
        return Some(PrimaryStreamDescriptor {
            stream_ssrc: info.ssrc,
            mime_type,
        });
    }
    None
}

fn classify_repair_stream(info: &StreamInfo) -> Option<RepairStreamDescriptor> {
    let mime_type = info.mime_type.trim().to_ascii_lowercase();
    let looks_like_repair = matches!(
        mime_type.as_str(),
        "video/rtx" | "video/red" | "video/ulpfec" | "video/flexfec-03"
    );
    if !looks_like_repair && info.associated_stream.is_none() {
        return None;
    }
    Some(RepairStreamDescriptor {
        repair_ssrc: info.ssrc,
        associated_primary_ssrc: info.associated_stream.as_ref().map(|stream| stream.ssrc),
    })
}

fn parse_rtx_repair_contracts(answer_sdp: &str) -> Vec<RtxRepairContract> {
    let mut in_video_media = false;
    let mut repair_payload_type: Option<u8> = None;
    let mut primary_payload_type: Option<u8> = None;
    let mut fid_pairs: Vec<(u32, u32)> = Vec::new();

    for line in answer_sdp.split("\r\n") {
        if let Some(media) = line.strip_prefix("m=") {
            in_video_media = media.starts_with("video ");
            continue;
        }
        if !in_video_media {
            continue;
        }
        if let Some(value) = line.strip_prefix("a=fmtp:") {
            if let Some((payload_type_text, fmtp)) = value.split_once(' ') {
                if let Ok(candidate_repair_pt) = payload_type_text.parse::<u8>() {
                    for kv in fmtp.split(';') {
                        let kv = kv.trim();
                        if let Some(apt_text) = kv.strip_prefix("apt=") {
                            if let Ok(candidate_primary_pt) = apt_text.parse::<u8>() {
                                repair_payload_type = Some(candidate_repair_pt);
                                primary_payload_type = Some(candidate_primary_pt);
                            }
                        }
                    }
                }
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("a=ssrc-group:FID ") {
            let mut values = value.split_whitespace();
            let primary_ssrc = values.next().and_then(|text| text.parse::<u32>().ok());
            let repair_ssrc = values.next().and_then(|text| text.parse::<u32>().ok());
            if let (Some(primary_ssrc), Some(repair_ssrc)) = (primary_ssrc, repair_ssrc) {
                fid_pairs.push((primary_ssrc, repair_ssrc));
            }
        }
    }

    let Some(repair_payload_type) = repair_payload_type else {
        return Vec::new();
    };
    let Some(primary_payload_type) = primary_payload_type else {
        return Vec::new();
    };
    fid_pairs
        .into_iter()
        .map(|(primary_ssrc, repair_ssrc)| RtxRepairContract {
            primary_ssrc,
            repair_ssrc,
            primary_payload_type,
            repair_payload_type,
        })
        .collect()
}

fn unwrap_rtx_packet(repair_packet: &Packet, contract: &RtxRepairContract) -> Option<Packet> {
    if repair_packet.payload.len() < 2 {
        return None;
    }
    let mut primary_packet = repair_packet.clone();
    primary_packet.header.ssrc = contract.primary_ssrc;
    primary_packet.header.payload_type = contract.primary_payload_type;
    primary_packet.header.sequence_number =
        u16::from_be_bytes([repair_packet.payload[0], repair_packet.payload[1]]);
    primary_packet.payload = repair_packet.payload[2..].to_vec().into();
    Some(primary_packet)
}

fn packet_key(packet: &Packet) -> u64 {
    ((packet.header.timestamp as u64) << 16) | u64::from(packet.header.sequence_number)
}

#[derive(Clone, Copy, Debug, Default)]
struct RtxReinjectMatchContext {
    gap_sequence: Option<u16>,
    nack_first_sequence: Option<u16>,
    nack_last_sequence: Option<u16>,
}

impl RtxReinjectMatchContext {
    fn matches_nack_range(&self, sequence_number: u16) -> bool {
        match (self.nack_first_sequence, self.nack_last_sequence) {
            (Some(first), Some(last)) => sequence_number >= first && sequence_number <= last,
            _ => false,
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::{parse_rtx_repair_contracts, unwrap_rtx_packet, RtxRepairContract};
    use bytes::Bytes;
    use rtp::header::Header;
    use rtp::packet::Packet;

    #[test]
    fn parse_rtx_repair_contracts_extracts_fid_and_apt_mapping() {
        let answer_sdp = concat!(
            "v=0\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124 97 122 116\r\n",
            "a=rtpmap:97 rtx/90000\r\n",
            "a=fmtp:97 apt=124\r\n",
            "a=rtpmap:122 red/90000\r\n",
            "a=rtpmap:116 ulpfec/90000\r\n",
            "a=ssrc-group:FID 3300031859 1343689466\r\n"
        );
        let contracts = parse_rtx_repair_contracts(answer_sdp);
        assert_eq!(
            contracts,
            vec![RtxRepairContract {
                primary_ssrc: 3300031859,
                repair_ssrc: 1343689466,
                primary_payload_type: 124,
                repair_payload_type: 97,
            }]
        );
    }

    #[test]
    fn unwrap_rtx_packet_restores_primary_header_and_payload() {
        let repair_packet = Packet {
            header: Header {
                payload_type: 97,
                sequence_number: 3000,
                timestamp: 4444,
                ssrc: 200,
                ..Default::default()
            },
            payload: Bytes::from_static(&[0x01, 0x02, 0xaa, 0xbb, 0xcc]),
        };
        let contract = RtxRepairContract {
            primary_ssrc: 100,
            repair_ssrc: 200,
            primary_payload_type: 124,
            repair_payload_type: 97,
        };
        let primary_packet = unwrap_rtx_packet(&repair_packet, &contract).expect("primary packet");
        assert_eq!(primary_packet.header.ssrc, 100);
        assert_eq!(primary_packet.header.payload_type, 124);
        assert_eq!(primary_packet.header.sequence_number, 0x0102);
        assert_eq!(primary_packet.header.timestamp, 4444);
        assert_eq!(
            primary_packet.payload,
            Bytes::from_static(&[0xaa, 0xbb, 0xcc])
        );
    }
}
