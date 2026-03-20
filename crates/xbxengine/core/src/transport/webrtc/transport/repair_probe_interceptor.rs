use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use interceptor::stream_info::StreamInfo;
use interceptor::Error as InterceptorError;
use interceptor::{
    Attributes, Interceptor, InterceptorBuilder, RTCPReader, RTCPWriter, RTPReader, RTPWriter,
};

use crate::{runtime_stats_sink::RuntimeStatsSink, XbxEngineVideoRepairProbeObservation};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepairStreamDescriptor {
    classification: RepairStreamClassification,
    stream_id: String,
    stream_ssrc: u32,
    mime_type: String,
    payload_type: u8,
    clock_rate: u32,
    associated_ssrc: Option<u32>,
    associated_payload_type: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RepairStreamClassification {
    RepairMime,
    AssociatedStream,
    RepairMimeAndAssociatedStream,
}

impl RepairStreamClassification {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RepairMime => "repair-mime",
            Self::AssociatedStream => "associated-stream",
            Self::RepairMimeAndAssociatedStream => "repair-mime+associated-stream",
        }
    }
}

pub struct RepairProbeInterceptorBuilder {
    runtime_stats: RuntimeStatsSink,
}

impl RepairProbeInterceptorBuilder {
    pub fn new(runtime_stats: RuntimeStatsSink) -> Self {
        Self { runtime_stats }
    }
}

impl InterceptorBuilder for RepairProbeInterceptorBuilder {
    fn build(
        &self,
        _id: &str,
    ) -> std::result::Result<Arc<dyn Interceptor + Send + Sync>, InterceptorError> {
        Ok(Arc::new(RepairProbeInterceptor {
            runtime_stats: self.runtime_stats.clone(),
        }))
    }
}

struct RepairProbeInterceptor {
    runtime_stats: RuntimeStatsSink,
}

#[async_trait]
impl Interceptor for RepairProbeInterceptor {
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
        let Some(descriptor) = classify_repair_stream(info) else {
            return reader;
        };

        crate::xbx_log_warn!(
            "[xbxengine][repair-probe] bind remote repair stream class={} id={} ssrc={} mime={} pt={} clock={} assoc_ssrc={:?} assoc_pt={:?} fmtp={}",
            descriptor.classification.as_str(),
            descriptor.stream_id,
            descriptor.stream_ssrc,
            descriptor.mime_type,
            descriptor.payload_type,
            descriptor.clock_rate,
            descriptor.associated_ssrc,
            descriptor.associated_payload_type,
            info.sdp_fmtp_line
        );
        self.runtime_stats.record_video_repair_probe(
            XbxEngineVideoRepairProbeObservation {
                observation_id: 0,
                phase: "bind".to_string(),
                classification: descriptor.classification.as_str().to_string(),
                stream_id: descriptor.stream_id.clone(),
                stream_ssrc: descriptor.stream_ssrc,
                mime_type: descriptor.mime_type.clone(),
                payload_type: descriptor.payload_type,
                clock_rate: descriptor.clock_rate,
                associated_ssrc: descriptor.associated_ssrc,
                associated_payload_type: descriptor.associated_payload_type,
                stream_packet_count: 0,
                observed_at_ms: now_ms(),
            },
            false,
        );

        Arc::new(RepairProbeRtpReader {
            inner: reader,
            descriptor,
            runtime_stats: self.runtime_stats.clone(),
            packet_count: std::sync::atomic::AtomicU64::new(0),
        })
    }

    async fn unbind_remote_stream(&self, info: &StreamInfo) {
        if let Some(descriptor) = classify_repair_stream(info) {
            crate::xbx_log_warn!(
                "[xbxengine][repair-probe] unbind remote repair stream class={} mime={} pt={} assoc_ssrc={:?} assoc_pt={:?}",
                descriptor.classification.as_str(),
                descriptor.mime_type,
                descriptor.payload_type,
                descriptor.associated_ssrc,
                descriptor.associated_payload_type
            );
        }
    }

    async fn close(&self) -> std::result::Result<(), InterceptorError> {
        Ok(())
    }
}

struct RepairProbeRtpReader {
    inner: Arc<dyn RTPReader + Send + Sync>,
    descriptor: RepairStreamDescriptor,
    runtime_stats: RuntimeStatsSink,
    packet_count: std::sync::atomic::AtomicU64,
}

