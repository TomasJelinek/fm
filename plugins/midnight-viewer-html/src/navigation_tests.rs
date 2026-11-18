//! Authored before navigation.rs. All cases are UNRUN until Build grants a lease.
use super::*;

fn source() -> api::Resource {
    api::Resource {
        id: 1,
        length: 4,
        mime: "text/html".into(),
    }
}

fn navigation() -> Navigation {
    Navigation::new(7, 9, source())
}

fn binding(path: &str) -> Binding {
    Binding {
        session: 7,
        document: 9,
        browser: 3,
        frame: "main-frame".into(),
        url: format!("https://mewv-9.invalid/{path}"),
    }
}

fn intent(path: &str) -> Intent {
    Intent {
        binding: binding(path),
        root: false,
    }
}

fn resolved(request: u64, id: u64, length: u64) -> api::ResourceReply {
    api::ResourceReply::Resolved {
        request,
        resource: api::Resource {
            id,
            length,
            mime: "text/html".into(),
        },
    }
}

fn data(request: u64, resource: u64, offset: u64, bytes: &[u8]) -> api::ResourceReply {
    api::ResourceReply::Data {
        request,
        resource,
        offset,
        bytes: bytes.to_vec(),
    }
}

fn effect(nav: &mut Navigation, now: Instant, request: u64) -> Effect {
    nav.next_effect(now, request, true)
        .expect("expected one public effect")
}

fn begin_resolved(nav: &mut Navigation, now: Instant, path: &str, length: u64) {
    assert!(nav.begin(intent(path), now));
    assert_eq!(
        effect(nav, now, 10),
        Effect::Resolve {
            request: 10,
            base: 1,
            url: path.into()
        }
    );
    assert_eq!(nav.pending_requests(), 1);
    assert_eq!(nav.receive(resolved(10, 41, length), now), Ok(()));
}

fn ready(nav: &mut Navigation, now: Instant, path: &str) -> Reissue {
    begin_resolved(nav, now, path, 3);
    assert_eq!(
        effect(nav, now, 11),
        Effect::Read {
            request: 11,
            resource: 41,
            offset: 0,
            length: 3
        }
    );
    assert_eq!(nav.receive(data(11, 41, 0, b"new"), now), Ok(()));
    nav.next_reissue(now)
        .expect("complete admitted bytes permit one reissue")
}

fn remaining(nav: &mut Navigation, now: Instant) -> Vec<Effect> {
    let mut effects = Vec::new();
    for request in 100..110 {
        let Some(next) = nav.next_effect(now, request, true) else {
            return effects;
        };
        effects.push(next);
    }
    panic!("bounded state emitted an unbounded effect sequence");
}

fn assert_denied_and_released(nav: &mut Navigation, now: Instant, ids: &[u64]) {
    let effects = remaining(nav, now);
    assert_eq!(
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Release { resource } => Some(*resource),
                _ => None,
            })
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Denied { .. }))
            .count(),
        1
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Resolve { .. } | Effect::Read { .. }))
    );
    assert!(nav.next_reissue(now).is_none());
}

#[test]
fn resolve_denial_keeps_navigation_unissued_without_releasing_main() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(intent("refused.html"), now));
    let _ = effect(&mut nav, now, 10);
    assert_eq!(
        nav.receive(
            api::ResourceReply::Denied {
                request: 10,
                reason: "denied".into()
            },
            now
        ),
        Ok(())
    );
    assert_eq!(nav.pending_requests(), 0);
    assert_denied_and_released(&mut nav, now, &[]);
}

#[test]
fn first_middle_and_final_read_denials_never_reissue_partial_body() {
    let now = Instant::now();
    for denied_chunk in 0..3 {
        let mut nav = navigation();
        begin_resolved(&mut nav, now, "three.html", 3);
        for chunk in 0..=denied_chunk {
            let request = 11 + chunk;
            assert_eq!(
                effect(&mut nav, now, request),
                Effect::Read {
                    request,
                    resource: 41,
                    offset: chunk,
                    length: (3 - chunk) as u32,
                }
            );
            let reply = if chunk == denied_chunk {
                api::ResourceReply::Denied {
                    request,
                    reason: "read refused".into(),
                }
            } else {
                data(request, 41, chunk, b"x")
            };
            assert_eq!(nav.receive(reply, now), Ok(()));
        }
        assert_denied_and_released(&mut nav, now, &[41]);
    }
}

