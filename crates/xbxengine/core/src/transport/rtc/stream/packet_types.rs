#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaPacketKind {
    Rtp,
    Rtcp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RtcMediaStreamIdentity {
    pub(crate) track_id: Option<String>,
    pub(crate) ssrc: Option<u32>,
    pub(crate) mid: Option<String>,
    pub(crate) rid: Option<String>,
}

impl RtcMediaStreamIdentity {
    pub(crate) fn from_source(source: &RtcMediaPacketSource) -> Self {
        match source {
            RtcMediaPacketSource::Unknown => Self::default(),
            RtcMediaPacketSource::Track { track_id } => Self {
                track_id: normalize_identity_token(track_id),
                ..Self::default()
            },
        }
    }

    pub(crate) fn with_rtp_meta(mut self, rtp_meta: Option<&RtcRtpPacketMeta>) -> Self {
        if let Some(meta) = rtp_meta {
            self.ssrc = Some(meta.ssrc);
        }
        self
    }

    pub(crate) fn track_hint(&self) -> Option<&str> {
        self.track_id.as_deref()
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "track_id={} ssrc={} mid={} rid={}",
            self.track_id.as_deref().unwrap_or("-"),
            self.ssrc
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            self.mid.as_deref().unwrap_or("-"),
            self.rid.as_deref().unwrap_or("-")
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RtcMediaIngressPacket {
    pub(crate) kind: MediaPacketKind,
    pub(crate) byte_len: usize,
    pub(crate) source: RtcMediaPacketSource,
    pub(crate) stream_identity: RtcMediaStreamIdentity,
    pub(crate) rtp_payload: Option<Vec<u8>>,
}

impl RtcMediaIngressPacket {
    pub(crate) fn new(
        kind: MediaPacketKind,
        byte_len: usize,
        source: RtcMediaPacketSource,
    ) -> Self {
        let stream_identity = RtcMediaStreamIdentity::from_source(&source);
        Self {
            kind,
            byte_len,
            source,
            stream_identity,
            rtp_payload: None,
        }
    }

    pub(crate) fn with_rtp_meta(mut self, rtp_meta: Option<&RtcRtpPacketMeta>) -> Self {
        self.stream_identity = self.stream_identity.with_rtp_meta(rtp_meta);
        self
    }

    pub(crate) fn with_rtp_payload(mut self, payload: Vec<u8>) -> Self {
        self.rtp_payload = Some(payload);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RtcMediaPacketSource {
    Unknown,
    Track { track_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RtcRtpPacketMeta {
    pub(crate) ssrc: u32,
    pub(crate) payload_type: u8,
    pub(crate) sequence_number: u16,
    pub(crate) timestamp: u32,
    pub(crate) marker: bool,
}

fn normalize_identity_token(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtcVideoRepairMetadata {
    pub(crate) native_ssrc: u32,
    pub(crate) native_payload_type: u8,
    pub(crate) native_sequence_number: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RtcVideoIngressKind {
    Primary,
    RepairPrimaryPassThrough {
        repair: RtcVideoRepairMetadata,
    },
    RtxReinject {
        repair: RtcVideoRepairMetadata,
    },
}

impl Default for RtcVideoIngressKind {
    fn default() -> Self {
        Self::Primary
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RtcVideoRtpPacket {
    pub(crate) payload: Vec<u8>,
    pub(crate) meta: RtcRtpPacketMeta,
    pub(crate) ingress_kind: RtcVideoIngressKind,
}

impl RtcVideoRtpPacket {
    pub(crate) fn to_rtp_packet(self) -> rtc_rtp::packet::Packet {
        rtc_rtp::packet::Packet {
            header: rtc_rtp::header::Header {
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

#[derive(Clone, Debug)]
pub(crate) struct RtcAudioRtpPacket {
    pub(crate) payload: Vec<u8>,
}
