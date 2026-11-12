//! Cross-feature contracts at the real input/transport boundary.
use super::*;
use crate::api::schema::Method;
use crate::protocol::endpoint::{EndpointTabSlot, EndpointTabSlots};
use crossterm::event::MouseButton;

fn metadata(snapshot: &ClientShellSnapshot, slot: u32) -> EndpointTabSlots {
    EndpointTabSlots {
        boot_id: snapshot.boot_id.clone(),
        revision: snapshot.revision,
        slots: snapshot
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.tab_id.clone(),
                    EndpointTabSlot {
                        workspace_id: tab.workspace_id.clone(),
                        slot,
                    },
                )
            })
            .collect(),
    }
}

fn install_pair(state: &mut ClientShellState, snapshot: ClientShellSnapshot, slot: u32) {
    let id = ClientEndpointId::Local;
    let snapshot = state
        .set_endpoint_tab_slots_for_generation(&id, 1, metadata(&snapshot, slot))
        .unwrap_or_else(|| Box::new(snapshot));
    let prepared = state
        .prepare_endpoint_slot_snapshot(&id, 1, snapshot, true)
        .unwrap();
    state.install_prepared_endpoint_snapshot(&id, Some(1), prepared, true);
    state.apply_cached_endpoint_snapshot(&id);
}