#[test]
fn short_positive_chunks_collect_once_without_read_after_allow() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "short.html", 4);
    assert_eq!(
        effect(&mut nav, now, 11),
        Effect::Read {
            request: 11,
            resource: 41,
            offset: 0,
            length: 4
        }
    );
    assert_eq!(nav.receive(data(11, 41, 0, b"a"), now), Ok(()));
    assert_eq!(
        effect(&mut nav, now, 12),
        Effect::Read {
            request: 12,
            resource: 41,
            offset: 1,
            length: 3
        }
    );
    assert_eq!(nav.receive(data(12, 41, 1, b"bcd"), now), Ok(()));
    let reissue = nav.next_reissue(now).unwrap();
    assert!(nav.allow(&reissue.marker, &reissue.binding, now));
    assert!(remaining(&mut nav, now).is_empty());
    let response = nav.claim(&reissue.marker, &reissue.binding).unwrap();
    assert_eq!(response.body.as_deref(), Some(b"abcd".as_slice()));
    assert_eq!(response.resource.id, 41);
    assert!(nav.claim(&reissue.marker, &reissue.binding).is_none());
    let attempt = response.attempt;
    drop(response); // The bridge owns checked lease release after the claim.
    nav.response_finished(attempt);
    assert!(
        remaining(&mut nav, now).is_empty(),
        "core must not release a transferred lease twice"
    );
}

#[test]
fn zero_length_requires_a_positive_read_and_matching_empty_eof() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "empty.html", 0);
    assert!(nav.next_reissue(now).is_none());
    assert_eq!(
        effect(&mut nav, now, 11),
        Effect::Read {
            request: 11,
            resource: 41,
            offset: 0,
            length: 1
        }
    );
    assert_eq!(nav.receive(data(11, 41, 0, b""), now), Ok(()));
    let ready = nav.next_reissue(now).unwrap();
    assert!(nav.allow(&ready.marker, &ready.binding, now));
    assert_eq!(
        nav.claim(&ready.marker, &ready.binding).unwrap().body,
        Some(Vec::new())
    );
}

#[test]
fn exact_one_mib_is_collected_but_larger_targets_transfer_the_same_stream_lease() {
    let now = Instant::now();
    let mut bounded = navigation();
    begin_resolved(&mut bounded, now, "one-mib.html", 1_048_576);
    for chunk in 0..4 {
        let request = 11 + chunk;
        assert_eq!(
            effect(&mut bounded, now, request),
            Effect::Read {
                request,
                resource: 41,
                offset: chunk * 262_144,
                length: 262_144,
            }
        );
        assert_eq!(
            bounded.receive(
                api::ResourceReply::Data {
                    request,
                    resource: 41,
                    offset: chunk * 262_144,
                    bytes: vec![b'x'; 262_144],
                },
                now
            ),
            Ok(())
        );
    }
    let reissue = bounded.next_reissue(now).unwrap();
    assert!(bounded.allow(&reissue.marker, &reissue.binding, now));
    let response = bounded.claim(&reissue.marker, &reissue.binding).unwrap();
    let body = response.body.unwrap();
    assert_eq!(body.len(), 1_048_576);
    assert!(body.capacity() <= 1_048_576);

    let mut large = navigation();
    begin_resolved(&mut large, now, "large.html", 1_048_577);
    assert!(
        remaining(&mut large, now).is_empty(),
        "large target must not be pre-read or resolved twice"
    );
    let reissue = large.next_reissue(now).unwrap();
    assert!(large.allow(&reissue.marker, &reissue.binding, now));
    let response = large.claim(&reissue.marker, &reissue.binding).unwrap();
    assert_eq!(
        response.resource,
        api::Resource {
            id: 41,
            length: 1_048_577,
            mime: "text/html".into()
        }
    );
    assert!(
        response.body.is_none(),
        "existing streaming handler receives the already resolved capability"
    );
}

