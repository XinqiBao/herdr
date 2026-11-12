//! Adversarial combinations found during the frozen client review.
use super::*;
use crossterm::event::KeyEventKind;

#[path = "mouse_history_dispatch.rs"]
mod dispatch;

fn reported_key(native: bool) -> crate::input::TerminalKey {
    let key = crate::input::TerminalKey::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
    if native {
        key.with_windows_record(crate::input::WindowsKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: 0x44,
            virtual_scan_code: 0x20,
            unicode: 0,
            control_key_state: 0x0008,
        })
    } else {
        key.with_vt_bytes(b"\x1b[100;5u".to_vec())
    }
}

fn captured_events(actions: &[ClientShellAction]) -> Vec<ClientPaneInputEvent> {
    actions
        .iter()
        .flat_map(|action| match action {
            ClientShellAction::CapturedInput {
                message: ClientMessage::ClientShellPaneInput { pane_id, events },
                ..
            } => {
                assert_eq!(pane_id, "pane_1", "never retarget a release or replay");
                events.clone()
            }
            _ => Vec::new(),
        })
        .collect()
}

fn second_key_wheel(
    route: PaneWheelRoute,
    native: bool,
) -> (
    ClientShellState,
    Vec<ClientShellAction>,
    String,
    crate::input::TerminalKey,
) {
    let mut state = history_state();
    let press = reported_key(native);
    let id = wheel_request(&state.handle_raw_events(vec![
        wheel(&state, MouseEventKind::ScrollUp),
        RawInputEvent::Key(press.clone()),
        wheel(&state, MouseEventKind::ScrollDown),
        RawInputEvent::Key(press.clone().with_kind(KeyEventKind::Repeat)),
        RawInputEvent::Key(press.clone().with_kind(KeyEventKind::Release)),
        RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
        )),
        RawInputEvent::Text(crate::input::TextCommit::new("uncertain")),
        RawInputEvent::Paste("uncertain paste".into()),
    ]));
    let (_, first) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
    let second = first
        .iter()
        .find_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.id.clone()),
            _ => None,
        })
        .unwrap();
    (state, first, second, press)
}

#[test]
fn mouse_history_second_wheel_failure_closes_forwarded_key_only() {
    for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
        for native in [false, true] {
            for failure in [
                "unknown", "timeout", "error", "events", "bytes", "prefix", "outside", "focus",
                "cancel",
            ] {
                let (mut state, first, second, press) = second_key_wheel(route, native);
                let closed = match failure {
                    "unknown" => {
                        state
                            .handle_endpoint_result(
                                "boot-1",
                                &second,
                                Ok(wheel_result(PaneWheelRoute::Unknown)),
                            )
                            .1
                    }
                    "timeout" | "error" => {
                        state
                            .handle_endpoint_result(
                                "boot-1",
                                &second,
                                Err(ClientShellEndpointError {
                                    code: Some(
                                        if failure == "timeout" {
                                            "endpoint_timeout"
                                        } else {
                                            "rejected"
                                        }
                                        .into(),
                                    ),
                                    message: "test failure".into(),
                                }),
                            )
                            .1
                    }
                    "events" => {
                        let mut actions = Vec::new();
                        for _ in 0..257 {
                            actions.extend(state.handle_input_bytes(b"z").actions);
                        }
                        actions
                    }
                    "bytes" => {
                        state
                            .handle_raw_events(vec![RawInputEvent::Paste("z".repeat(64 * 1024))])
                            .actions
                    }
                    "prefix" => state.handle_input_bytes(b"\x02").actions,
                    "outside" => {
                        state
                            .handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
                                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                                column: 0,
                                row: 0,
                                modifiers: KeyModifiers::empty(),
                            })])
                            .actions
                    }
                    "focus" => {
                        state
                            .handle_raw_events(vec![RawInputEvent::OuterFocusLost])
                            .actions
                    }
                    "cancel" => {
                        state.cancel_endpoint_request(&second);
                        state.take_history_input_closures()
                    }
                    _ => unreachable!(),
                };
                let actual: Vec<_> = captured_events(&first)
                    .into_iter()
                    .chain(captured_events(&closed))
                    .collect();
                let expected: Vec<_> = [press.clone(), press.with_kind(KeyEventKind::Release)]
                    .into_iter()
                    .map(|key| ClientPaneInputEvent::from_terminal_key(key).unwrap())
                    .collect();
                assert_eq!(
                    actual, expected,
                    "{route:?}, native={native}, failure={failure}"
                );
                // The deliberate prefix can own its new consumed press; the
                // old forwarded key and all uncertain new keys are gone.
                assert_eq!(
                    state.input_leases.len(),
                    usize::from(failure == "prefix"),
                    "{route:?}, native={native}, failure={failure}"
                );
                let dismissed = state.handle_input_bytes(b"q");
                assert!(captured_events(&dismissed.actions).is_empty());
                let blur = state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
                assert!(blur
                    .requests
                    .iter()
                    .all(|message| !matches!(message, ClientMessage::ClientShellPaneInput { .. })));
                assert!(captured_events(&blur.actions).is_empty());
                assert!(captured_events(
                    &state
                        .handle_endpoint_result("boot-1", &second, Ok(wheel_result(route)))
                        .1
                )
                .is_empty());
            }
        }
    }
}

