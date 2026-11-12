//! Exercise real dispatch, including cancellation callers with no input outcome.
use super::*;
use crate::client::endpoint::{EndpointNegotiation, EndpointRegistry, EndpointTransport};
use crate::client::endpoint_commands::EndpointCommands;
use std::sync::{Arc, Mutex};

struct RecordingTransport {
    sent: Arc<Mutex<Vec<ClientMessage>>>,
    fail_api: bool,
}

impl EndpointTransport for RecordingTransport {
    fn send(&mut self, message: &ClientMessage) -> std::io::Result<()> {
        if self.fail_api && matches!(message, ClientMessage::ClientShellEndpointRequest { .. }) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }
}

fn dispatch_actions(
    state: &mut ClientShellState,
    endpoints: &mut EndpointRegistry,
    commands: &mut EndpointCommands,
    actions: Vec<ClientShellAction>,
) {
    crate::client::shell_runtime::dispatch_client_shell_actions(
        actions,
        commands,
        endpoints,
        Some(state),
        &mut Vec::new(),
        &mut None,
    )
    .unwrap();
}

fn sent_events(sent: &Arc<Mutex<Vec<ClientMessage>>>) -> Vec<ClientPaneInputEvent> {
    sent.lock()
        .unwrap()
        .iter()
        .flat_map(|message| match message {
            ClientMessage::ClientShellPaneInput { pane_id, events } => {
                assert_eq!(pane_id, "pane_1");
                events.clone()
            }
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn mouse_history_dispatches_closures_from_callback_send_failure_and_retired_lane() {
    for route in [PaneWheelRoute::MouseReport, PaneWheelRoute::AlternateScroll] {
        for path in ["callback", "send_next", "stale_lane", "retirement"] {
            let (mut state, mut first, second, press) = second_key_wheel(route, false);
            let sent = Arc::new(Mutex::new(Vec::new()));
            let mut endpoints = EndpointRegistry::new(
                RecordingTransport {
                    sent: sent.clone(),
                    fail_api: path == "send_next",
                },
                1,
                EndpointNegotiation::default(),
            );
            let mut commands = EndpointCommands::default();
            if path == "stale_lane" {
                let ClientShellAction::Endpoint {
                    endpoint_id,
                    boot_id,
                    request,
                } = first.pop().unwrap()
                else {
                    panic!("second wheel")
                };
                commands.enqueue(endpoint_id, 0, boot_id, request);
            }
            dispatch_actions(&mut state, &mut endpoints, &mut commands, first);
            match path {
                "callback" => {
                    let (_, actions) = state.handle_endpoint_result(
                        "boot-1",
                        &second,
                        Ok(wheel_result(PaneWheelRoute::Unknown)),
                    );
                    dispatch_actions(&mut state, &mut endpoints, &mut commands, actions);
                }
                "retirement" => {
                    let retired = commands.retire_lane(&ClientEndpointId::Local);
                    assert_eq!(retired, vec![second.clone()]);
                    for id in retired {
                        state.cancel_endpoint_request(&id);
                    }
                    // No new raw input or endpoint completion is needed to flush.
                    dispatch_actions(&mut state, &mut endpoints, &mut commands, Vec::new());
                }
                "send_next" | "stale_lane" => {
                    assert!(!state.pending_requests.contains_key(&second))
                }
                _ => unreachable!(),
            }
            // A failed transport is removed by the registry. Close local
            // ownership, but never bypass the fence to promise a remote release.
            let keys = if path == "send_next" {
                vec![press]
            } else {
                vec![press.clone(), press.with_kind(KeyEventKind::Release)]
            };
            let expected: Vec<_> = keys
                .into_iter()
                .map(|key| ClientPaneInputEvent::from_terminal_key(key).unwrap())
                .collect();
            assert_eq!(sent_events(&sent), expected, "{route:?}, {path}");
            assert!(state.input_leases.is_empty());
            if path == "send_next" {
                assert!(endpoints.connection(&ClientEndpointId::Local).is_none());
                endpoints.insert(
                    ClientEndpointId::Local,
                    RecordingTransport {
                        sent: sent.clone(),
                        fail_api: false,
                    },
                    2,
                    EndpointNegotiation::default(),
                    true,
                );
                dispatch_actions(&mut state, &mut endpoints, &mut commands, Vec::new());
                assert_eq!(
                    sent_events(&sent),
                    expected,
                    "no closure retries on replacement connection"
                );
            }
            let outcome = state.handle_input_bytes(b"q");
            dispatch_actions(&mut state, &mut endpoints, &mut commands, outcome.actions);
            let blur = state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
            assert!(blur
                .requests
                .iter()
                .all(|message| !matches!(message, ClientMessage::ClientShellPaneInput { .. })));
            dispatch_actions(&mut state, &mut endpoints, &mut commands, blur.actions);
            let late = state
                .handle_endpoint_result("boot-1", &second, Ok(wheel_result(route)))
                .1;
            dispatch_actions(&mut state, &mut endpoints, &mut commands, late);
            assert_eq!(
                sent_events(&sent),
                expected,
                "never retry or duplicate closures"
            );
        }
    }
}

#[test]
fn mouse_history_rejected_enqueue_and_invalid_identity_never_replay_input() {
    for invalidation in ["fence", "boot", "pane", "endpoint", "focus"] {
        let (mut state, mut first, second, _) =
            second_key_wheel(PaneWheelRoute::MouseReport, false);
        let endpoint_action = first.pop().unwrap();
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
        assert_eq!(
            sent_events(&sent).len(),
            1,
            "the original press really reached transport"
        );
        let actions = if invalidation == "fence" {
            endpoints.set_surface_active(&ClientEndpointId::Local, false);
            // Rejected admission invokes repaint-only cancellation, then drains
            // the closure through the same closed surface fence.
            vec![endpoint_action]
        } else {
            let (_, actions) = state.handle_endpoint_result(
                "boot-1",
                &second,
                Ok(wheel_result(PaneWheelRoute::Unknown)),
            );
            match invalidation {
                "boot" => {
                    let mut changed = snapshot();
                    changed.boot_id = "replacement".into();
                    state.set_snapshot(Box::new(changed));
                }
                "pane" => {
                    let mut changed = snapshot();
                    changed.revision = 2;
                    changed.panes.clear();
                    state.set_snapshot(Box::new(changed));
                }
                "endpoint" => {
                    state.active_endpoint_id = ClientEndpointId::Ssh(
                        crate::client::endpoint::ProfileId::parse(
                            "0123456789abcdef0123456789abcdef",
                        )
                        .unwrap(),
                    )
                }
                "focus" => {
                    let mut changed = snapshot();
                    let mut pane = changed.panes[0].clone();
                    pane.pane_id = "pane_2".into();
                    changed.panes.push(pane);
                    changed.revision = 2;
                    changed.focused_pane_id = Some("pane_2".into());
                    state.set_snapshot(Box::new(changed));
                }
                _ => unreachable!(),
            }
            actions
        };
        dispatch_actions(&mut state, &mut endpoints, &mut commands, actions);
        assert!(state.input_leases.is_empty());
        let expected = if invalidation == "focus" { 2 } else { 1 };
        assert_eq!(
            sent_events(&sent).len(),
            expected,
            "no closure crosses {invalidation}"
        );
        endpoints.set_surface_active(&ClientEndpointId::Local, true);
        dispatch_actions(&mut state, &mut endpoints, &mut commands, Vec::new());
        assert_eq!(
            sent_events(&sent).len(),
            expected,
            "never replay a rejected closure later"
        );
    }
}

#[test]
fn mouse_history_forwarded_mouse_closes_once_at_transport_on_second_wheel_failure() {
    use crossterm::event::MouseButton;
    for cancelled in [false, true] {
        let mut state = history_state();
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
            mouse(MouseEventKind::Down(MouseButton::Left)),
            wheel(&state, MouseEventKind::ScrollDown),
            mouse(MouseEventKind::Up(MouseButton::Left)),
            RawInputEvent::Paste("uncertain".into()),
        ]));
        let (_, first) = state.handle_endpoint_result(
            "boot-1",
            &id,
            Ok(wheel_result(PaneWheelRoute::MouseReport)),
        );
        let second = first
            .iter()
            .find_map(|action| match action {
                ClientShellAction::Endpoint { request, .. }
                    if matches!(request.method, crate::api::schema::Method::PaneWheel(_)) =>
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
        dispatch_actions(&mut state, &mut endpoints, &mut commands, first);
        assert!(state.pane_mouse_gesture.is_some());
        let actions = if cancelled {
            state.handle_input_bytes(b"\x02").actions
        } else {
            state
                .handle_endpoint_result(
                    "boot-1",
                    &second,
                    Ok(wheel_result(PaneWheelRoute::Unknown)),
                )
                .1
        };
        dispatch_actions(&mut state, &mut endpoints, &mut commands, actions);
        assert!(state.pane_mouse_gesture.is_none());
        assert!(matches!(
            &sent_events(&sent)[..],
            [
                ClientPaneInputEvent::Mouse {
                    kind: crate::protocol::ClientMouseKind::Down(_),
                    ..
                },
                ClientPaneInputEvent::Mouse {
                    kind: crate::protocol::ClientMouseKind::Up(_),
                    ..
                }
            ]
        ));
        let outcome = state.handle_input_bytes(if cancelled { b"q" } else { b"\x1b" });
        dispatch_actions(&mut state, &mut endpoints, &mut commands, outcome.actions);
        let blur = state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
        dispatch_actions(&mut state, &mut endpoints, &mut commands, blur.actions);
        assert_eq!(sent_events(&sent).len(), 2);
    }
}