#[test]
fn bad_read_identity_empty_progress_and_excess_data_fail_closed() {
    let now = Instant::now();
    let replies = [
        data(11, 99, 0, b"a"),
        data(11, 41, 1, b"a"),
        data(11, 41, 0, b""),
        data(11, 41, 0, b"abcde"),
    ];
    for reply in replies {
        let mut nav = navigation();
        begin_resolved(&mut nav, now, "bad.html", 4);
        let _ = effect(&mut nav, now, 11);
        assert_eq!(nav.receive(reply, now), Ok(()));
        assert_denied_and_released(&mut nav, now, &[41]);
    }
}

#[test]
fn unmatched_reply_is_returned_with_the_original_owned_body_for_legacy_streams() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "body.html", 4);
    let _ = effect(&mut nav, now, 11);
    let bytes = Vec::from(b"legacy".as_slice());
    let original = bytes.as_ptr();
    let reply = nav
        .receive(
            api::ResourceReply::Data {
                request: 77,
                resource: 90,
                offset: 12,
                bytes,
            },
            now,
        )
        .unwrap_err();
    let api::ResourceReply::Data { bytes, .. } = reply else {
        panic!("returned different reply")
    };
    assert_eq!(bytes.as_ptr(), original);
    assert_eq!(bytes, b"legacy");
    assert_eq!(nav.pending_requests(), 1);
    let released = api::ResourceReply::Released { resource: 90 };
    assert_eq!(nav.receive(released.clone(), now), Err(released));
}

#[test]
fn malformed_resolved_metadata_denies_and_returns_every_real_nonmain_grant() {
    let now = Instant::now();
    for resource in [
        api::Resource {
            id: 0,
            length: 3,
            mime: "text/html".into(),
        },
        api::Resource {
            id: 41,
            length: 3,
            mime: "".into(),
        },
        api::Resource {
            id: 41,
            length: 3,
            mime: "text/\nhtml".into(),
        },
    ] {
        let mut nav = navigation();
        assert!(nav.begin(intent("bad-metadata.html"), now));
        let _ = effect(&mut nav, now, 10);
        let expected = if resource.id == 0 {
            Vec::new()
        } else {
            vec![41]
        };
        assert_eq!(
            nav.receive(
                api::ResourceReply::Resolved {
                    request: 10,
                    resource
                },
                now
            ),
            Ok(())
        );
        assert_denied_and_released(&mut nav, now, &expected);
    }
}

#[test]
fn allocation_failure_cannot_switch_a_small_target_to_streaming() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(intent("no-memory.html"), now));
    let _ = effect(&mut nav, now, 10);
    assert_eq!(nav.receive_with(resolved(10, 41, 3), now, |_| None), Ok(()));
    assert_denied_and_released(&mut nav, now, &[41]);
}

#[test]
fn reported_capacity_above_the_body_credit_is_rejected() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(intent("too-much-capacity.html"), now));
    let _ = effect(&mut nav, now, 10);
    assert_eq!(
        nav.receive_with(resolved(10, 41, 3), now, |_| Some(Vec::with_capacity(
            1_048_577
        ))),
        Ok(())
    );
    assert_denied_and_released(&mut nav, now, &[41]);
}

#[test]
fn whole_attempt_deadline_does_not_restart_after_short_progress() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "slow.html", 3);
    let _ = effect(&mut nav, now, 11);
    assert_eq!(
        nav.receive(data(11, 41, 0, b"a"), now + Duration::from_secs(29)),
        Ok(())
    );
    let _ = effect(&mut nav, now + Duration::from_secs(29), 12);
    nav.tick(now + Duration::from_secs(30));
    assert_eq!(
        nav.pending_requests(),
        1,
        "uncancellable request must retain retirement correlation"
    );
    assert_denied_and_released(&mut nav, now + Duration::from_secs(30), &[]);
    assert_eq!(
        nav.receive(data(12, 41, 1, b"bc"), now + Duration::from_secs(31)),
        Ok(())
    );
    assert_eq!(nav.pending_requests(), 0);
    assert!(nav.next_reissue(now + Duration::from_secs(31)).is_none());
    assert_eq!(
        remaining(&mut nav, now + Duration::from_secs(31)),
        vec![Effect::Release { resource: 41 }]
    );
}

