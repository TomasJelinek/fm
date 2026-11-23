use super::*;

#[derive(Debug)]
struct DropProbe(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn joined_actor_panic_retains_its_arbitrary_payload() {
    let mut session = endpoint(PolicyCeiling::OfflineOnly);
    let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let actor_drops = drops.clone();
    session.thread = Some(std::thread::spawn(move || {
        std::panic::panic_any(DropProbe(actor_drops))
    }));
    assert!(session.release_on_worker().is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}

fn endpoint(ceiling: PolicyCeiling) -> Session {
    let closed = Arc::new(AtomicBool::new(false));
    Session {
        signal: Arc::new(actor::Signal::new(64).unwrap()),
        admission: Mutex::new(()),
        sequence: AtomicU64::new(1),
        output: Arc::default(),
        generation: Arc::default(),
        closed,
        document: Arc::new(AtomicU64::new(1)),
        ceiling,
        terminal: Arc::new(Mutex::new(None)),
        wake: Arc::new(|| true),
        exited: Arc::default(),
        thread: None,
    }
}

#[test]
fn offline_only_session_denies_policy_without_revoking_document_or_input() {
    // A missing admission ceiling would increment the document and enqueue an escalation.
    let session = endpoint(PolicyCeiling::OfflineOnly);
    for policy in [
        api::Policy {
            network: true,
            scripts: false,
        },
        api::Policy {
            network: true,
            scripts: true,
        },
        api::Policy::default(),
    ] {
        assert!(!session.send(api::CommandKind::Policy(policy)));
    }
    assert!(!session.policy_change_allowed());
    assert_eq!(session.document.load(Ordering::Acquire), 1);
    assert!(session.signal.pop_command().is_none());
    assert!(!session.closed.load(Ordering::Acquire));
    assert!(session.send(api::CommandKind::Zoom(1.5)));
}

#[test]
fn actor_rejects_injected_policy_independently_of_session_admission() {
    // The actor's guard must hold even if a caller bypasses Session::send.
    for policy in [
        api::Policy {
            network: true,
            scripts: false,
        },
        api::Policy {
            network: true,
            scripts: true,
        },
        api::Policy::default(),
    ] {
        assert!(
            actor::validate_frontend_command(
                PolicyCeiling::OfflineOnly,
                &api::CommandKind::Policy(policy)
            )
            .is_err()
        );
    }
    assert!(
        actor::validate_frontend_command(
            PolicyCeiling::OfflineOnly,
            &api::CommandKind::Open {
                source: api::Resource {
                    id: 1,
                    length: 5,
                    mime: "text/html".into()
                },
                viewport: api::Viewport {
                    revision: 1,
                    width: 1,
                    height: 1,
                    scale: 1.0
                },
                policy: api::Policy {
                    network: true,
                    scripts: true
                }
            }
        )
        .is_err()
    );
}

#[test]
fn ordinary_html_retains_explicit_online_script_opt_in() {
    let session = endpoint(PolicyCeiling::UserChoice);
    let policy = api::Policy {
        network: true,
        scripts: true,
    };
    assert!(session.policy_change_allowed());
    assert!(session.send(api::CommandKind::Policy(policy)));
    assert_eq!(session.document.load(Ordering::Acquire), 2);
    assert!(matches!(session.signal.pop_command().unwrap().kind,
        api::CommandKind::Policy(value) if value == policy));
    assert_eq!(
        actor::validated_policy(PolicyCeiling::UserChoice, policy).unwrap(),
        policy
    );
}

/// Web viewer API 0.4: scripts may run offline, contained by the provider without any
/// network, and an offline-only preview still cannot enable them.
#[test]
fn offline_scripts_are_a_user_choice_but_never_for_an_offline_only_preview() {
    let offline_scripts = api::Policy {
        network: false,
        scripts: true,
    };
    let session = endpoint(PolicyCeiling::UserChoice);
    assert!(session.send(api::CommandKind::Policy(offline_scripts)));
    assert!(matches!(session.signal.pop_command().unwrap().kind,
        api::CommandKind::Policy(value) if value == offline_scripts));
    assert_eq!(
        actor::validated_policy(PolicyCeiling::UserChoice, offline_scripts).unwrap(),
        offline_scripts
    );
    assert!(actor::validated_policy(PolicyCeiling::OfflineOnly, offline_scripts).is_err());
    let preview = endpoint(PolicyCeiling::OfflineOnly);
    assert!(!preview.send(api::CommandKind::Policy(offline_scripts)));
}

#[test]
fn captured_scope_wrapper_consumes_its_last_underlying_reference() {
    use midnight_plugin_host::{ContentResources, Error};
    use std::sync::atomic::AtomicUsize;
    #[derive(Debug)]
    struct Scope {
        retired: Arc<AtomicUsize>,
    }
    impl ContentResources for Scope {
        fn capture(&self, _: &[String]) -> Result<CapturedFile, Error> {
            Err(Error("No assets".into()))
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
            if Arc::strong_count(&self) == 1 {
                self.retired.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        }
    }
    let retired = Arc::new(AtomicUsize::new(0));
    let scope = Arc::new(CapturedResources(Arc::new(Scope {
        retired: retired.clone(),
    })));
    scope.release_on_worker().unwrap();
    assert_eq!(retired.load(Ordering::Acquire), 1);
}

fn revocation_origin() -> api::Event {
    api::Event {
        header: api::Header {
            session: 1,
            document: 1,
            sequence: 47,
        },
        kind: api::EventKind::ResourceResolve {
            request: 71,
            base: 4,
            url: "/office/v1/catalog.html".into(),
        },
    }
}

fn denied(request: u64) -> api::ResourceReply {
    api::ResourceReply::Denied {
        request,
        reason: "source validation refused".into(),
    }
}

#[test]
fn source_revocation_reports_only_matching_sent_denial_and_preserves_frame_and_input() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    let origin = revocation_origin();
    let frame = api::Event {
        header: api::Header {
            sequence: 46,
            ..origin.header
        },
        kind: api::EventKind::Frame(api::Frame {
            viewport_revision: 2,
            acknowledged_input: 1,
            width: 1,
            height: 1,
            rgba: vec![19, 31, 47, 255],
        }),
    };
    session.output.lock().unwrap().put(frame.clone()).unwrap();
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = wakes.clone();
    let wake: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert!(session.generation.revoke_resources(origin.clone()));
    session.generation.stage_denials(1, &[denied(71)]);
    // The real helper must perform the send before consuming the correlation.
    assert!(
        actor::send_resource_reply(
            &session.generation,
            &session.output,
            &wake,
            1,
            denied(71),
            |_| Err("broken transport".into()),
        )
        .is_err()
    );
    assert!(session.output.lock().unwrap().events.is_empty());
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    // An unrelated request/document denial is not a receipt for the trigger.
    for (document, request) in [(1, 72), (2, 71)] {
        actor::send_resource_reply(
            &session.generation,
            &session.output,
            &wake,
            document,
            denied(request),
            |_| Ok(()),
        )
        .unwrap();
    }
    assert!(session.output.lock().unwrap().events.is_empty());
    assert_eq!(session.output.lock().unwrap().frame, Some(frame.clone()));
    actor::send_resource_reply(
        &session.generation,
        &session.output,
        &wake,
        1,
        denied(71),
        |reply| {
            assert_eq!(reply, denied(71));
            assert!(
                session.output.lock().unwrap().events.is_empty(),
                "receipt preceded successful send"
            );
            Ok(())
        },
    )
    .unwrap();
    assert!(!session.generation.revoke_resources(origin.clone()));
    actor::send_resource_reply(
        &session.generation,
        &session.output,
        &wake,
        1,
        denied(71),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(session.output.lock().unwrap().frame, Some(frame.clone()));
    let events = session.drain(8);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], frame);
    assert_eq!(events[1], origin);
    assert_eq!(events[2].header, origin.header);
    assert!(matches!(&events[2].kind, api::EventKind::Error {
        code: api::ErrorCode::ResourceChanged, message
    } if message == "Source changed; showing previous contents. Reopen to refresh."));
    assert!(session.drain(8).is_empty());
    assert!(session.generation.resources_revoked());
    assert!(!session.closed.load(Ordering::Acquire));
    assert!(
        session.terminal.lock().unwrap().is_none(),
        "source change is not a cleanup failure"
    );
    let commands = [
        api::CommandKind::Find(api::Find {
            query: "already rendered".into(),
            forward: true,
            match_case: false,
            find_next: false,
        }),
        api::CommandKind::Copy,
        api::CommandKind::Resize(api::Viewport {
            revision: 3,
            width: 20,
            height: 30,
            scale: 1.0,
        }),
        api::CommandKind::Zoom(1.5),
    ];
    for command in &commands {
        assert!(session.send(command.clone()));
    }
    let queued: Vec<_> = std::iter::from_fn(|| session.signal.pop_command())
        .map(|command| command.kind)
        .collect();
    assert_eq!(queued, commands);
    assert_eq!(session.document.load(Ordering::Acquire), 1);
    assert!(!session.send(api::CommandKind::Policy(api::Policy::default())));
    session.release_on_worker().unwrap();
}

#[test]
fn ordinary_denial_or_request_echo_alone_is_not_a_source_change_receipt() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    let origin = revocation_origin();
    session.output.lock().unwrap().put(origin.clone()).unwrap();
    let wake: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);
    actor::send_resource_reply(
        &session.generation,
        &session.output,
        &wake,
        1,
        denied(71),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(session.drain(8), [origin]);
    assert!(!session.generation.resources_revoked());
    assert!(!session.closed.load(Ordering::Acquire));
    session.release_on_worker().unwrap();
}

