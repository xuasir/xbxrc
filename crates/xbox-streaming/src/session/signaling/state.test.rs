use super::*;

fn candidate(ufrag: Option<&str>) -> IceCandidate {
    IceCandidate {
        candidate: "candidate:1 1 UDP 1 192.0.2.1 5000 typ host".to_owned(),
        sdp_mid: Some("0".to_owned()),
        sdp_m_line_index: Some(0),
        username_fragment: ufrag.map(str::to_owned),
        ..Default::default()
    }
}
fn end() -> IceCandidate {
    IceCandidate {
        candidate: "a=end-of-candidates".to_owned(),
        ..Default::default()
    }
}
fn answer(ufrag: &str) -> AnswerPayload {
    AnswerPayload {
        sdp: format!("v=0\r\na=ice-ufrag:{ufrag}\r\n"),
        message_type: None,
    }
}
fn restarted() -> SessionSignalingState {
    let mut state = SessionSignalingState::default();
    state.apply_media_answer(&answer("old"));
    state.begin_media_ice(
        vec!["local-new".to_owned()],
        Some(vec![candidate(Some("old")), end()]),
    );
    state.apply_media_answer(&answer("new"));
    state
}

#[test]
fn rejects_old_credentials_and_end_marker_even_when_snapshot_changes() {
    let mut state = restarted();
    let mut old = candidate(Some("old"));
    old.candidate = "candidate:2 1 UDP 1 192.0.2.9 6000 typ host".to_owned();
    assert_eq!(state.filter_ice(Some(vec![old, end()])), None);
    assert_eq!(state.filter_ice(Some(vec![end()])), None);
    assert_eq!(state.filter_ice(None), None);
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), end()])),
        None
    );
}

#[test]
fn accepts_reused_address_with_current_ufrag_and_preserves_end_marker() {
    let mut state = restarted();
    let current = vec![candidate(Some("new")), end()];
    assert_eq!(
        state.filter_ice(Some(current.clone())),
        Some(current.clone())
    );
    assert_eq!(state.filter_ice(Some(current)), None);
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), end()])),
        None
    );
}

#[test]
fn withholds_end_marker_from_a_mixed_generation_snapshot() {
    let mut state = restarted();
    let current = candidate(Some("new"));
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), current.clone(), end()])),
        Some(vec![current.clone()])
    );
    assert_eq!(
        state.filter_ice(Some(vec![current.clone(), end()])),
        Some(vec![current, end()])
    );
}

#[test]
fn uses_candidate_extension_ufrag_when_json_field_is_absent() {
    let mut state = restarted();
    let mut current = candidate(None);
    current.candidate.push_str(" ufrag new");
    assert_eq!(
        state.filter_ice(Some(vec![current.clone(), end()])),
        Some(vec![current, end()])
    );
}

#[test]
fn filters_untagged_baseline_and_accepts_new_untagged_endpoint() {
    let old = candidate(None);
    let mut state = SessionSignalingState::default();
    state.begin_media_ice(vec![], Some(vec![old.clone(), end()]));
    state.apply_media_answer(&answer("new"));
    assert_eq!(state.filter_ice(Some(vec![old.clone(), end()])), None);
    let mut new = old;
    new.candidate = "candidate:1 1 UDP 1 192.0.2.1 5001 typ host".to_owned();
    assert_eq!(
        state.filter_ice(Some(vec![new.clone(), end()])),
        Some(vec![new, end()])
    );
}

#[test]
fn permits_same_candidates_when_fresh_answer_reuses_remote_credentials() {
    let mut state = SessionSignalingState::default();
    let candidates = vec![candidate(None), end()];
    state.apply_media_answer(&answer("same"));
    state.begin_media_ice(vec!["local-new".to_owned()], Some(candidates.clone()));
    state.apply_media_answer(&answer("same"));
    assert_eq!(state.filter_ice(Some(candidates.clone())), Some(candidates));
}