#[test]
fn ready_and_issued_expire_before_the_final_before_browse_allow() {
    let now = Instant::now();
    let mut nav = navigation();
    let reissue = ready(&mut nav, now, "expired.html");
    assert!(!nav.allow(
        &reissue.marker,
        &reissue.binding,
        now + Duration::from_secs(30)
    ));
    assert_denied_and_released(&mut nav, now + Duration::from_secs(30), &[41]);

    let mut ready_only = navigation();
    begin_resolved(&mut ready_only, now, "ready.html", 0);
    let _ = effect(&mut ready_only, now, 11);
    assert_eq!(ready_only.receive(data(11, 41, 0, b""), now), Ok(()));
    assert!(
        ready_only
            .next_reissue(now + Duration::from_secs(30))
            .is_none()
    );
    assert_denied_and_released(&mut ready_only, now + Duration::from_secs(30), &[41]);
}

#[test]
fn allowed_response_survives_timeout_and_new_intent_until_the_exact_claim_finishes() {
    let now = Instant::now();
    let mut nav = navigation();
    let first = ready(&mut nav, now, "a.html");
    assert!(nav.allow(&first.marker, &first.binding, now));
    assert!(nav.begin(intent("b.html"), now + Duration::from_secs(29)));
    nav.tick(now + Duration::from_secs(31));
    assert!(remaining(&mut nav, now + Duration::from_secs(31)).is_empty());
    assert!(nav.next_reissue(now + Duration::from_secs(31)).is_none());
    let response = nav
        .claim(&first.marker, &first.binding)
        .expect("allowed bytes cannot disappear before IO claim");
    assert_eq!(response.body.as_deref(), Some(b"new".as_slice()));
    assert!(remaining(&mut nav, now + Duration::from_secs(31)).is_empty());
    nav.response_finished(first.attempt + 1);
    assert!(remaining(&mut nav, now + Duration::from_secs(31)).is_empty());
    drop(response);
    nav.response_finished(first.attempt);
    assert_eq!(
        effect(&mut nav, now + Duration::from_secs(31), 20),
        Effect::Resolve {
            request: 20,
            base: 1,
            url: "b.html".into()
        }
    );
}

#[test]
fn serving_body_credit_coalesces_only_the_latest_intent() {
    let now = Instant::now();
    let mut nav = navigation();
    let first = ready(&mut nav, now, "a.html");
    assert!(nav.allow(&first.marker, &first.binding, now));
    let response = nav.claim(&first.marker, &first.binding).unwrap();
    assert!(nav.begin(intent("b.html"), now));
    assert!(nav.begin(intent("c.html"), now));
    assert!(remaining(&mut nav, now).is_empty());
    drop(response);
    nav.response_finished(first.attempt);
    assert_eq!(
        effect(&mut nav, now, 20),
        Effect::Resolve {
            request: 20,
            base: 1,
            url: "c.html".into()
        }
    );
}

#[test]
fn queued_intent_uses_its_original_deadline_instead_of_restarting_after_serving() {
    let now = Instant::now();
    let mut nav = navigation();
    let first = ready(&mut nav, now, "a.html");
    assert!(nav.allow(&first.marker, &first.binding, now));
    let response = nav.claim(&first.marker, &first.binding).unwrap();
    assert!(nav.begin(intent("b.html"), now));
    nav.tick(now + Duration::from_secs(30));
    drop(response);
    nav.response_finished(first.attempt);
    assert_denied_and_released(&mut nav, now + Duration::from_secs(30), &[]);
}

