use super::ice::IceCandidate;
use crate::session::api::signaling::AnswerPayload;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionSignalingState {
    pub generation: u64,
    pub offer_started: bool,
    pub answers: HashMap<&'static str, AnswerPayload>,
    pub media_offer_credentials: Option<Vec<String>>,
    pub last_polled_ice: Option<Vec<IceCandidate>>,
    pub baseline_ice: Option<Vec<IceCandidate>>,
    remote_ufrags: Vec<String>,
}

impl SessionSignalingState {
    pub fn begin_media_ice(
        &mut self,
        credentials: Vec<String>,
        baseline: Option<Vec<IceCandidate>>,
    ) {
        self.generation += 1;
        self.media_offer_credentials = Some(credentials);
        self.baseline_ice = baseline;
        self.last_polled_ice = None;
    }

    pub fn apply_media_answer(&mut self, answer: &AnswerPayload) {
        let ufrags = sdp_attributes(&answer.sdp, &["a=ice-ufrag:"]);
        // 新 answer 明确复用远端 ICE 凭证时，同地址候选仍属于当前传输。
        if !ufrags.is_empty() && ufrags == self.remote_ufrags {
            self.baseline_ice = None;
        }
        self.remote_ufrags = ufrags;
    }

    pub fn filter_ice(&mut self, response: Option<Vec<IceCandidate>>) -> Option<Vec<IceCandidate>> {
        let response = response?;
        let mut candidates = Vec::new();
        let mut stale_seen = false;
        for candidate in response.iter().filter(|candidate| !is_end(candidate)) {
            let belongs_to_current = if let Some(ufrag) =
                candidate_ufrag(candidate).filter(|_| !self.remote_ufrags.is_empty())
            {
                self.remote_ufrags.iter().any(|current| current == ufrag)
            } else {
                !self.baseline_ice.as_ref().is_some_and(|baseline| {
                    baseline.iter().any(|old| same_candidate(old, candidate))
                })
            };
            if belongs_to_current {
                candidates.push(candidate.clone());
            } else {
                stale_seen = true;
            }
        }
        // 旧快照的 EOC 无独立代际；跨轮时须与本次快照的有效候选一起准入。
        if !stale_seen && (self.baseline_ice.is_none() || !candidates.is_empty()) {
            candidates.extend(response.into_iter().filter(is_end));
        }
        if candidates.is_empty() || self.last_polled_ice.as_ref() == Some(&candidates) {
            return None;
        }
        self.last_polled_ice = Some(candidates.clone());
        Some(candidates)
    }
}

pub(crate) fn media_ice_credentials(sdp: &str) -> Vec<String> {
    sdp_attributes(sdp, &["a=ice-ufrag:", "a=ice-pwd:"])
}

fn sdp_attributes(sdp: &str, prefixes: &[&str]) -> Vec<String> {
    sdp.lines()
        .filter_map(|line| {
            prefixes.iter().find_map(|prefix| {
                line.trim()
                    .strip_prefix(prefix)
                    .map(|value| value.to_owned())
            })
        })
        .collect()
}

fn candidate_ufrag(candidate: &IceCandidate) -> Option<&str> {
    candidate.username_fragment.as_deref().or_else(|| {
        let mut tokens = candidate.candidate.split_whitespace();
        while let Some(token) = tokens.next() {
            if token.eq_ignore_ascii_case("ufrag") {
                return tokens.next();
            }
        }
        None
    })
}

fn same_candidate(left: &IceCandidate, right: &IceCandidate) -> bool {
    left.candidate == right.candidate
        && left.sdp_mid == right.sdp_mid
        && left.sdp_m_line_index == right.sdp_m_line_index
        && candidate_ufrag(left) == candidate_ufrag(right)
}

fn is_end(candidate: &IceCandidate) -> bool {
    let raw = candidate.candidate.trim();
    raw.is_empty()
        || raw.eq_ignore_ascii_case("a=end-of-candidates")
        || raw.eq_ignore_ascii_case("end-of-candidates")
}

#[cfg(test)]
#[path = "state.test.rs"]
mod tests;
