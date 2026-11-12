use super::*;
use crate::api::schema::{PaneScrollInfo, PaneWheelResult, PaneWheelRoute};

#[path = "mouse_history_followups.rs"]
mod followups;

fn history_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec![
        "pane.wheel".into(),
        "pane.scroll".into(),
        "pane.focus".into(),
    ]));
    let mut projection = surface();
    projection.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: 0,
        max_offset_from_bottom: 20,
        viewport_rows: 2,
    });
    state.set_pane_surface(projection);
    state.compose(106, 20).unwrap();
    state
}

fn wheel(state: &ClientShellState, kind: MouseEventKind) -> RawInputEvent {
    let hit = &state.hits.panes[0];
    RawInputEvent::Mouse(MouseEvent {
        kind,
        column: hit.inner_rect.x,
        row: hit.inner_rect.y,
        modifiers: KeyModifiers::empty(),
    })
}

fn wheel_request(outcome: &ClientShellInput) -> String {
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            ClientShellAction::Endpoint { request, .. }
                if matches!(request.method, crate::api::schema::Method::PaneWheel(_)) =>
            {
                Some(request.id.clone())
            }
            _ => None,
        })
        .expect("wheel endpoint request")
}

fn wheel_result(route: PaneWheelRoute) -> crate::api::schema::ResponseResult {
    crate::api::schema::ResponseResult::PaneWheel {
        wheel: PaneWheelResult {
            pane_id: "pane_1".into(),
            route,
            before: PaneScrollInfo {
                offset_from_bottom: 0,
                max_offset_from_bottom: 20,
                viewport_rows: 2,
            },
            after: PaneScrollInfo {
                offset_from_bottom: 3,
                max_offset_from_bottom: 20,
                viewport_rows: 2,
            },
            cols: 4,
            rows: 2,
            alternate_screen_active: false,
            content_revision: 0,
        },
    }
}

#[test]
fn mouse_history_captures_immediate_ctrl_d_before_async_reply() {
    for same_batch in [true, false] {
        let mut state = history_state();
        let mut events = vec![wheel(&state, MouseEventKind::ScrollUp)];
        if same_batch {
            events.extend(crate::raw_input::parse_framed_input(b"\x04"));
        }
        let outcome = state.handle_raw_events(events);
        let id = wheel_request(&outcome);
        assert!(outcome.requests.is_empty());
        if !same_batch {
            assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
        }
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &id,
            Ok(wheel_result(PaneWheelRoute::HostScroll)),
        );
        assert_eq!(state.mode, ClientShellMode::Copy);
        let copy = state.copy_mode.as_ref().unwrap();
        assert_eq!(copy.entry_offset_from_bottom, 0);
        assert!(copy.offset_from_bottom < 3);
        assert!(!actions
            .iter()
            .any(|action| matches!(action, ClientShellAction::CapturedInput { .. })));
    }
}

#[test]
fn mouse_history_exit_invalidates_reply_without_replaying_pending_keys() {
    let mut state = history_state();
    let event = wheel(&state, MouseEventKind::ScrollUp);
    let id = wheel_request(&state.handle_raw_events(vec![event]));
    assert!(state.handle_input_bytes(b"\x04q").requests.is_empty());
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::HostScroll)));
    assert!(!actions
        .iter()
        .any(|action| matches!(action, ClientShellAction::CapturedInput { .. })));
    assert!(state.copy_mode.is_none());
}

#[test]
fn mouse_history_unknown_outcome_never_replays_navigation() {
    let mut state = history_state();
    let event = wheel(&state, MouseEventKind::ScrollUp);
    let id = wheel_request(&state.handle_raw_events(vec![event]));
    state.handle_input_bytes(b"\x04");
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::Unknown)));
    assert!(actions.is_empty());
    assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
    assert!(state.wheel_capture.as_ref().unwrap().failed);
}