#[test]
fn mouse_history_combined_navigation_closes_held_input_before_command() {
    for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
        for navigation in ["last", "slot", "pending", "unsupported"] {
            let mut state = history_state();
            state.set_endpoint_methods(Some(vec![
                "pane.wheel".into(),
                "pane.focus".into(),
                "tab.last".into(),
                "tab.focus_slot".into(),
            ]));
            state.set_endpoint_tab_slots_supported(&ClientEndpointId::Local, true);
            install_pair(&mut state, snapshot(), 1);
            let press = reported_key(false);
            let mouse = |kind| {
                RawInputEvent::Mouse(MouseEvent {
                    kind,
                    column: state.hits.panes[0].inner_rect.x,
                    row: state.hits.panes[0].inner_rect.y,
                    modifiers: KeyModifiers::empty(),
                })
            };
            let id = wheel_request(&state.handle_raw_events(vec![
                wheel(&state, MouseEventKind::ScrollUp),
                RawInputEvent::Key(press.clone()),
                mouse(MouseEventKind::Down(MouseButton::Left)),
                wheel(&state, MouseEventKind::ScrollDown),
                RawInputEvent::Key(press.clone().with_kind(KeyEventKind::Release)),
                mouse(MouseEventKind::Up(MouseButton::Left)),
                RawInputEvent::Paste("must not replay".into()),
            ]));
            let (_, first) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
            let second = first
                .iter()
                .find_map(|action| match action {
                    ClientShellAction::Endpoint { request, .. }
                        if matches!(request.method, Method::PaneWheel(_)) =>
                    {
                        Some(request.id.clone())
                    }
                    _ => None,
                })
                .unwrap();
            let sent = Arc::new(Mutex::new(Vec::new()));
            let mut endpoints = EndpointRegistry::new(
                RecordingTransport {
                    sent: sent.clone(),
                    fail_api: false,
                },
                1,
                EndpointNegotiation::default(),
            );
            let mut commands = EndpointCommands::default();
            let preceding: Vec<_> = first
                .iter()
                .filter_map(|action| match action {
                    ClientShellAction::Endpoint { request, .. } if request.id != second => {
                        Some(request.id.clone())
                    }
                    _ => None,
                })
                .collect();
            dispatch_actions(&mut state, &mut endpoints, &mut commands, first);
            for request_id in preceding {
                assert!(commands
                    .receive_chunk(
                        &ClientEndpointId::Local,
                        1,
                        "boot-1",
                        &request_id,
                        true,
                        b"{}".to_vec()
                    )
                    .unwrap()
                    .is_some());
                assert!(commands
                    .send_next(&ClientEndpointId::Local, &mut endpoints)
                    .is_empty());
            }
            let held = if route == PaneWheelRoute::MouseReport {
                2
            } else {
                1
            };
            assert_eq!(sent_events(&sent).len(), held, "{route:?}/{navigation}");
            if navigation == "pending" {
                let mut next = snapshot();
                next.revision = 2;
                state.set_endpoint_snapshot_for_generation(
                    &ClientEndpointId::Local,
                    1,
                    Box::new(next),
                );
            } else if navigation == "unsupported" {
                state.set_endpoint_methods(Some(vec!["pane.wheel".into()]));
            }
            let outcome =
                state.handle_input_bytes(if navigation == "last" || navigation == "unsupported" {
                    b"\x02\x02"
                } else {
                    b"\x021"
                });
            assert!(
                outcome.requests.is_empty(),
                "no pane-byte or legacy binding fallback"
            );
            dispatch_actions(&mut state, &mut endpoints, &mut commands, outcome.actions);
            let events = sent_events(&sent);
            assert_eq!(events.len(), held * 2, "{route:?}/{navigation}: {events:?}");
            assert_eq!(
                events[0],
                ClientPaneInputEvent::from_terminal_key(press.clone()).unwrap()
            );
            assert_eq!(
                events[held],
                ClientPaneInputEvent::from_terminal_key(press.with_kind(KeyEventKind::Release))
                    .unwrap()
            );
            if route == PaneWheelRoute::MouseReport {
                assert!(matches!(
                    events[1],
                    ClientPaneInputEvent::Mouse {
                        kind: crate::protocol::ClientMouseKind::Down(_),
                        ..
                    }
                ));
                assert!(matches!(
                    events[3],
                    ClientPaneInputEvent::Mouse {
                        kind: crate::protocol::ClientMouseKind::Up(_),
                        ..
                    }
                ));
            }
            assert!(state.pane_mouse_gesture.is_none());
            // Navigation keys can retain consumed leases until physical release.
            // Blurring must not emit any additional child release.
            let blur = state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
            assert!(blur
                .requests
                .iter()
                .all(|request| !matches!(request, ClientMessage::ClientShellPaneInput { .. })));
            dispatch_actions(&mut state, &mut endpoints, &mut commands, blur.actions);
            assert!(state.input_leases.is_empty());
            assert_eq!(sent_events(&sent), events);
            // The command lane serializes navigation behind the outstanding wheel.
            // Finish that wheel, then send the queued navigation through transport.
            assert!(commands
                .receive_chunk(
                    &ClientEndpointId::Local,
                    1,
                    "boot-1",
                    &second,
                    true,
                    b"{}".to_vec()
                )
                .unwrap()
                .is_some());
            dispatch_actions(&mut state, &mut endpoints, &mut commands, Vec::new());
            let late = state
                .handle_endpoint_result("boot-1", &second, Ok(wheel_result(route)))
                .1;
            dispatch_actions(&mut state, &mut endpoints, &mut commands, late);
            assert_eq!(
                sent_events(&sent),
                events,
                "uncertain input and late completion stay inert"
            );
            let methods: Vec<_> = sent
                .lock()
                .unwrap()
                .iter()
                .filter_map(|message| match message {
                    ClientMessage::ClientShellEndpointRequest { request, .. } => Some(
                        serde_json::from_str::<serde_json::Value>(request).unwrap()["method"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    ),
                    _ => None,
                })
                .collect();
            let expected = match navigation {
                "last" => vec!["pane.focus", "pane.wheel", "tab.last"],
                "slot" => vec!["pane.focus", "pane.wheel", "tab.focus_slot"],
                _ => vec!["pane.focus", "pane.wheel"],
            };
            assert_eq!(methods, expected);
            if matches!(navigation, "last" | "slot") {
                let messages = sent.lock().unwrap();
                let navigation_index = messages
                    .iter()
                    .rposition(|message| {
                        matches!(message, ClientMessage::ClientShellEndpointRequest { .. })
                    })
                    .unwrap();
                let closure_index = messages
                    .iter()
                    .rposition(|message| {
                        matches!(message, ClientMessage::ClientShellPaneInput { .. })
                    })
                    .unwrap();
                assert!(
                    closure_index < navigation_index,
                    "closures precede navigation on transport"
                );
            }
            if navigation == "pending" {
                let mut next = snapshot();
                next.revision = 2;
                install_pair(&mut state, next, 1);
            }
            if navigation == "pending" {
                let next = state.handle_input_bytes(b"\x021");
                assert!(next.requests.is_empty());
                assert!(
                    matches!(&next.actions[..], [ClientShellAction::Endpoint { request, .. }]
                    if matches!(&request.method, Method::TabFocusSlot(params)
                        if params.workspace_id == "ws_1" && params.slot == 1))
                );
                dispatch_actions(&mut state, &mut endpoints, &mut commands, next.actions);
                let messages = sent.lock().unwrap();
                let Some(ClientMessage::ClientShellEndpointRequest { request, .. }) =
                    messages.last()
                else {
                    panic!("focus_slot transport");
                };
                let request: serde_json::Value = serde_json::from_str(request).unwrap();
                assert_eq!(request["method"], "tab.focus_slot");
                assert_eq!(request["params"]["workspace_id"], "ws_1");
                assert_eq!(request["params"]["slot"], 1);
            }
        }
    }
}