#[test]
fn mouse_history_failed_capture_q_or_escape_does_not_duplicate_closure() {
    for dismiss in [b"q".as_slice(), b"\x1b".as_slice()] {
        let (mut state, _, second, _) = second_key_wheel(PaneWheelRoute::MouseReport, false);
        let closed = state
            .handle_endpoint_result("boot-1", &second, Ok(wheel_result(PaneWheelRoute::Unknown)))
            .1;
        assert_eq!(captured_events(&closed).len(), 1);
        let outcome = state.handle_input_bytes(dismiss);
        assert!(outcome.requests.is_empty());
        assert!(captured_events(&outcome.actions).is_empty());
        let blur = state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
        assert!(blur
            .requests
            .iter()
            .all(|message| !matches!(message, ClientMessage::ClientShellPaneInput { .. })));
        assert!(captured_events(&blur.actions).is_empty());
    }
}

#[test]
fn mouse_history_app_replay_parks_old_copy_before_focus_projection() {
    for route in [
        PaneWheelRoute::MouseReport,
        PaneWheelRoute::AlternateScroll,
        PaneWheelRoute::HostScroll,
    ] {
        for focus_first in [false, true] {
            for search_pending in [false, true] {
                let mut state = history_state();
                // pane_2 is A, retained in Copy; the visible wheel target pane_1 is B.
                let mut old = snapshot();
                let mut pane = old.panes[0].clone();
                pane.pane_id = "pane_2".into();
                old.panes[0].focused = false;
                old.panes.push(pane);
                old.focused_pane_id = Some("pane_2".into());
                state.set_snapshot(Box::new(old.clone()));
                let mut hit = state.hits.panes[0].clone();
                hit.pane_id = "pane_2".into();
                state.enter_copy_mode_from_hit(hit, &mut ClientShellInput::default());
                state.copy_mode.as_mut().unwrap().selection =
                    Some(ClientCopySelection::Linewise { anchor_row: 19 });
                state.sync_copy_selection();
                state.set_endpoint_methods(Some(vec![
                    "pane.wheel".into(),
                    "pane.scroll".into(),
                    "pane.focus".into(),
                    "pane.copy_motion".into(),
                    "pane.copy_search".into(),
                ]));
                let motion = if search_pending {
                    state
                        .copy_operation_queue
                        .push_back(ClientCopyOperation::Search {
                            query: "needle".into(),
                            direction: crate::api::schema::PaneCopySearchDirection::Forward,
                            repeat: false,
                        });
                    let mut outcome = ClientShellInput::default();
                    state.dispatch_next_copy_operation(&mut outcome);
                    outcome
                } else {
                    state.handle_raw_events(vec![RawInputEvent::Key(
                        crate::input::TerminalKey::new(KeyCode::Char('w'), KeyModifiers::empty()),
                    )])
                };
                let motion_id = motion
                    .actions
                    .iter()
                    .find_map(|action| match action {
                        ClientShellAction::Endpoint { request, .. }
                            if matches!(
                                request.method,
                                crate::api::schema::Method::PaneCopyMotion(_)
                                    | crate::api::schema::Method::PaneCopySearch(_)
                            ) =>
                        {
                            Some(request.id.clone())
                        }
                        _ => None,
                    })
                    .expect("old A motion is pending");
                assert!(state.copy_operation_in_flight);
                let before = state.copy_mode.as_ref().unwrap().clone();
                let direction = if route == PaneWheelRoute::HostScroll {
                    MouseEventKind::ScrollDown
                } else {
                    MouseEventKind::ScrollUp
                };
                let id = wheel_request(&state.handle_raw_events(vec![wheel(&state, direction)]));
                state.handle_input_bytes(b"\x04q");
                state.handle_raw_events(vec![
                    RawInputEvent::Text(crate::input::TextCommit::new("text")),
                    RawInputEvent::Paste("paste".into()),
                ]);
                let mut focused = old.clone();
                focused.revision = 2;
                focused.focused_pane_id = Some("pane_1".into());
                if focus_first {
                    state.set_snapshot(Box::new(focused.clone()));
                }
                let mut result = wheel_result(route);
                if route == PaneWheelRoute::HostScroll {
                    let crate::api::schema::ResponseResult::PaneWheel { wheel } = &mut result
                    else {
                        unreachable!()
                    };
                    wheel.after.offset_from_bottom = 0;
                }
                let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(result));
                let events = captured_events(&actions);
                assert_eq!(
                    events.len(),
                    4,
                    "{route:?}, focus_first={focus_first}: {events:?}"
                );
                assert!(
                    matches!(&events[2], ClientPaneInputEvent::TextCommit(text) if text == "text")
                );
                assert!(matches!(&events[3], ClientPaneInputEvent::Paste(text) if text == "paste"));
                assert_eq!(state.mode, ClientShellMode::Terminal);
                if !focus_first {
                    old.revision = 2;
                    state.set_snapshot(Box::new(old));
                    assert_eq!(
                        state.mode,
                        ClientShellMode::Terminal,
                        "stale A focus must not resume Copy"
                    );
                    for event in [
                        RawInputEvent::Key(reported_key(false)),
                        RawInputEvent::Key(crate::input::TerminalKey::new(
                            KeyCode::Char('q'),
                            KeyModifiers::empty(),
                        )),
                        RawInputEvent::Text(crate::input::TextCommit::new("next text")),
                        RawInputEvent::Paste("next".into()),
                    ] {
                        let next = state.handle_raw_events(vec![event]);
                        assert!(
                            matches!(&next.requests[..], [ClientMessage::ClientShellPaneInput {pane_id, ..}] if pane_id == "pane_1")
                        );
                    }
                    focused.revision = 3;
                    state.set_snapshot(Box::new(focused));
                }
                let after = state.copy_mode.as_ref().expect("A remains parked");
                assert_eq!(after, &before, "the entire retained A session is unchanged");
                let cursor = crate::api::schema::PaneTextPoint {
                    row: before.cursor.row,
                    col: 3,
                };
                let late_result = if search_pending {
                    crate::api::schema::ResponseResult::PaneCopySearch {
                        pane_id: "pane_2".into(),
                        content_revision: before.content_revision,
                        matches: vec![crate::api::schema::PaneTextRange {
                            start: cursor,
                            end: cursor,
                        }],
                        total: 1,
                        current: Some(0),
                        current_global: Some(0),
                    }
                } else {
                    crate::api::schema::ResponseResult::PaneCopyMotion {
                        pane_id: "pane_2".into(),
                        cursor,
                        content_revision: before.content_revision,
                    }
                };
                let late = state.handle_endpoint_result("boot-1", &motion_id, Ok(late_result));
                assert!(late.1.is_empty());
                assert_eq!(
                    state.copy_mode.as_ref(),
                    Some(&before),
                    "old A callbacks cannot mutate the parked session"
                );
            }
        }
    }
}