#[test]
fn mouse_history_application_queue_keeps_wheel_key_wheel_text_paste_order() {
    for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
        let mut state = history_state();
        let events = vec![
            wheel(&state, MouseEventKind::ScrollUp),
            RawInputEvent::Key(crate::input::TerminalKey::new(
                KeyCode::Char('q'),
                KeyModifiers::empty(),
            )),
            wheel(&state, MouseEventKind::ScrollDown),
            RawInputEvent::Text(crate::input::TextCommit::new("text")),
            RawInputEvent::Paste("paste".into()),
        ];
        let id = wheel_request(&state.handle_raw_events(events));
        let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
        assert!(
            matches!(&actions[..], [ClientShellAction::CapturedInput { message: ClientMessage::ClientShellPaneInput {pane_id, events}, .. }, ClientShellAction::Endpoint {request, .. }]
            if pane_id == "pane_1" && events.len() == 1 && matches!(request.method, crate::api::schema::Method::PaneWheel(_)))
        );
        let second_id = actions
            .iter()
            .find_map(|action| match action {
                ClientShellAction::Endpoint { request, .. } => Some(request.id.clone()),
                _ => None,
            })
            .unwrap();
        let (_, actions) =
            state.handle_endpoint_result("boot-1", &second_id, Ok(wheel_result(route)));
        assert!(
            matches!(&actions[..], [ClientShellAction::CapturedInput {message: ClientMessage::ClientShellPaneInput {events: text, ..}, ..}, ClientShellAction::CapturedInput {message: ClientMessage::ClientShellPaneInput {events: paste, ..}, ..}]
            if matches!(&text[..], [ClientPaneInputEvent::TextCommit(value)] if value == "text") && matches!(&paste[..], [ClientPaneInputEvent::Paste(value)] if value == "paste"))
        );
        assert!(state.copy_mode.is_none());
    }
}

#[test]
fn mouse_history_queued_click_uses_copy_selection_or_application_gesture() {
    for route in [PaneWheelRoute::HostScroll, PaneWheelRoute::MouseReport] {
        let mut state = history_state();
        let hit = state.hits.panes[0].clone();
        let click = RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: hit.inner_rect.x,
            row: hit.inner_rect.y,
            modifiers: KeyModifiers::empty(),
        });
        let events = vec![wheel(&state, MouseEventKind::ScrollUp), click];
        let id = wheel_request(&state.handle_raw_events(events));
        let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
        let sent_mouse = actions.iter().any(|action| matches!(action, ClientShellAction::CapturedInput {message: ClientMessage::ClientShellPaneInput {events, ..}, ..}
            if events.iter().any(|event| matches!(event, ClientPaneInputEvent::Mouse {..}))));
        assert_eq!(sent_mouse, route == PaneWheelRoute::MouseReport);
        if route == PaneWheelRoute::HostScroll {
            assert!(state.selection.is_some());
            assert!(state.copy_mode.is_some());
        } else {
            assert!(state.pane_mouse_gesture.is_some());
        }
    }
}

#[test]
fn mouse_history_timeout_and_overflow_remain_captured_until_explicit_exit() {
    for overflow in [false, true] {
        let mut state = history_state();
        let event = wheel(&state, MouseEventKind::ScrollUp);
        let id = wheel_request(&state.handle_raw_events(vec![event]));
        if overflow {
            for _ in 0..257 {
                assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
            }
        } else {
            let (_, actions) = state.handle_endpoint_result(
                "boot-1",
                &id,
                Err(ClientShellEndpointError {
                    code: Some("endpoint_timeout".into()),
                    message: "timeout".into(),
                }),
            );
            assert!(actions.is_empty());
        }
        assert!(state.wheel_capture.as_ref().unwrap().failed);
        assert!(state.handle_input_bytes(b"\x04q").requests.is_empty());
        assert!(state.wheel_capture.is_none());
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &id,
            Ok(wheel_result(PaneWheelRoute::HostScroll)),
        );
        assert!(actions.is_empty());
        assert!(state.copy_mode.is_none());
    }
}

#[test]
fn mouse_history_prefix_and_pane_removal_invalidate_pending_reply() {
    for remove in [false, true] {
        let mut state = history_state();
        let event = wheel(&state, MouseEventKind::ScrollUp);
        let id = wheel_request(&state.handle_raw_events(vec![event]));
        state.handle_input_bytes(b"\x04");
        if remove {
            let mut next = snapshot();
            next.revision = 2;
            next.panes.clear();
            state.set_snapshot(Box::new(next));
        } else {
            state.handle_input_bytes(b"\x02");
            assert_eq!(state.mode, ClientShellMode::Prefix);
        }
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &id,
            Ok(wheel_result(PaneWheelRoute::HostScroll)),
        );
        assert!(actions.is_empty());
        assert!(state.copy_mode.is_none());
    }
}