#[test]
fn read_trigger_correlation_is_actual_read_and_waits_for_staged_terminal_denial() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    let origin = api::Event {
        header: revocation_origin().header,
        kind: api::EventKind::ResourceRead {
            request: 91,
            resource: 22,
            offset: 3,
            length: 1,
        },
    };
    let wake: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);
    assert!(session.generation.revoke_resources(origin.clone()));
    actor::send_resource_reply(
        &session.generation,
        &session.output,
        &wake,
        1,
        denied(91),
        |_| Ok(()),
    )
    .unwrap();
    assert!(
        session.drain(8).is_empty(),
        "unstaged denial is not the broker's pending request"
    );
    session.generation.stage_denials(1, &[denied(91)]);
    actor::send_resource_reply(
        &session.generation,
        &session.output,
        &wake,
        1,
        denied(91),
        |_| Ok(()),
    )
    .unwrap();
    let events = session.drain(8);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], origin);
    assert_eq!(events[1].header, origin.header);
    assert!(matches!(
        events[1].kind,
        api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            ..
        }
    ));
    session.release_on_worker().unwrap();
}

#[test]
fn explicit_close_remains_terminal_and_cannot_be_reported_as_source_change() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    session.close();
    assert!(session.closed.load(Ordering::Acquire));
    assert!(!session.generation.revoke_resources(revocation_origin()));
    session.generation.stage_denials(1, &[denied(71)]);
    assert!(session.generation.take_revocation(1, 71).is_none());
    assert!(!session.send(api::CommandKind::Copy));
    assert!(!session.send(api::CommandKind::Zoom(1.5)));
    session.release_on_worker().unwrap();
}