#[test]
fn late_resolved_grant_is_returned_before_only_the_latest_replacement_can_start() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(intent("a.html"), now));
    let _ = effect(&mut nav, now, 10);
    assert!(nav.begin(intent("b.html"), now));
    assert!(nav.begin(intent("c.html"), now));
    assert!(nav.next_effect(now, 20, true).is_none());
    assert_eq!(nav.pending_requests(), 1);
    assert_eq!(nav.receive(resolved(10, 88, 3), now), Ok(()));
    assert_eq!(effect(&mut nav, now, 20), Effect::Release { resource: 88 });
    assert_eq!(
        effect(&mut nav, now, 21),
        Effect::Resolve {
            request: 21,
            base: 1,
            url: "c.html".into()
        }
    );
}

#[test]
fn superseded_read_waits_for_late_data_without_rearming_old_candidate() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "a.html", 3);
    let _ = effect(&mut nav, now, 11);
    assert!(nav.begin(intent("b.html"), now));
    assert!(
        nav.next_effect(now, 20, true).is_none(),
        "retiring Read still owns its target lease"
    );
    assert_eq!(nav.receive(data(11, 41, 0, b"old"), now), Ok(()));
    assert!(nav.next_reissue(now).is_none());
    assert_eq!(effect(&mut nav, now, 21), Effect::Release { resource: 41 });
    assert_eq!(
        effect(&mut nav, now, 22),
        Effect::Resolve {
            request: 22,
            base: 1,
            url: "b.html".into()
        }
    );
}

#[test]
fn stale_issued_marker_cannot_consume_same_url_replacement_or_start_a_new_attempt() {
    let now = Instant::now();
    let mut nav = navigation();
    let old = ready(&mut nav, now, "same.html");
    assert!(nav.begin(intent("same.html"), now));
    assert_eq!(effect(&mut nav, now, 20), Effect::Release { resource: 41 });
    assert_eq!(
        effect(&mut nav, now, 21),
        Effect::Resolve {
            request: 21,
            base: 1,
            url: "same.html".into()
        }
    );
    assert_eq!(nav.receive(resolved(21, 42, 0), now), Ok(()));
    let _ = effect(&mut nav, now, 22);
    assert_eq!(nav.receive(data(22, 42, 0, b""), now), Ok(()));
    let current = nav.next_reissue(now).unwrap();
    assert_ne!(old.marker, current.marker);
    assert!(!nav.allow(&old.marker, &old.binding, now));
    assert!(nav.allow(&current.marker, &current.binding, now));
    assert!(remaining(&mut nav, now).is_empty());
}

#[test]
fn marker_and_full_binding_allow_and_claim_only_once() {
    let now = Instant::now();
    let mut nav = navigation();
    let reissue = ready(&mut nav, now, "bound.html");
    let mut wrong = Vec::new();
    let mut b = reissue.binding.clone();
    b.session += 1;
    wrong.push(b);
    let mut b = reissue.binding.clone();
    b.document += 1;
    wrong.push(b);
    let mut b = reissue.binding.clone();
    b.browser += 1;
    wrong.push(b);
    let mut b = reissue.binding.clone();
    b.frame.push('x');
    wrong.push(b);
    let mut b = reissue.binding.clone();
    b.url.push('x');
    wrong.push(b);
    assert!(!nav.allow("malformed", &reissue.binding, now));
    for b in &wrong {
        assert!(!nav.allow(&reissue.marker, b, now));
    }
    assert!(nav.allow(&reissue.marker, &reissue.binding, now));
    assert!(!nav.allow(&reissue.marker, &reissue.binding, now));
    for b in &wrong {
        assert!(nav.claim(&reissue.marker, b).is_none());
    }
    assert!(nav.claim("malformed", &reissue.binding).is_none());
    assert!(nav.claim(&reissue.marker, &reissue.binding).is_some());
    assert!(nav.claim(&reissue.marker, &reissue.binding).is_none());
}

#[test]
fn explicit_same_url_repeat_must_obtain_a_new_public_grant() {
    let now = Instant::now();
    let mut nav = navigation();
    let first = ready(&mut nav, now, "same.html");
    assert!(nav.allow(&first.marker, &first.binding, now));
    drop(nav.claim(&first.marker, &first.binding).unwrap());
    nav.response_finished(first.attempt);
    assert!(nav.begin(intent("same.html"), now));
    assert_eq!(
        effect(&mut nav, now, 20),
        Effect::Resolve {
            request: 20,
            base: 1,
            url: "same.html".into()
        }
    );
}