#[test]
fn mouse_history_combined_secondary_prefix_tap_balances_after_capture_cancel() {
    let (mut state, first, second, press) = second_key_wheel(PaneWheelRoute::MouseReport, false);
    let mut config = Config::default();
    config.keys.prefix = crate::config::BindingConfig::Many(vec!["ctrl+a".into(), "ctrl+b".into()]);
    config.keys.send_prefix = crate::config::BindingConfig::one("prefix+x");
    state.config = ClientShellConfig::from_config(&config);
    state.set_host_reports_key_releases(true);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut endpoints = EndpointRegistry::new(
        RecordingTransport {
            sent: sent.clone(),
            fail_api: false,
        },
        1,
        EndpointNegotiation::default(),
    );
    let mut commands = EndpointCommands::default();
    dispatch_actions(&mut state, &mut endpoints, &mut commands, first);
    let prefix = state.handle_input_bytes(b"\x02");
    dispatch_actions(&mut state, &mut endpoints, &mut commands, prefix.actions);
    assert_eq!(
        sent_events(&sent),
        [press.clone(), press.with_kind(KeyEventKind::Release)]
            .into_iter()
            .map(|key| ClientPaneInputEvent::from_terminal_key(key).unwrap())
            .collect::<Vec<_>>()
    );
    let tap = state.handle_input_bytes(b"x");
    assert!(tap.actions.is_empty());
    assert!(
        matches!(&tap.requests[..], [ClientMessage::ClientShellPaneInput { pane_id, events }]
        if pane_id == "pane_1" && matches!(&events[..], [
            ClientPaneInputEvent::Key { code: crate::protocol::ClientKeyCode::Char('b'), kind: crate::protocol::ClientKeyKind::Press, .. },
            ClientPaneInputEvent::Key { code: crate::protocol::ClientKeyCode::Char('b'), kind: crate::protocol::ClientKeyKind::Release, .. }]))
    );
    assert!(state
        .handle_endpoint_result(
            "boot-1",
            &second,
            Ok(wheel_result(PaneWheelRoute::MouseReport))
        )
        .1
        .is_empty());
}

#[test]
fn mouse_history_combined_delayed_slot_pair_preserves_parked_copy_and_app_target() {
    for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
        for focus_first in [false, true] {
            let mut state = history_state();
            let mut old = snapshot();
            let mut pane = old.panes[0].clone();
            pane.pane_id = "pane_2".into();
            old.panes.push(pane);
            old.focused_pane_id = Some("pane_2".into());
            state.set_endpoint_tab_slots_supported(&ClientEndpointId::Local, true);
            install_pair(&mut state, old.clone(), 1);
            let mut hit = state.hits.panes[0].clone();
            hit.pane_id = "pane_2".into();
            state.enter_copy_mode_from_hit(hit, &mut ClientShellInput::default());
            let before = state.copy_mode.clone().unwrap();
            let id = wheel_request(
                &state.handle_raw_events(vec![wheel(&state, MouseEventKind::ScrollUp)]),
            );
            state.handle_input_bytes(b"\x04");
            let mut focused = old.clone();
            focused.revision = 3;
            focused.focused_pane_id = Some("pane_1".into());
            if focus_first {
                state.set_endpoint_snapshot_for_generation(
                    &ClientEndpointId::Local,
                    1,
                    Box::new(focused.clone()),
                );
                assert_eq!(state.snapshot.as_ref().unwrap().revision, 1);
                install_pair(&mut state, focused.clone(), 20);
            }
            let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(wheel_result(route)));
            assert_eq!(captured_events(&actions).len(), 1);
            if !focus_first {
                old.revision = 2;
                install_pair(&mut state, old, 1);
                state.set_endpoint_snapshot_for_generation(
                    &ClientEndpointId::Local,
                    1,
                    Box::new(focused.clone()),
                );
                assert_eq!(state.snapshot.as_ref().unwrap().revision, 2);
            }
            for event in [
                RawInputEvent::Key(reported_key(false)),
                RawInputEvent::Text(crate::input::TextCommit::new("continued")),
                RawInputEvent::Paste("continued".into()),
            ] {
                let outcome = state.handle_raw_events(vec![event]);
                assert!(
                    matches!(&outcome.requests[..], [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1")
                );
            }
            if !focus_first {
                install_pair(&mut state, focused, 20);
            }
            assert_eq!(state.mode, ClientShellMode::Terminal);
            assert_eq!(state.copy_mode.as_ref(), Some(&before));
            assert_eq!(state.snapshot.as_ref().unwrap().revision, 3);
            assert_eq!(
                state.presented_tab_slots.as_ref().unwrap()["tab_1"].slot,
                20
            );
        }
    }
}