#[test]
fn explicit_close_after_source_revocation_still_stops_local_command_admission() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    assert!(session.generation.revoke_resources(revocation_origin()));
    assert!(session.send(api::CommandKind::Copy));
    session.close();
    assert!(session.closed.load(Ordering::Acquire));
    assert!(!session.send(api::CommandKind::Copy));
    session.release_on_worker().unwrap();
}

#[test]
fn browser_exit_is_published_but_explicit_close_is_not() {
    for requested in [false, true] {
        let session = endpoint(PolicyCeiling::OfflineOnly);
        if requested {
            session.close();
        }
        supervise(
            || Ok(actor::End::Browser),
            &session.output,
            &session.closed,
            &session.document,
            &*session.wake,
        );
        let events = session.drain(8);
        if requested {
            assert!(events.is_empty());
        } else {
            assert!(matches!(
                &events[..],
                [api::Event {
                    kind: api::EventKind::Closed,
                    ..
                }]
            ));
        }
    }
}

#[test]
fn source_change_pair_reserves_two_event_slots_atomically_without_clearing_frame() {
    let origin = revocation_origin();
    let frame = api::Event {
        header: api::Header {
            sequence: 90,
            ..origin.header
        },
        kind: api::EventKind::Frame(api::Frame {
            viewport_revision: 3,
            acknowledged_input: 2,
            width: 1,
            height: 1,
            rgba: vec![29, 53, 79, 255],
        }),
    };
    for occupied in [63, 62] {
        let mut output = Output::default();
        output.put(frame.clone()).unwrap();
        for sequence in 1..=occupied {
            output
                .put(api::Event {
                    header: api::Header {
                        sequence,
                        ..origin.header
                    },
                    kind: api::EventKind::Selection(format!("selection-{sequence}")),
                })
                .unwrap();
        }
        let before = output.events.clone();
        let result = output.put_source_changed(origin.clone());
        if occupied == 63 {
            assert!(result.is_err());
            assert_eq!(output.events, before, "no half correlation pair may escape");
        } else {
            result.unwrap();
            assert_eq!(output.events.len(), 64);
            let at = before.partition_point(|event| {
                (event.header.document, event.header.sequence)
                    <= (origin.header.document, origin.header.sequence)
            });
            assert_eq!(output.events[at], origin);
            assert_eq!(
                output.events[at + 1],
                api::Event {
                    header: origin.header,
                    kind: api::EventKind::Error {
                        code: api::ErrorCode::ResourceChanged,
                        message: "Source changed; showing previous contents. Reopen to refresh."
                            .into(),
                    },
                }
            );
            let mut remaining = output.events.clone();
            remaining.remove(at);
            remaining.remove(at);
            assert_eq!(remaining, before, "all earlier controls retain their order");
            let full = output.events.clone();
            assert!(
                output
                    .put(api::Event {
                        header: api::Header {
                            sequence: 91,
                            ..origin.header
                        },
                        kind: api::EventKind::Ready,
                    })
                    .is_err()
            );
            assert_eq!(output.events, full);
        }
        assert_eq!(output.frame, Some(frame.clone()));
    }
}

