use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use async_trait::async_trait;
use interceptor::stream_info::StreamInfo;
use interceptor::twcc::Recorder;
use interceptor::Error as InterceptorError;
use interceptor::{
    Attributes, Interceptor, InterceptorBuilder, RTCPReader, RTCPWriter, RTPReader, RTPWriter,
};
use rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
use rtp::extension::transport_cc_extension::TransportCcExtension;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant, MissedTickBehavior};
use webrtc_util::Unmarshal;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};

const TRANSPORT_CC_URI: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

#[derive(Clone, Debug)]
struct TwccObservedPacket {
    sequence_number: u16,
    arrival_time_us: i64,
    ssrc: u32,
    payload_size_bytes: u32,
}

pub struct OwnedTwccReceiverBuilder {
    interval: Duration,
    runtime_stats: RuntimeStatsSink,
}

impl OwnedTwccReceiverBuilder {
    pub fn new(
        interval: Duration,
        runtime_stats: Arc<StdMutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        Self {
            interval,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
        }
    }
}

impl InterceptorBuilder for OwnedTwccReceiverBuilder {
    fn build(
        &self,
        _id: &str,
    ) -> std::result::Result<Arc<dyn Interceptor + Send + Sync>, InterceptorError> {
        let (packet_tx, packet_rx) = mpsc::channel(256);
        let (close_tx, close_rx) = mpsc::channel(1);
        Ok(Arc::new(OwnedTwccReceiver {
            interval: self.interval,
            runtime_stats: self.runtime_stats.clone(),
            observation_counter: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
            packet_tx,
            packet_rx: Mutex::new(Some(packet_rx)),
            close_tx: Mutex::new(Some(close_tx)),
            close_rx: Mutex::new(Some(close_rx)),
            worker: Mutex::new(None),
        }))
    }
}