#[test]
fn mouse_history_copy_wheel_stays_captured_at_bottom_and_in_projection_gap() {
    let mut state = history_state();
    let event = wheel(&state, MouseEventKind::ScrollUp);
    let id = wheel_request(&state.handle_raw_events(vec![event]));
    state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::HostScroll)));
    let event = wheel(&state, MouseEventKind::ScrollDown);
    let outcome = state.handle_raw_events(vec![event]);
    assert!(outcome.requests.is_empty());
    assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 0);
    state.hits.panes.clear();
    state.pane_surface = None;
    assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
    assert_eq!(state.mode, ClientShellMode::Copy);
}

#[test]
fn mouse_history_application_held_keys_release_once_across_next_wheel() {
    use crossterm::event::KeyEventKind;
    for native in [false, true] {
        for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
            let mut state = history_state();
            let press = crate::input::TerminalKey::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
            let press = if native {
                press.with_windows_record(crate::input::WindowsKeyRecord {
                    key_down: true,
                    repeat_count: 1,
                    virtual_key_code: 0x44,
                    virtual_scan_code: 0x20,
                    unicode: 0,
                    control_key_state: 0x0008,
                })
            } else {
                press.with_vt_bytes(b"\x1b[100;5u".to_vec())
            };
            let repeat = press.clone().with_kind(KeyEventKind::Repeat);
            let release = press.clone().with_kind(KeyEventKind::Release);
            let expected: Vec<_> = [press.clone(), repeat.clone(), release.clone()]
                .into_iter()
                .map(|key| ClientPaneInputEvent::from_terminal_key(key).unwrap())
                .collect();
            let id = wheel_request(&state.handle_raw_events(vec![
                wheel(&state, MouseEventKind::ScrollUp),
                RawInputEvent::Key(press),
                wheel(&state, MouseEventKind::ScrollDown),
                RawInputEvent::Key(repeat),
                RawInputEvent::Key(release),
            ]));
            let (_, first) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
            let second_id = first
                .iter()
                .find_map(|action| match action {
                    ClientShellAction::Endpoint { request, .. } => Some(request.id.clone()),
                    _ => None,
                })
                .unwrap();
            let (_, second) =
                state.handle_endpoint_result("boot-1", &second_id, Ok(wheel_result(route)));
            let actual: Vec<_> = first
                .into_iter()
                .chain(second)
                .filter_map(|action| match action {
                    ClientShellAction::CapturedInput {
                        message: ClientMessage::ClientShellPaneInput { pane_id, events },
                        ..
                    } => {
                        assert_eq!(pane_id, "pane_1");
                        Some(events)
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(actual, expected, "native={native}, route={route:?}");
            assert!(state
                .handle_raw_events(vec![RawInputEvent::OuterFocusLost])
                .requests
                .iter()
                .all(|message| !matches!(message, ClientMessage::ClientShellPaneInput { .. })));
        }
    }
}

#[test]
fn mouse_history_closes_previous_lease_then_captures_held_ctrl_d_repeat() {
    use crossterm::event::KeyEventKind;
    let mut state = history_state();
    let press = crate::input::TerminalKey::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        .with_physical_identity_hint(true);
    assert!(!state
        .handle_raw_events(vec![RawInputEvent::Key(press.clone())])
        .requests
        .is_empty());
    let outcome = state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]);
    assert!(outcome.requests.is_empty());
    assert!(
        matches!(outcome.actions.first(), Some(ClientShellAction::CapturedInput {message: ClientMessage::ClientShellPaneInput {events, ..}, ..})
        if matches!(&events[..], [ClientPaneInputEvent::Key {kind: crate::protocol::ClientKeyKind::Release, ..}]))
    );
    let id = wheel_request(&outcome);
    assert!(state
        .handle_raw_events(vec![
            RawInputEvent::Key(press.clone().with_kind(KeyEventKind::Repeat)),
            RawInputEvent::Key(press.with_kind(KeyEventKind::Release))
        ])
        .requests
        .is_empty());
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::HostScroll)));
    assert!(!actions
        .iter()
        .any(|action| matches!(action, ClientShellAction::CapturedInput { .. })));
    assert_eq!(state.mode, ClientShellMode::Copy);
    assert!(state.copy_mode.as_ref().unwrap().offset_from_bottom < 3);
}