#[test]
fn root_reload_reads_the_supplied_main_resource_without_resolve_or_release() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(
        Intent {
            binding: binding(""),
            root: true
        },
        now
    ));
    assert_eq!(
        effect(&mut nav, now, 10),
        Effect::Read {
            request: 10,
            resource: 1,
            offset: 0,
            length: 4
        }
    );
    assert_eq!(nav.receive(data(10, 1, 0, b"root"), now), Ok(()));
    let reissue = nav.next_reissue(now).unwrap();
    nav.close();
    assert!(remaining(&mut nav, now).is_empty());
    assert!(!nav.allow(&reissue.marker, &reissue.binding, now));
}

#[test]
fn root_query_and_fragment_keep_the_full_binding_but_read_only_the_supplied_main_id() {
    let now = Instant::now();
    for suffix in ["?q=one", "#anchor", "?q=one#anchor"] {
        let mut nav = navigation();
        let target = binding(suffix);
        assert!(nav.begin(
            Intent {
                binding: target.clone(),
                root: true
            },
            now
        ));
        assert_eq!(
            effect(&mut nav, now, 10),
            Effect::Read {
                request: 10,
                resource: 1,
                offset: 0,
                length: 4
            }
        );
        assert_eq!(nav.receive(data(10, 1, 0, b"root"), now), Ok(()));
        let reissue = nav.next_reissue(now).unwrap();
        assert_eq!(reissue.binding, target);
        assert!(!nav.allow(&reissue.marker, &binding(""), now));
        assert!(nav.allow(&reissue.marker, &target, now));
        let response = nav.claim(&reissue.marker, &target).unwrap();
        assert_eq!(response.resource.id, 1);
        assert_eq!(response.body.as_deref(), Some(b"root".as_slice()));
        drop(response);
        nav.response_finished(reissue.attempt);
        assert!(remaining(&mut nav, now).is_empty());
        assert!(!nav.begin(
            Intent {
                binding: target,
                root: false
            },
            now
        ));
    }
}

#[test]
fn zero_length_root_reload_still_uses_one_positive_eof_read() {
    let now = Instant::now();
    let mut nav = Navigation::new(
        7,
        9,
        api::Resource {
            length: 0,
            ..source()
        },
    );
    assert!(nav.begin(
        Intent {
            binding: binding(""),
            root: true
        },
        now
    ));
    assert_eq!(
        effect(&mut nav, now, 10),
        Effect::Read {
            request: 10,
            resource: 1,
            offset: 0,
            length: 1
        }
    );
    assert_eq!(nav.receive(data(10, 1, 0, b""), now), Ok(()));
    assert!(nav.next_reissue(now).is_some());
}

#[test]
fn close_releases_unallowed_body_and_retires_late_grant_without_starting_latest() {
    let now = Instant::now();
    let mut nav = navigation();
    let _ = ready(&mut nav, now, "ready.html");
    nav.close();
    assert_eq!(
        remaining(&mut nav, now),
        vec![Effect::Release { resource: 41 }]
    );
    assert!(!nav.begin(intent("later.html"), now));

    let mut waiting = navigation();
    assert!(waiting.begin(intent("a.html"), now));
    let _ = effect(&mut waiting, now, 10);
    assert!(waiting.begin(intent("b.html"), now));
    waiting.close();
    assert_eq!(waiting.pending_requests(), 1);
    assert_eq!(waiting.receive(resolved(10, 88, 3), now), Ok(()));
    assert_eq!(
        remaining(&mut waiting, now),
        vec![Effect::Release { resource: 88 }]
    );
}