// 自定义 TWCC receiver：保留 transport-cc 能力，但 feedback 的生成节奏与实现由我们接管。
pub struct OwnedTwccReceiver {
    interval: Duration,
    runtime_stats: RuntimeStatsSink,
    observation_counter: Arc<AtomicU64>,
    start_time: Instant,
    packet_tx: mpsc::Sender<TwccObservedPacket>,
    packet_rx: Mutex<Option<mpsc::Receiver<TwccObservedPacket>>>,
    close_tx: Mutex<Option<mpsc::Sender<()>>>,
    close_rx: Mutex<Option<mpsc::Receiver<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl OwnedTwccReceiver {
    async fn ensure_worker(
        &self,
        rtcp_writer: Arc<dyn RTCPWriter + Send + Sync>,
    ) -> std::result::Result<(), InterceptorError> {
        let mut worker = self.worker.lock().await;
        if worker.is_some() {
            return Ok(());
        }

        let mut packet_rx = self
            .packet_rx
            .lock()
            .await
            .take()
            .ok_or(InterceptorError::ErrInvalidPacketRx)?;
        let mut close_rx = self
            .close_rx
            .lock()
            .await
            .take()
            .ok_or(InterceptorError::ErrInvalidCloseRx)?;
        let interval = self.interval;
        let runtime_stats = self.runtime_stats.clone();
        let observation_counter = self.observation_counter.clone();

        *worker = Some(tokio::spawn(async move {
            let mut recorder = Recorder::new(1);
            let attributes = Attributes::new();
            let mut ticker = time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut observed_packet_count = 0u16;
            let mut observed_byte_count = 0u64;
            let mut first_arrival_time_us: Option<i64> = None;
            let mut last_arrival_time_us: Option<i64> = None;
            let mut last_feedback_at_ms: Option<f64> = None;

            loop {
                tokio::select! {
                    _ = close_rx.recv() => {
                        break;
                    }
                    packet = packet_rx.recv() => {
                        let Some(packet) = packet else {
                            break;
                        };
                        recorder.record(packet.ssrc, packet.sequence_number, packet.arrival_time_us);
                        observed_packet_count = observed_packet_count.saturating_add(1);
                        observed_byte_count = observed_byte_count
                            .saturating_add(u64::from(packet.payload_size_bytes));
                        first_arrival_time_us = Some(
                            first_arrival_time_us
                                .map(|current| current.min(packet.arrival_time_us))
                                .unwrap_or(packet.arrival_time_us),
                        );
                        last_arrival_time_us = Some(
                            last_arrival_time_us
                                .map(|current| current.max(packet.arrival_time_us))
                                .unwrap_or(packet.arrival_time_us),
                        );
                    }
                    _ = ticker.tick() => {
                        let packets = recorder.build_feedback_packet();
                        if packets.is_empty() {
                            continue;
                        }
                        let twcc_summary = summarize_twcc_feedback(&packets, observed_packet_count);
                        let observed_at_ms = now_ms_f64();
                        if let Err(error) = rtcp_writer.write(&packets, &attributes).await {
                            crate::xbx_log_debug!(
                                "[xbxengine][twcc] owned receiver write failed: {error}"
                            );
                        } else if let Some((feedback_packet_count, covered_sequence_start, covered_sequence_end)) = twcc_summary {
                            let feedback_interval_ms =
                                last_feedback_at_ms.map(|last_at_ms| (observed_at_ms - last_at_ms).max(0.0));
                            let covered_sequence_span = covered_sequence_end
                                .wrapping_sub(covered_sequence_start)
                                .saturating_add(1);
                            let arrival_span_ms = first_arrival_time_us
                                .zip(last_arrival_time_us)
                                .map(|(first, last)| ((last - first).max(0) as f64) / 1_000.0);
                            let receive_bitrate_kbps = feedback_interval_ms
                                .filter(|interval_ms| *interval_ms > 0.0)
                                .map(|interval_ms| (observed_byte_count * 8) as f64 / interval_ms);
                            let delivery_ratio = if covered_sequence_span > 0 {
                                (observed_packet_count as f64 / covered_sequence_span as f64)
                                    .clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            let packet_loss_ratio = (1.0 - delivery_ratio).clamp(0.0, 1.0);
                            runtime_stats.record_latest_video_twcc_observation(
                                XbxEngineVideoTwccObservation {
                                    observation_id: observation_counter
                                        .fetch_add(1, Ordering::SeqCst)
                                        + 1,
                                    feedback_packet_count,
                                    covered_sequence_start,
                                    covered_sequence_end,
                                    covered_sequence_span,
                                    observed_packet_count,
                                    observed_byte_count,
                                    feedback_interval_ms,
                                    arrival_span_ms,
                                    receive_bitrate_kbps,
                                    delivery_ratio,
                                    packet_loss_ratio,
                                    observed_at_ms,
                                },
                            );
                            last_feedback_at_ms = Some(observed_at_ms);
                        }
                        observed_packet_count = 0;
                        observed_byte_count = 0;
                        first_arrival_time_us = None;
                        last_arrival_time_us = None;
                    }
                }
            }
        }));

        Ok(())
    }
}

fn summarize_twcc_feedback(
    packets: &[Box<dyn rtcp::packet::Packet + Send + Sync>],
    observed_packet_count: u16,
) -> Option<(u16, u16, u16)> {
    let mut feedback_packet_count = 0u16;
    let mut covered_sequence_start = None;
    let mut covered_sequence_end = None;

    for packet in packets {
        let Some(twcc) = packet.as_any().downcast_ref::<TransportLayerCc>() else {
            continue;
        };
        feedback_packet_count = feedback_packet_count.saturating_add(1);
        covered_sequence_start = Some(
            covered_sequence_start
                .map(|current: u16| current.min(twcc.base_sequence_number))
                .unwrap_or(twcc.base_sequence_number),
        );
        let end = twcc
            .base_sequence_number
            .wrapping_add(twcc.packet_status_count.saturating_sub(1));
        covered_sequence_end = Some(
            covered_sequence_end
                .map(|current: u16| current.max(end))
                .unwrap_or(end),
        );
    }

    match (
        feedback_packet_count,
        covered_sequence_start,
        covered_sequence_end,
    ) {
        (count, Some(start), Some(end)) if count > 0 || observed_packet_count > 0 => {
            Some((count, start, end))
        }
        _ => None,
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[async_trait]
impl Interceptor for OwnedTwccReceiver {
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
        if let Err(error) = self.ensure_worker(writer.clone()).await {
            crate::xbx_log_warn!("[xbxengine][twcc] owned receiver worker init failed: {error}");
        }
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
        let Some(hdr_ext_id) = info
            .rtp_header_extensions
            .iter()
            .find(|extension| extension.uri == TRANSPORT_CC_URI)
            .map(|extension| extension.id as u8)
            .filter(|value| *value > 0)
        else {
            return reader;
        };

        Arc::new(OwnedTwccReceiverStream {
            reader,
            hdr_ext_id,
            ssrc: info.ssrc,
            packet_tx: self.packet_tx.clone(),
            start_time: self.start_time,
        })
    }

    async fn unbind_remote_stream(&self, _info: &StreamInfo) {}

    async fn close(&self) -> std::result::Result<(), InterceptorError> {
        if let Some(close_tx) = self.close_tx.lock().await.take() {
            let _ = close_tx.send(()).await;
        }
        if let Some(worker) = self.worker.lock().await.take() {
            let _ = worker.await;
        }
        Ok(())
    }
}

struct OwnedTwccReceiverStream {
    reader: Arc<dyn RTPReader + Send + Sync>,
    hdr_ext_id: u8,
    ssrc: u32,
    packet_tx: mpsc::Sender<TwccObservedPacket>,
    start_time: Instant,
}

#[async_trait]
impl RTPReader for OwnedTwccReceiverStream {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> std::result::Result<(rtp::packet::Packet, Attributes), InterceptorError> {
        let (packet, attributes) = self.reader.read(buf, attributes).await?;

        if let Some(mut extension) = packet.header.get_extension(self.hdr_ext_id) {
            let transport_cc = TransportCcExtension::unmarshal(&mut extension)?;
            let _ = self
                .packet_tx
                .send(TwccObservedPacket {
                    sequence_number: transport_cc.transport_sequence,
                    arrival_time_us: self.start_time.elapsed().as_micros() as i64,
                    ssrc: self.ssrc,
                    payload_size_bytes: packet.payload.len().min(u32::MAX as usize) as u32,
                })
                .await;
        }

        Ok((packet, attributes))
    }
}