#[test]
fn mouse_history_waits_for_target_focus_without_parking_or_retargeting() {
    for focus_first in [false, true] {
        for route in [PaneWheelRoute::HostScroll, PaneWheelRoute::MouseReport] {
            let mut state = history_state();
            let mut old = snapshot();
            old.focused_pane_id = Some("pane_2".into());
            old.panes[0].focused = false;
            let mut pane = old.panes[0].clone();
            pane.pane_id = "pane_2".into();
            pane.focused = true;
            old.panes.push(pane);
            state.set_snapshot(Box::new(old.clone()));
            let outcome = state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]);
            assert!(
                matches!(outcome.actions.first(), Some(ClientShellAction::Endpoint {request, ..})
                if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1"))
            );
            let id = wheel_request(&outcome);
            state.handle_input_bytes(b"\x04");
            if focus_first {
                let mut next = snapshot();
                next.revision = 2;
                state.set_snapshot(Box::new(next));
            }
            let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
            if route == PaneWheelRoute::HostScroll {
                if !focus_first {
                    old.revision = 2;
                    state.set_snapshot(Box::new(old));
                }
                assert_eq!(state.mode, ClientShellMode::Copy);
                assert_eq!(state.copy_mode.as_ref().unwrap().pane_id, "pane_1");
            } else {
                assert!(
                    matches!(&actions[..], [ClientShellAction::CapturedInput {message: ClientMessage::ClientShellPaneInput {pane_id, ..}, ..}] if pane_id == "pane_1")
                );
            }
        }
    }
}

#[test]
fn mouse_history_scrollbar_captures_thumb_track_and_release_offset() {
    use crossterm::event::MouseButton;
    for thumb_click in [false, true] {
        let mut state = history_state();
        let hit = &mut state.hits.panes[0];
        let track = Rect::new(hit.inner_rect.x + 3, hit.inner_rect.y, 1, 2);
        hit.scrollbar_rect = Some(track);
        let metrics = hit.scroll.unwrap();
        let thumb = crate::ui::scrollbar_thumb(metrics, track).unwrap();
        let event = |kind, row| {
            RawInputEvent::Mouse(MouseEvent {
                kind,
                column: track.x,
                row,
                modifiers: KeyModifiers::empty(),
            })
        };
        let outcome = state.handle_raw_events(vec![event(
            MouseEventKind::Down(MouseButton::Left),
            if thumb_click { thumb.top } else { track.y },
        )]);
        assert_eq!(state.mode, ClientShellMode::Copy);
        assert_eq!(
            state.copy_mode.as_ref().unwrap().entry_offset_from_bottom,
            0
        );
        assert!(outcome.requests.is_empty());
        if thumb_click {
            assert!(state.chrome_drag.is_some());
            state.handle_raw_events(vec![event(
                MouseEventKind::Drag(MouseButton::Left),
                track.y,
            )]);
            // The final release must update local copy state even if a drag
            // request is still in flight/coalesced.
            state.handle_raw_events(vec![event(
                MouseEventKind::Up(MouseButton::Left),
                track.y + 1,
            )]);
            assert!(state.chrome_drag.is_none());
            assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 0);
        }
        assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
        assert_eq!(state.mode, ClientShellMode::Copy);
    }
}

#[test]
fn mouse_history_old_endpoint_retains_legacy_wheel_with_local_notice() {
    let mut state = history_state();
    state.set_endpoint_methods(Some(vec!["pane.scroll".into()]));
    let outcome = state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]);
    assert!(state.wheel_capture.is_none());
    assert!(state.copy_mode.is_none());
    assert!(!outcome.requests.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
}

