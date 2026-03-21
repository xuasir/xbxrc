use std::time::Instant;

use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use rtp::packet::Packet;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::h264::inspection::H264AccessUnitInspector;
use crate::media::video::types::{AssembledVideoFrame, FrameValue, VideoCodec};
use crate::transport::adapter::{FrameSource, FrameSourceEvent};
use crate::transport::rtc::media::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::media::packet_types::{RtcMediaIngressPacket, RtcRtpPacketMeta};
use crate::transport::rtc::media::sink::RtcMediaSink;

#[derive(Clone, Debug)]
struct RtcVideoRtpPacket {
    payload: Vec<u8>,
    meta: RtcRtpPacketMeta,
}

impl RtcVideoRtpPacket {
    fn to_rtp_packet(self) -> Packet {
        Packet {
            header: rtp::header::Header {
                version: 2,
                marker: self.meta.marker,
                payload_type: self.meta.payload_type,
                sequence_number: self.meta.sequence_number,
                timestamp: self.meta.timestamp,
                ssrc: self.meta.ssrc,
                ..Default::default()
            },
            payload: bytes::Bytes::from(self.payload),
        }
    }
}

pub(crate) fn build_rtc_legacy_frame_bridge(
    ingress_capacity: usize,
) -> (Box<dyn RtcMediaSink>, Box<dyn FrameSource>) {
    let (tx, rx) = tokio::sync::mpsc::channel::<RtcVideoRtpPacket>(ingress_capacity.max(256));
    (
        Box::new(RtcLegacyVideoSink { tx }),
        Box::new(RtcLegacyFrameSource::new(rx)),
    )
}

struct RtcLegacyVideoSink {
    tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
}

impl RtcMediaSink for RtcLegacyVideoSink {
    fn on_raw_packet(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        if !matches!(route_label, RtcMediaRouteLabel::PrimaryVideo) {
            return;
        }
        let (Some(meta), Some(payload)) = (rtp_meta, packet.rtp_payload.as_ref()) else {
            return;
        };
        if let Err(err) = self.tx.try_send(RtcVideoRtpPacket {
            payload: payload.clone(),
            meta: meta.clone(),
        }) {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] legacy frame bridge ingress dropped err={}",
                err
            );
        }
    }
}

struct RtcLegacyFrameSource {
    rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
    sample_builder: SampleBuilder<H264Packet>,
    h264_inspector: H264AccessUnitInspector,
    current_width: u32,
    current_height: u32,
}

impl RtcLegacyFrameSource {
    fn new(rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>) -> Self {
        Self {
            rx,
            sample_builder: SampleBuilder::new(512, H264Packet::default(), 90_000)
                .with_max_time_delay(std::time::Duration::from_millis(8)),
            h264_inspector: H264AccessUnitInspector::new(),
            current_width: 0,
            current_height: 0,
        }
    }
}

impl FrameSource for RtcLegacyFrameSource {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>>
    {
        Box::pin(async move {
            loop {
                if let Some(sample) = self.sample_builder.pop() {
                    let payload = sample.data.to_vec();
                    let inspection = match self.h264_inspector.inspect_access_unit(&payload) {
                        Ok(inspection) => inspection,
                        Err(error) => {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc] legacy frame bridge inspection failed err={error}"
                            );
                            continue;
                        }
                    };
                    if !inspection.slice_headers_valid {
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] legacy frame bridge drop invalid slice headers ts={}",
                            sample.packet_timestamp
                        );
                        continue;
                    }
                    if let Some(width) = inspection.width {
                        self.current_width = width;
                    }
                    if let Some(height) = inspection.height {
                        self.current_height = height;
                    }
                    let config_changed = inspection.config_changed;
                    let is_keyframe = inspection.is_idr;
                    let value = FrameValue::new(is_keyframe, config_changed, payload.len());
                    return Some(FrameSourceEvent::Frame(AssembledVideoFrame {
                        codec: VideoCodec::H264,
                        is_keyframe,
                        config_changed,
                        value,
                        width: self.current_width,
                        height: self.current_height,
                        rtp_timestamp: sample.packet_timestamp,
                        assembled_at: Instant::now(),
                        h264: inspection,
                        payload: Bytes::from(payload),
                    }));
                }

                let Some(rtp_packet) = self.rx.recv().await else {
                    crate::xbx_log_info!("[xbxengine][rtc] legacy frame bridge closed");
                    return None;
                };
                self.sample_builder.push(rtp_packet.to_rtp_packet());
            }
        })
    }
}