#[test]
fn mouse_history_combined_late_pair_installer_cannot_open_input_fence() {
    struct NoHostInput;
    impl crate::platform::PrefixInputSource for NoHostInput {
        fn switch_to_ascii(&mut self) {}
        fn restore(&mut self) {}
    }
    for surface_active in [false, true] {
        let (mut shell, first, second, _) = second_key_wheel(PaneWheelRoute::MouseReport, false);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut endpoints = EndpointRegistry::new(
            RecordingTransport {
                sent: sent.clone(),
                fail_api: false,
            },
            1,
            EndpointNegotiation::default(),
        );
        let mut commands = EndpointCommands::default();
        dispatch_actions(&mut shell, &mut endpoints, &mut commands, first);
        assert_eq!(sent_events(&sent).len(), 1);
        sent.lock().unwrap().clear();
        shell.set_endpoint_tab_slots_supported(&ClientEndpointId::Local, true);
        let mut next = snapshot();
        next.revision = 2;
        let slots = metadata(&next, 20);
        assert!(shell
            .prepare_endpoint_slot_snapshot(&ClientEndpointId::Local, 1, Box::new(next), false)
            .is_none());
        assert_eq!(shell.snapshot.as_ref().unwrap().revision, 1);
        endpoints.set_surface_active(&ClientEndpointId::Local, surface_active);
        endpoints.freeze_input();
        let pending = shell
            .set_endpoint_tab_slots_for_generation(&ClientEndpointId::Local, 1, slots)
            .expect("late companion returns retained snapshot");
        let prepared = shell
            .prepare_endpoint_slot_snapshot(&ClientEndpointId::Local, 1, pending, false)
            .unwrap();
        let mut client = crate::client::state::ClientState::test_new();
        client.shell = Some(shell);
        crate::client::shell_runtime::install_client_shell_snapshot(
            &mut client,
            &ClientEndpointId::Local,
            prepared,
            true,
            &mut endpoints,
            &mut NoHostInput,
        )
        .unwrap();
        assert!(!endpoints.active_surface_available());
        let shell = client.shell.as_mut().unwrap();
        assert_eq!(
            shell.snapshot.as_ref().unwrap().revision,
            1,
            "unprojected while activation pending"
        );
        assert!(shell.presented_tab_slots.is_none());
        assert_eq!(shell.endpoints[0].snapshot.as_ref().unwrap().revision, 2);
        let (_, closures) = shell.handle_endpoint_result(
            "boot-1",
            &second,
            Ok(wheel_result(PaneWheelRoute::Unknown)),
        );
        dispatch_actions(shell, &mut endpoints, &mut commands, closures);
        assert!(
            sent_events(&sent).is_empty(),
            "closure cannot cross the input fence"
        );
        endpoints.set_surface_active(&ClientEndpointId::Local, true);
        endpoints.unfreeze_input();
        shell.apply_cached_endpoint_snapshot(&ClientEndpointId::Local);
        assert_eq!(shell.snapshot.as_ref().unwrap().revision, 2);
        assert_eq!(
            shell.presented_tab_slots.as_ref().unwrap()["tab_1"].slot,
            20
        );
        dispatch_actions(shell, &mut endpoints, &mut commands, Vec::new());
        assert!(
            sent_events(&sent).is_empty(),
            "never replay a dropped closure after activation"
        );
    }
}