#[test]
fn mouse_history_initial_wheel_follows_preceding_key_and_lease_release() {
    let mut state = history_state();
    let key = crate::input::TerminalKey::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
    let outcome = state.handle_raw_events(vec![
        RawInputEvent::Key(key),
        wheel(&state, MouseEventKind::ScrollUp),
    ]);
    assert!(outcome.requests.is_empty());
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::CapturedInput {
        message: ClientMessage::ClientShellPaneInput {events, ..}, ..}, ClientShellAction::Endpoint {request, ..}]
        if matches!(&events[..], [ClientPaneInputEvent::Key {kind: crate::protocol::ClientKeyKind::Press, ..},
            ClientPaneInputEvent::Key {kind: crate::protocol::ClientKeyKind::Release, ..}])
            && matches!(request.method, crate::api::schema::Method::PaneWheel(_)))
    );
}

#[test]
fn mouse_history_pending_pointer_motion_cannot_cancel_keyboard_capture() {
    let mut state = history_state();
    let id = wheel_request(&state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]));
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.handle_input_bytes(b"\x04").requests.is_empty());
    state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::HostScroll)));
    assert_eq!(state.mode, ClientShellMode::Copy);
}

#[test]
fn mouse_history_application_keeps_pending_escape_and_rejects_stale_target() {
    let mut state = history_state();
    let id = wheel_request(&state.handle_raw_events(vec![
        wheel(&state, MouseEventKind::ScrollUp),
        RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Esc,
            KeyModifiers::empty(),
        )),
    ]));
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::MouseReport)));
    let ClientShellAction::CapturedInput {
        endpoint_id,
        boot_id,
        message,
    } = &actions[0]
    else {
        panic!("captured Escape");
    };
    assert!(
        matches!(message, ClientMessage::ClientShellPaneInput {events, ..}
        if matches!(&events[..], [ClientPaneInputEvent::Key {code: crate::protocol::ClientKeyCode::Esc, ..}]))
    );
    assert!(state.captured_input_is_current(endpoint_id, boot_id, message));
    assert!(!state.captured_input_is_current(endpoint_id, "old-boot", message));
    let mut next = snapshot();
    next.revision = 2;
    next.panes.clear();
    state.set_snapshot(Box::new(next));
    assert!(!state.captured_input_is_current(endpoint_id, boot_id, message));
}

#[test]
fn mouse_history_replays_original_pixel_payload_and_restores_current_host_pixels() {
    use crate::input::mouse::{HostGeometry, HostPixels};
    let mut state = history_state();
    let hit = &mut state.hits.panes[0];
    hit.sgr_pixel_mouse = true;
    hit.pixel_width = u32::from(hit.inner_rect.width) * 10;
    hit.pixel_height = u32::from(hit.inner_rect.height) * 20;
    let x = u32::from(hit.inner_rect.x) * 10 + 5;
    let y = u32::from(hit.inner_rect.y) * 20 + 7;
    let geometry = HostGeometry::new(106, 20, 1060, 400).unwrap();
    let outcome = state.handle_pixel_mouse(format!("\x1b[<64;{x};{y}M").as_bytes(), geometry);
    let id = wheel_request(&outcome);
    state.handle_pixel_mouse(format!("\x1b[<65;{};{y}M", x + 1).as_bytes(), geometry);
    let current = Some(HostPixels {
        x: 1000,
        y: 390,
        geometry,
    });
    state.host_mouse_pixels = current;
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::MouseReport)));
    assert_eq!(state.host_mouse_pixels, current);
    assert!(
        matches!(&actions[..], [ClientShellAction::Endpoint {request, ..}]
        if matches!(&request.method, crate::api::schema::Method::PaneWheel(params)
            if params.pixels.is_some_and(|pixels| pixels.x == 6 && pixels.y == 7)))
    );
}

#[test]
fn mouse_history_explicit_prefix_clears_focus_wait_after_host_result() {
    let mut state = history_state();
    let mut old = snapshot();
    old.focused_pane_id = Some("pane_2".into());
    old.panes[0].focused = false;
    let mut pane = old.panes[0].clone();
    pane.pane_id = "pane_2".into();
    pane.focused = true;
    old.panes.push(pane);
    state.set_snapshot(Box::new(old.clone()));
    let id = wheel_request(&state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]));
    state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(PaneWheelRoute::HostScroll)));
    assert!(state.history_focus_target.is_some());
    state.handle_input_bytes(b"\x02");
    assert!(state.history_focus_target.is_none());
    old.revision = 2;
    state.set_snapshot(Box::new(old));
    state.handle_input_bytes(b"\x1b");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.copy_mode.is_some());
}
