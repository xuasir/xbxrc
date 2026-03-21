use std::collections::{HashMap, HashSet};

use xbxengine_protocol::{XbxEngineIceCandidateDto, XbxEngineTargetTypeDto};

use crate::transport::rtc::stats::now_ms_f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RtcIceCandidateKind {
    Host,
    Srflx,
    Relay,
    Unknown,
}

impl RtcIceCandidateKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Srflx => "srflx",
            Self::Relay => "relay",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RtcConnectionRuntimeState {
    pub(crate) local_candidates: Vec<XbxEngineIceCandidateDto>,
    pub(crate) local_candidate_keys: HashSet<String>,
    pub(crate) local_candidate_count_total: u64,
    pub(crate) local_candidate_host_count: u64,
    pub(crate) local_candidate_srflx_count: u64,
    pub(crate) local_candidate_relay_count: u64,
    pub(crate) local_candidate_unknown_count: u64,
    pub(crate) local_candidate_end_of_candidates_count: u64,
    pub(crate) latest_local_candidate_kind: Option<RtcIceCandidateKind>,
    pub(crate) latest_local_candidate_key: Option<String>,
    pub(crate) local_candidate_first_observed_at_ms: Option<f64>,
    pub(crate) local_candidate_last_observed_at_ms: Option<f64>,
    pub(crate) remote_candidates: Vec<XbxEngineIceCandidateDto>,
    pub(crate) pending_remote_candidates: Vec<XbxEngineIceCandidateDto>,
    pub(crate) remote_candidate_keys: HashSet<String>,
    pub(crate) pending_remote_candidate_keys: HashSet<String>,
    pub(crate) applied_remote_candidate_keys: HashSet<String>,
    pub(crate) remote_ice_gathering_complete: bool,
    pub(crate) remote_candidate_count_total: u64,
    pub(crate) remote_candidate_host_count: u64,
    pub(crate) remote_candidate_srflx_count: u64,
    pub(crate) remote_candidate_relay_count: u64,
    pub(crate) remote_candidate_unknown_count: u64,
    pub(crate) latest_remote_candidate_kind: Option<RtcIceCandidateKind>,
    pub(crate) latest_remote_candidate_key: Option<String>,
    pub(crate) data_channel_labels: HashMap<u16, String>,
    pub(crate) input_channel_open: bool,
    pub(crate) input_metadata_bootstrapped: bool,
    pub(crate) input_metadata_bootstrapped_after_handshake: bool,
    pub(crate) input_backpressure_high: bool,
    pub(crate) chat_channel_open: bool,
    pub(crate) local_ice_gathering_complete: bool,
    pub(crate) local_offer_sdp: Option<String>,
    pub(crate) remote_answer_sdp: Option<String>,
    pub(crate) session_target_type: Option<XbxEngineTargetTypeDto>,
}

impl RtcConnectionRuntimeState {
    pub(crate) fn reset_for_session(&mut self, session_target_type: XbxEngineTargetTypeDto) {
        self.local_candidates.clear();
        self.local_candidate_keys.clear();
        self.local_candidate_count_total = 0;
        self.local_candidate_host_count = 0;
        self.local_candidate_srflx_count = 0;
        self.local_candidate_relay_count = 0;
        self.local_candidate_unknown_count = 0;
        self.local_candidate_end_of_candidates_count = 0;
        self.latest_local_candidate_kind = None;
        self.latest_local_candidate_key = None;
        self.local_candidate_first_observed_at_ms = None;
        self.local_candidate_last_observed_at_ms = None;
        self.remote_candidates.clear();
        self.pending_remote_candidates.clear();
        self.remote_candidate_keys.clear();
        self.pending_remote_candidate_keys.clear();
        self.applied_remote_candidate_keys.clear();
        self.remote_ice_gathering_complete = false;
        self.remote_candidate_count_total = 0;
        self.remote_candidate_host_count = 0;
        self.remote_candidate_srflx_count = 0;
        self.remote_candidate_relay_count = 0;
        self.remote_candidate_unknown_count = 0;
        self.latest_remote_candidate_kind = None;
        self.latest_remote_candidate_key = None;
        self.data_channel_labels.clear();
        self.input_channel_open = false;
        self.input_metadata_bootstrapped = false;
        self.input_metadata_bootstrapped_after_handshake = false;
        self.input_backpressure_high = false;
        self.chat_channel_open = false;
        self.local_ice_gathering_complete = false;
        self.local_offer_sdp = None;
        self.remote_answer_sdp = None;
        self.session_target_type = Some(session_target_type);
    }