#[test]
fn close_retains_a_pending_read_lease_until_the_actual_reply_settles() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "closing-read.html", 3);
    let _ = effect(&mut nav, now, 11);
    nav.close();
    assert!(remaining(&mut nav, now).is_empty());
    assert_eq!(nav.pending_requests(), 1);
    assert_eq!(nav.receive(data(11, 41, 0, b"new"), now), Ok(()));
    assert_eq!(nav.pending_requests(), 0);
    assert_eq!(
        remaining(&mut nav, now),
        vec![Effect::Release { resource: 41 }]
    );
    assert!(nav.next_reissue(now).is_none());
}

#[test]
fn retired_read_returns_its_owned_lease_and_any_different_late_resolved_grant() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "retiring-read.html", 3);
    let _ = effect(&mut nav, now, 11);
    nav.close();
    assert_eq!(nav.receive(resolved(11, 42, 3), now), Ok(()));
    let mut releases: Vec<_> = remaining(&mut nav, now)
        .into_iter()
        .filter_map(|effect| {
            if let Effect::Release { resource } = effect {
                Some(resource)
            } else {
                None
            }
        })
        .collect();
    releases.sort_unstable();
    assert_eq!(releases, [41, 42]);
}

#[test]
fn close_keeps_allowed_unclaimed_lease_until_explicit_response_teardown() {
    let now = Instant::now();
    let mut nav = navigation();
    let reissue = ready(&mut nav, now, "allowed.html");
    assert_eq!(nav.unclaimed_allowed_attempt(), None);
    assert!(nav.allow(&reissue.marker, &reissue.binding, now));
    assert_eq!(nav.unclaimed_allowed_attempt(), Some(reissue.attempt));
    nav.close();
    assert_eq!(nav.unclaimed_allowed_attempt(), Some(reissue.attempt));
    assert!(remaining(&mut nav, now + Duration::from_secs(60)).is_empty());
    assert!(nav.claim(&reissue.marker, &reissue.binding).is_none());
    nav.response_finished(reissue.attempt);
    assert_eq!(nav.unclaimed_allowed_attempt(), None);
    assert_eq!(
        remaining(&mut nav, now),
        vec![Effect::Release { resource: 41 }]
    );
    nav.response_finished(reissue.attempt);
    assert!(remaining(&mut nav, now).is_empty());
}

#[test]
fn close_never_double_releases_a_lease_already_transferred_to_response_owner() {
    let now = Instant::now();
    let mut nav = navigation();
    let reissue = ready(&mut nav, now, "serving.html");
    assert!(nav.allow(&reissue.marker, &reissue.binding, now));
    let response = nav.claim(&reissue.marker, &reissue.binding).unwrap();
    assert_eq!(nav.unclaimed_allowed_attempt(), None);
    nav.close();
    assert!(remaining(&mut nav, now).is_empty());
    drop(response);
    nav.response_finished(reissue.attempt);
    assert!(remaining(&mut nav, now).is_empty());
}

#[test]
fn unexpected_resolved_for_read_returns_both_derived_leases_once() {
    let now = Instant::now();
    let mut nav = navigation();
    begin_resolved(&mut nav, now, "wrong-type.html", 3);
    let _ = effect(&mut nav, now, 11);
    assert_eq!(nav.receive(resolved(11, 42, 3), now), Ok(()));
    let effects = remaining(&mut nav, now);
    let mut releases: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Release { resource } => Some(*resource),
            _ => None,
        })
        .collect();
    releases.sort_unstable();
    assert_eq!(releases, [41, 42]);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Denied { .. }))
            .count(),
        1
    );
    assert!(nav.next_reissue(now).is_none());
}

#[test]
fn bridge_refusal_discards_only_the_matching_unallowed_reissue() {
    let now = Instant::now();
    let mut nav = navigation();
    let issued = ready(&mut nav, now, "cannot-issue.html");
    nav.refuse(issued.attempt, "Cannot issue navigation.");
    assert_denied_and_released(&mut nav, now, &[41]);
    assert!(!nav.allow(&issued.marker, &issued.binding, now));
}