#[async_trait]
impl RTPReader for RepairProbeRtpReader {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> std::result::Result<(rtp::packet::Packet, Attributes), InterceptorError> {
        let (packet, attributes) = self.inner.read(buf, attributes).await?;
        let packet_count = self
            .packet_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if packet_count == 1 || packet_count.is_power_of_two() {
            crate::xbx_log_warn!(
                "[xbxengine][repair-probe] observed repair packet class={} id={} ssrc={} mime={} pt={} clock={} count={} seq={} ts={} marker={}",
                self.descriptor.classification.as_str(),
                self.descriptor.stream_id,
                self.descriptor.stream_ssrc,
                self.descriptor.mime_type,
                self.descriptor.payload_type,
                self.descriptor.clock_rate,
                packet_count,
                packet.header.sequence_number,
                packet.header.timestamp,
                packet.header.marker
            );
        }
        self.runtime_stats.record_video_repair_probe(
            XbxEngineVideoRepairProbeObservation {
                observation_id: 0,
                phase: "packet".to_string(),
                classification: self.descriptor.classification.as_str().to_string(),
                stream_id: self.descriptor.stream_id.clone(),
                stream_ssrc: self.descriptor.stream_ssrc,
                mime_type: self.descriptor.mime_type.clone(),
                payload_type: self.descriptor.payload_type,
                clock_rate: self.descriptor.clock_rate,
                associated_ssrc: self.descriptor.associated_ssrc,
                associated_payload_type: self.descriptor.associated_payload_type,
                stream_packet_count: packet_count,
                observed_at_ms: now_ms(),
            },
            packet_count == 1,
        );
        Ok((packet, attributes))
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn classify_repair_stream(info: &StreamInfo) -> Option<RepairStreamDescriptor> {
    let mime_type = info.mime_type.trim().to_ascii_lowercase();
    let looks_like_repair_mime = matches!(
        mime_type.as_str(),
        "video/rtx" | "video/red" | "video/ulpfec" | "video/flexfec-03"
    );
    let has_associated_stream = info.associated_stream.is_some();
    if !looks_like_repair_mime && !has_associated_stream {
        return None;
    }
    let classification = match (looks_like_repair_mime, has_associated_stream) {
        (true, true) => RepairStreamClassification::RepairMimeAndAssociatedStream,
        (true, false) => RepairStreamClassification::RepairMime,
        (false, true) => RepairStreamClassification::AssociatedStream,
        (false, false) => unreachable!("repair stream guard should have returned early"),
    };
    Some(RepairStreamDescriptor {
        classification,
        stream_id: info.id.clone(),
        stream_ssrc: info.ssrc,
        mime_type,
        payload_type: info.payload_type,
        clock_rate: info.clock_rate,
        associated_ssrc: info.associated_stream.as_ref().map(|stream| stream.ssrc),
        associated_payload_type: info
            .associated_stream
            .as_ref()
            .map(|stream| stream.payload_type),
    })
}

#[cfg(test)]
mod tests {
    use super::{classify_repair_stream, RepairStreamClassification};
    use interceptor::stream_info::{AssociatedStreamInfo, StreamInfo};

    #[test]
    fn classify_repair_stream_detects_rtx_with_associated_stream() {
        let info = StreamInfo {
            id: "rtx-1".to_string(),
            mime_type: "video/rtx".to_string(),
            payload_type: 97,
            ssrc: 11,
            clock_rate: 90_000,
            associated_stream: Some(AssociatedStreamInfo {
                ssrc: 42,
                payload_type: 124,
            }),
            ..Default::default()
        };
        let descriptor = classify_repair_stream(&info).expect("repair stream");
        assert_eq!(
            descriptor.classification,
            RepairStreamClassification::RepairMimeAndAssociatedStream
        );
        assert_eq!(descriptor.mime_type, "video/rtx");
        assert_eq!(descriptor.stream_id, "rtx-1");
        assert_eq!(descriptor.stream_ssrc, 11);
        assert_eq!(descriptor.payload_type, 97);
        assert_eq!(descriptor.clock_rate, 90_000);
        assert_eq!(descriptor.associated_ssrc, Some(42));
        assert_eq!(descriptor.associated_payload_type, Some(124));
    }

    #[test]
    fn classify_repair_stream_rejects_primary_h264_stream() {
        let info = StreamInfo {
            id: "video-1".to_string(),
            mime_type: "video/h264".to_string(),
            payload_type: 124,
            ssrc: 7,
            clock_rate: 90_000,
            ..Default::default()
        };
        assert!(classify_repair_stream(&info).is_none());
    }

    #[test]
    fn classify_repair_stream_detects_associated_stream_even_with_primary_like_mime() {
        let info = StreamInfo {
            id: "repair-aux".to_string(),
            mime_type: "video/h264".to_string(),
            payload_type: 0,
            ssrc: 21,
            clock_rate: 90_000,
            associated_stream: Some(AssociatedStreamInfo {
                ssrc: 99,
                payload_type: 124,
            }),
            ..Default::default()
        };
        let descriptor = classify_repair_stream(&info).expect("repair stream");
        assert_eq!(
            descriptor.classification,
            RepairStreamClassification::AssociatedStream
        );
        assert_eq!(descriptor.mime_type, "video/h264");
        assert_eq!(descriptor.stream_id, "repair-aux");
        assert_eq!(descriptor.stream_ssrc, 21);
        assert_eq!(descriptor.associated_ssrc, Some(99));
        assert_eq!(descriptor.associated_payload_type, Some(124));
    }
}