    pub(crate) fn record_local_candidate(
        &mut self,
        candidate: XbxEngineIceCandidateDto,
        kind: RtcIceCandidateKind,
    ) -> bool {
        let key = candidate_identity_key(&candidate);
        if !self.local_candidate_keys.insert(key.clone()) {
            self.latest_local_candidate_kind = Some(kind);
            self.latest_local_candidate_key = Some(key);
            return false;
        }
        self.local_candidate_count_total = self.local_candidate_count_total.saturating_add(1);
        let now_ms = now_ms_f64();
        if self.local_candidate_first_observed_at_ms.is_none() {
            self.local_candidate_first_observed_at_ms = Some(now_ms);
        }
        self.local_candidate_last_observed_at_ms = Some(now_ms);
        match kind {
            RtcIceCandidateKind::Host => {
                self.local_candidate_host_count = self.local_candidate_host_count.saturating_add(1)
            }
            RtcIceCandidateKind::Srflx => {
                self.local_candidate_srflx_count =
                    self.local_candidate_srflx_count.saturating_add(1)
            }
            RtcIceCandidateKind::Relay => {
                self.local_candidate_relay_count =
                    self.local_candidate_relay_count.saturating_add(1)
            }
            RtcIceCandidateKind::Unknown => {
                self.local_candidate_unknown_count =
                    self.local_candidate_unknown_count.saturating_add(1)
            }
        }
        self.latest_local_candidate_kind = Some(kind);
        self.latest_local_candidate_key = Some(key);
        self.local_candidates.push(candidate);
        true
    }

    pub(crate) fn record_local_end_of_candidates(&mut self) {
        self.local_candidate_end_of_candidates_count = self
            .local_candidate_end_of_candidates_count
            .saturating_add(1);
        self.local_ice_gathering_complete = true;
        self.latest_local_candidate_kind = None;
        self.latest_local_candidate_key = None;
    }

    pub(crate) fn record_remote_end_of_candidates(&mut self) -> bool {
        if self.remote_ice_gathering_complete {
            return false;
        }
        self.remote_ice_gathering_complete = true;
        self.latest_remote_candidate_kind = None;
        self.latest_remote_candidate_key = None;
        true
    }

    pub(crate) fn record_remote_candidate(
        &mut self,
        candidate: XbxEngineIceCandidateDto,
        kind: RtcIceCandidateKind,
        pending_only: bool,
        applied: bool,
    ) -> bool {
        let key = candidate_identity_key(&candidate);
        if !self.remote_candidate_keys.insert(key.clone()) {
            self.latest_remote_candidate_kind = Some(kind);
            self.latest_remote_candidate_key = Some(key);
            return false;
        }
        self.remote_candidate_count_total = self.remote_candidate_count_total.saturating_add(1);
        match kind {
            RtcIceCandidateKind::Host => {
                self.remote_candidate_host_count =
                    self.remote_candidate_host_count.saturating_add(1)
            }
            RtcIceCandidateKind::Srflx => {
                self.remote_candidate_srflx_count =
                    self.remote_candidate_srflx_count.saturating_add(1)
            }
            RtcIceCandidateKind::Relay => {
                self.remote_candidate_relay_count =
                    self.remote_candidate_relay_count.saturating_add(1)
            }
            RtcIceCandidateKind::Unknown => {
                self.remote_candidate_unknown_count =
                    self.remote_candidate_unknown_count.saturating_add(1)
            }
        }
        self.latest_remote_candidate_kind = Some(kind);
        self.latest_remote_candidate_key = Some(key.clone());
        if pending_only && !applied {
            if self.pending_remote_candidate_keys.insert(key) {
                self.pending_remote_candidates.push(candidate.clone());
            }
        }
        self.remote_candidates.push(candidate);
        true
    }

    pub(crate) fn candidate_snapshot_summary(&self) -> String {
        let local_latest = self
            .latest_local_candidate_kind
            .map(|kind| kind.as_str())
            .unwrap_or("none");
        let remote_latest = self
            .latest_remote_candidate_kind
            .map(|kind| kind.as_str())
            .unwrap_or("none");
        format!(
            "local total={} host={} srflx={} relay={} unknown={} eoc={} complete={} latestKind={} latestKey={:?} remote total={} host={} srflx={} relay={} unknown={} complete={} latestKind={} latestKey={:?}",
            self.local_candidate_count_total,
            self.local_candidate_host_count,
            self.local_candidate_srflx_count,
            self.local_candidate_relay_count,
            self.local_candidate_unknown_count,
            self.local_candidate_end_of_candidates_count,
            self.local_ice_gathering_complete,
            local_latest,
            self.latest_local_candidate_key,
            self.remote_candidate_count_total,
            self.remote_candidate_host_count,
            self.remote_candidate_srflx_count,
            self.remote_candidate_relay_count,
            self.remote_candidate_unknown_count,
            self.remote_ice_gathering_complete,
            remote_latest,
            self.latest_remote_candidate_key
        )
    }
}

fn candidate_identity_key(candidate: &XbxEngineIceCandidateDto) -> String {
    format!(
        "{}|{:?}|{}",
        candidate.candidate,
        candidate.sdp_m_line_index,
        candidate.sdp_mid.as_deref().unwrap_or("")
    )
}