#[test]
fn delayed_source_change_pair_keeps_queued_provenance_and_frame_order_across_drains() {
    let session = endpoint(PolicyCeiling::OfflineOnly);
    let origin = revocation_origin();
    let event = |sequence, kind| api::Event {
        header: api::Header {
            sequence,
            ..origin.header
        },
        kind,
    };
    let ready = event(1, api::EventKind::Ready);
    let frame = event(
        60,
        api::EventKind::Frame(api::Frame {
            viewport_revision: 1,
            acknowledged_input: 0,
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        }),
    );
    let newer = event(80, api::EventKind::Selection("newer control".into()));
    {
        let mut output = session.output.lock().unwrap();
        output.put(ready.clone()).unwrap();
        output.put(frame.clone()).unwrap();
        output.put(newer.clone()).unwrap();
        output.put_source_changed(origin.clone()).unwrap();
    }
    // Split immediately after the provenance event. The diagnostic must follow
    // it before the newer retained frame and control, without changing headers.
    assert_eq!(session.drain(2), [ready, origin.clone()]);
    let rest = session.drain(8);
    assert_eq!(rest.len(), 3);
    assert_eq!(rest[0].header, origin.header);
    assert!(matches!(
        &rest[0].kind,
        api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            ..
        }
    ));
    assert_eq!(rest[1], frame);
    assert_eq!(rest[2], newer);
    assert!(session.drain(8).is_empty());
}