#[test]
fn stale_or_after_allow_bridge_refusal_cannot_destroy_the_current_response() {
    let now = Instant::now();
    let mut nav = navigation();
    let old = ready(&mut nav, now, "same.html");
    assert!(nav.begin(intent("same.html"), now));
    assert_eq!(effect(&mut nav, now, 20), Effect::Release { resource: 41 });
    let _ = effect(&mut nav, now, 21);
    assert_eq!(nav.receive(resolved(21, 42, 0), now), Ok(()));
    let _ = effect(&mut nav, now, 22);
    assert_eq!(nav.receive(data(22, 42, 0, b""), now), Ok(()));
    let current = nav.next_reissue(now).unwrap();
    nav.refuse(old.attempt, "Obsolete request failure.");
    assert!(nav.allow(&current.marker, &current.binding, now));
    nav.refuse(current.attempt, "Late issue failure.");
    assert!(remaining(&mut nav, now).is_empty());
    let response = nav.claim(&current.marker, &current.binding).unwrap();
    nav.refuse(current.attempt, "Late response failure.");
    assert!(remaining(&mut nav, now).is_empty());
    assert_eq!(response.body, Some(Vec::new()));
}

#[test]
fn ordinary_navigation_cancels_only_unallowed_work_without_a_denial_notice() {
    let now = Instant::now();
    let mut pending = navigation();
    begin_resolved(&mut pending, now, "old.html", 3);
    let _ = effect(&mut pending, now, 11);
    assert!(pending.begin(intent("queued.html"), now));
    pending.cancel_preparation();
    assert!(remaining(&mut pending, now).is_empty());
    assert_eq!(pending.receive(data(11, 41, 0, b"old"), now), Ok(()));
    assert_eq!(
        remaining(&mut pending, now),
        vec![Effect::Release { resource: 41 }]
    );
    assert!(pending.next_reissue(now).is_none());

    let mut allowed = navigation();
    let reissue = ready(&mut allowed, now, "allowed.html");
    assert!(allowed.allow(&reissue.marker, &reissue.binding, now));
    assert!(allowed.begin(intent("queued.html"), now));
    allowed.cancel_preparation();
    assert_eq!(allowed.unclaimed_allowed_attempt(), Some(reissue.attempt));
    let response = allowed.claim(&reissue.marker, &reissue.binding).unwrap();
    assert_eq!(response.body.as_deref(), Some(b"new".as_slice()));
    drop(response);
    allowed.response_finished(reissue.attempt);
    assert!(remaining(&mut allowed, now).is_empty());
}

#[test]
fn pending_table_capacity_blocks_new_work_but_not_release_or_denial() {
    let now = Instant::now();
    let mut nav = navigation();
    assert!(nav.begin(intent("a.html"), now));
    assert!(nav.next_effect(now, 10, false).is_none());
    assert_eq!(nav.pending_requests(), 0);
    assert!(matches!(
        effect(&mut nav, now, 11),
        Effect::Resolve { request: 11, .. }
    ));
    assert!(nav.next_effect(now, 12, true).is_none());
    assert_eq!(
        nav.receive(
            api::ResourceReply::Denied {
                request: 11,
                reason: "no".into()
            },
            now
        ),
        Ok(())
    );
    assert!(matches!(
        nav.next_effect(now, 12, false),
        Some(Effect::Denied { .. })
    ));
}

#[test]
fn invalid_namespace_metadata_and_exhausted_attempt_ids_fail_closed() {
    let now = Instant::now();
    let mut nav = navigation();
    let mut wrong = intent("a.html");
    wrong.binding.document = 10;
    assert!(!nav.begin(wrong, now));
    let mut wrong = intent("a.html");
    wrong.binding.url = "https://external.example/a.html".into();
    assert!(!nav.begin(wrong, now));
    let mut wrong = intent("a.html");
    wrong.binding.frame.clear();
    assert!(!nav.begin(wrong, now));
    let mut wrong = intent("a.html");
    wrong.root = true;
    assert!(!nav.begin(wrong, now));
    nav.last_attempt = u64::MAX;
    assert!(!nav.begin(intent("a.html"), now));
    let effects = remaining(&mut nav, now);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Resolve { .. } | Effect::Read { .. }))
    );
}
