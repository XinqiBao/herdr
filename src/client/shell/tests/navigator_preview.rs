use super::*;
use crate::api::schema::{PaneReadResult, ReadFormat, ReadSource, ResponseResult};
use std::time::{Duration, Instant};

fn prepared() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut projection = snapshot();
    let mut second = projection.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projection.panes.push(second);
    state.set_snapshot(Box::new(projection));
    state.set_pane_surface(surface());
    state.set_endpoint_methods_for(
        &ClientEndpointId::Local,
        Some(
            crate::server::client_commands::supported_client_shell_method_names()
                .iter()
                .map(|name| (*name).into())
                .collect(),
        ),
    );
    state.open_navigator_overlay();
    state
}

fn select(
    state: &mut ClientShellState,
    endpoint_id: ClientEndpointId,
    pane_id: &str,
    now: Instant,
) {
    let Some(ClientShellOverlay::Navigator(n)) = state.overlay.as_mut() else {
        panic!("navigator")
    };
    n.selected = Some(ClientNavigatorTarget::Pane {
        endpoint_id,
        pane_id: pane_id.into(),
    });
    let mut outcome = ClientShellInput::default();
    state.refresh_navigator_preview(now, &mut outcome);
    assert!(outcome.requests.is_empty());
    assert!(outcome.actions.is_empty());
}

fn start(state: &mut ClientShellState, now: Instant) -> (String, Vec<ClientShellAction>) {
    let mut outcome = ClientShellInput::default();
    state.tick_navigator_preview(now, false, &mut outcome);
    let [ClientShellAction::NavigatorPreview { request_id, .. }] = outcome.actions.as_slice()
    else {
        panic!("one read action: {:?}", outcome.actions)
    };
    assert!(outcome.requests.is_empty());
    (request_id.clone(), outcome.actions)
}

fn read(pane_id: &str, text: &str) -> ResponseResult {
    ResponseResult::PaneRead {
        read: PaneReadResult {
            pane_id: pane_id.into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            source: ReadSource::Visible,
            format: ReadFormat::Text,
            text: text.into(),
            revision: 0,
            truncated: false,
        },
    }
}

fn preview(state: &ClientShellState) -> &super::super::navigator_preview::NavigatorPreview {
    let Some(ClientShellOverlay::Navigator(n)) = state.overlay.as_ref() else {
        panic!("navigator")
    };
    &n.preview
}

fn working_projection() -> ClientShellSnapshot {
    let mut projection = snapshot();
    let mut second = projection.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projection.panes.push(second);
    projection.agents = projection
        .panes
        .iter()
        .map(|pane| ClientShellAgent {
            pane_id: pane.pane_id.clone(),
            workspace_id: pane.workspace_id.clone(),
            tab_id: pane.tab_id.clone(),
            name: None,
            display_agent: None,
            agent: Some("test".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Working,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: pane.focused,
        })
        .collect();
    projection
}

fn effective_selection(state: &ClientShellState) -> Option<ClientNavigatorTarget> {
    let Some(ClientShellOverlay::Navigator(n)) = &state.overlay else {
        panic!("navigator")
    };
    let rows = render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, n);
    super::super::aggregate_navigation::selected_navigator_target(&rows, n)
}

#[test]
fn filtered_snapshot_changes_reject_late_preview_before_the_next_tick() {
    for explicit_selection in [false, true] {
        let mut state = prepared();
        let mut projection = working_projection();
        state.set_snapshot(Box::new(projection.clone()));
        state.handle_input_bytes(b"w");
        let now = Instant::now();
        if explicit_selection {
            select(&mut state, ClientEndpointId::Local, "pane_1", now);
        }
        let (old, _) = start(&mut state, now + Duration::from_secs(1));
        projection.revision += 1;
        projection.agents[0].agent_status = AgentStatus::Idle;
        state.set_snapshot(Box::new(projection));
        let expected = (!explicit_selection).then_some(ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_2".into(),
        });
        assert_eq!(effective_selection(&state), expected);
        // The runtime polls completed reads before ticking the preview.
        assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "STALE"))));
        assert!(preview(&state).lines.is_empty());
        if !explicit_selection {
            let (new, _) = start(&mut state, now + Duration::from_secs(2));
            assert!(state.complete_navigator_preview(&new, Ok(read("pane_2", "CURRENT"))));
            let accepted = state.handle_input_bytes(b"\r");
            assert!(
                matches!(accepted.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
                if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"))
            );
        }
    }
}

#[test]
fn another_endpoints_snapshot_can_replace_the_filtered_fallback() {
    use crate::client::endpoint::{ProfileId, SavedSshEndpoint};
    let mut state = prepared();
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Remote".into(),
        target: "example".into(),
        session: "named-remote".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    state.cache_endpoint_snapshot_inactive_for_generation(
        &remote,
        7,
        Box::new(working_projection()),
    );
    state.handle_input_bytes(b"w");
    assert_eq!(
        effective_selection(&state),
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: remote,
            pane_id: "pane_1".into(),
        })
    );
    let now = Instant::now();
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    let mut local = working_projection();
    local.revision += 1;
    state.set_snapshot(Box::new(local));
    assert_eq!(
        effective_selection(&state),
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_1".into(),
        })
    );
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "REMOTE_STALE"))));
    let (new, _) = start(&mut state, now + Duration::from_secs(2));
    assert!(state.complete_navigator_preview(&new, Ok(read("pane_1", "LOCAL_CURRENT"))));
    assert_eq!(preview(&state).lines, ["LOCAL_CURRENT"]);
}

#[test]
fn asynchronous_fallback_round_trip_retires_the_first_selection_epoch() {
    let mut state = prepared();
    let mut projection = working_projection();
    state.set_snapshot(Box::new(projection.clone()));
    state.handle_input_bytes(b"w");
    let now = Instant::now();
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    for status in [AgentStatus::Idle, AgentStatus::Working] {
        projection.revision += 1;
        projection.agents[0].agent_status = status;
        state.set_snapshot(Box::new(projection.clone()));
    }
    assert_eq!(
        effective_selection(&state),
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_1".into(),
        })
    );
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "FIRST_VISIT"))));
    let (new, _) = start(&mut state, now + Duration::from_secs(2));
    assert_ne!(new, old);
}

#[test]
fn endpoint_availability_round_trip_retires_pending_preview_before_tick() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Reconnecting);
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Online);
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "OLD_CONNECTION"))));
}

#[test]
fn an_empty_filter_starts_preview_when_a_matching_terminal_appears() {
    let mut state = prepared();
    state.handle_input_bytes(b"w");
    assert_eq!(effective_selection(&state), None);
    let now = Instant::now();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator_preview(now + Duration::from_secs(1), false, &mut outcome);
    assert!(outcome.actions.is_empty());
    state.set_snapshot(Box::new(working_projection()));
    let (_, actions) = start(&mut state, now + Duration::from_secs(2));
    assert!(
        matches!(actions.as_slice(), [ClientShellAction::NavigatorPreview { pane_id, .. }] if pane_id == "pane_1")
    );
}

#[test]
fn asynchronous_query_membership_changes_retire_the_old_preview() {
    let mut state = prepared();
    let mut projection = working_projection();
    for pane in &mut projection.panes {
        pane.label = Some("matching".into());
    }
    state.set_snapshot(Box::new(projection.clone()));
    state.handle_input_bytes(b"/matching");
    let now = Instant::now();
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    projection.revision += 1;
    projection.panes[0].label = Some("renamed".into());
    state.set_snapshot(Box::new(projection));
    assert_eq!(
        effective_selection(&state),
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_2".into(),
        })
    );
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "RENAMED"))));
}

#[test]
fn preview_selection_resolution_ignores_output_only_revisions_and_frames() {
    let mut state = prepared();
    let mut projection = working_projection();
    state.set_snapshot(Box::new(projection.clone()));
    state.handle_input_bytes(b"w");
    let now = Instant::now();
    let (id, _) = start(&mut state, now + Duration::from_secs(1));
    let resolutions = preview(&state).row_resolutions;
    for revision in 2..22 {
        projection.revision = revision;
        projection.agents[0].tokens = vec![("output".into(), revision.to_string())];
        projection.agents[0].state_change_seq = revision;
        projection.agents[0].terminal_title = Some(format!("raw title {revision}"));
        projection.focused_pane_id = Some("pane_2".into());
        projection.panes[0].focused = false;
        projection.agents[0].focused = false;
        state.set_snapshot(Box::new(projection.clone()));
        let mut next_surface = surface();
        next_surface.projection_revision = revision;
        next_surface.surface_revision = revision;
        state.set_pane_surface(next_surface);
        let mut outcome = ClientShellInput::default();
        state.tick_navigator_preview(now + Duration::from_secs(2), true, &mut outcome);
        assert!(outcome.actions.is_empty());
        state.compose(120, 30);
        assert_eq!(preview(&state).row_resolutions, resolutions);
    }
    // Relevant metadata on another row resolves membership once without retiring unchanged A.
    projection.revision += 1;
    projection.panes[1].label = Some("renamed".into());
    state.set_snapshot(Box::new(projection));
    assert_eq!(preview(&state).row_resolutions, resolutions + 1);
    assert!(state.complete_navigator_preview(&id, Ok(read("pane_1", "STILL_CURRENT"))));
}

#[test]
#[ignore = "manual metadata-delivery scaling profile"]
fn navigator_preview_metadata_scale_profile() {
    const ITERATIONS: u64 = 5_000;
    for panes in [1, 15] {
        let mut projection = working_projection();
        let pane = projection.panes[0].clone();
        let agent = projection.agents[0].clone();
        projection.panes = (1..=panes)
            .map(|index| {
                let mut pane = pane.clone();
                pane.pane_id = format!("pane_{index}");
                pane.focused = index == 1;
                pane
            })
            .collect();
        projection.agents = (1..=panes)
            .map(|index| {
                let mut agent = agent.clone();
                agent.pane_id = format!("pane_{index}");
                agent.focused = index == 1;
                agent
            })
            .collect();
        for relevant in [false, true] {
            for overlay_open in [false, true] {
                let mut samples = Vec::new();
                for _ in 0..3 {
                    let mut state = prepared();
                    state.set_snapshot(Box::new(projection.clone()));
                    state.compose(120, 30);
                    if overlay_open {
                        state.handle_input_bytes(b"w");
                    } else {
                        state.handle_input_bytes(b"\x1b");
                    }
                    let resolutions = if overlay_open {
                        preview(&state).row_resolutions
                    } else {
                        0
                    };
                    let started = Instant::now();
                    for revision in 2..ITERATIONS + 2 {
                        projection.revision = revision;
                        if relevant {
                            projection.panes[0].label = Some(revision.to_string());
                        }
                        state.cache_endpoint_snapshot_inactive_for_generation(
                            &ClientEndpointId::Local,
                            1,
                            Box::new(projection.clone()),
                        );
                    }
                    samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / ITERATIONS as f64);
                    if overlay_open && !relevant {
                        // First delivery changes the test connection generation only.
                        assert_eq!(preview(&state).row_resolutions, resolutions + 1);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!("goto metadata panes={panes} relevant={relevant} overlay={overlay_open} geometry=120x30 median_us_per_delivery={:.3}", samples[1]);
            }
        }
    }
}

#[test]
fn preview_debounces_coalesces_and_leaves_focus_to_enter() {
    let mut state = prepared();
    let before = state.snapshot.clone();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let mut outcome = ClientShellInput::default();
    state.tick_navigator_preview(now + Duration::from_millis(99), false, &mut outcome);
    assert!(outcome.actions.is_empty());
    let (old, _) = start(&mut state, now + Duration::from_millis(100));
    for offset in 0..40 {
        select(
            &mut state,
            ClientEndpointId::Local,
            "pane_2",
            now + Duration::from_millis(101 + offset),
        );
        state.tick_navigator_preview(now + Duration::from_secs(1), true, &mut outcome);
    }
    assert!(outcome.actions.is_empty(), "no FIFO preview backlog");
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "STALE"))));
    assert!(preview(&state).lines.is_empty());
    let (latest, _) = start(&mut state, now + Duration::from_secs(2));
    assert!(state.complete_navigator_preview(&latest, Ok(read("pane_2", "SELECTED"))));
    assert_eq!(preview(&state).lines, ["SELECTED"]);
    assert_eq!(state.snapshot, before);
    let accepted = state.handle_input_bytes(b"\r");
    assert!(
        matches!(accepted.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"))
    );
    assert!(accepted.requests.is_empty());
}

#[test]
fn returning_to_a_pane_starts_a_new_selection_epoch() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    select(
        &mut state,
        ClientEndpointId::Local,
        "pane_2",
        now + Duration::from_secs(2),
    );
    select(
        &mut state,
        ClientEndpointId::Local,
        "pane_1",
        now + Duration::from_secs(3),
    );
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "STALE FIRST VISIT"))));
    assert!(preview(&state).lines.is_empty());
    let mut blocked = ClientShellInput::default();
    state.tick_navigator_preview(now + Duration::from_secs(4), true, &mut blocked);
    assert!(blocked.actions.is_empty());
    let (new, _) = start(&mut state, now + Duration::from_secs(5));
    assert_ne!(old, new);
    state.complete_navigator_preview(&new, Ok(read("pane_1", "FRESH SECOND VISIT")));
    assert_eq!(preview(&state).lines, ["FRESH SECOND VISIT"]);
}

#[test]
fn cancel_and_reopen_discard_late_content_and_keep_the_original_view() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let (old, _) = start(&mut state, now + Duration::from_secs(1));
    let before = state.snapshot.clone();
    let cancelled = state.handle_input_bytes(b"\x1b");
    assert!(cancelled.actions.is_empty());
    assert!(cancelled.requests.is_empty());
    assert_eq!(state.snapshot, before);
    state.open_navigator_overlay();
    select(
        &mut state,
        ClientEndpointId::Local,
        "pane_1",
        now + Duration::from_secs(2),
    );
    let mut blocked = ClientShellInput::default();
    state.tick_navigator_preview(now + Duration::from_secs(3), true, &mut blocked);
    assert!(
        blocked.actions.is_empty(),
        "reopen must retain the worker's real busy slot"
    );
    assert!(!state.complete_navigator_preview(&old, Ok(read("pane_1", "OLD SESSION"))));
    let (new, _) = start(&mut state, now + Duration::from_secs(4));
    assert_ne!(old, new);
    assert!(state.complete_navigator_preview(&new, Ok(read("pane_1", "NEW SESSION"))));
    assert_eq!(preview(&state).lines, ["NEW SESSION"]);
}

#[test]
fn offline_unsupported_and_removed_pane_updates_invalidate_only_the_preview() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_2", now);
    let (id, _) = start(&mut state, now + Duration::from_secs(1));
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Reconnecting);
    let mut outcome = ClientShellInput::default();
    state.tick_navigator_preview(now + Duration::from_secs(2), false, &mut outcome);
    assert!(preview(&state).message.unwrap().contains("offline"));
    assert!(!state.complete_navigator_preview(&id, Ok(read("pane_2", "OLD"))));
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Online);
    state.tick_navigator_preview(now + Duration::from_secs(3), false, &mut outcome);
    let [ClientShellAction::NavigatorPreview { request_id, .. }] = outcome.actions.as_slice()
    else {
        panic!("one read on reconnect: {:?}", outcome.actions)
    };
    let id = request_id.clone();
    outcome.actions.clear();
    state.complete_navigator_preview(
        &id,
        Err(ClientShellEndpointError {
            code: Some("unsupported_method".into()),
            message: "unsupported".into(),
        }),
    );
    assert!(preview(&state).message.unwrap().contains("unsupported"));
    let mut projection = snapshot();
    projection.revision += 1;
    state.set_snapshot(Box::new(projection));
    state.tick_navigator_preview(now + Duration::from_secs(4), false, &mut outcome);
    assert!(preview(&state).lines.is_empty());
    assert!(!preview(&state).message.unwrap().contains("unsupported"));
    assert!(outcome.actions.is_empty());
}

#[test]
fn preview_sanitizes_controls_and_clips_unicode_to_its_byte_budget() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let (id, _) = start(&mut state, now + Duration::from_secs(1));
    let text = format!("line\r\t\u{1b}\u{7}\u{9b}\n{}", "界".repeat(100_000));
    assert!(state.complete_navigator_preview(&id, Ok(read("pane_1", &text))));
    assert_eq!(preview(&state).lines[0], "line");
    assert!(preview(&state).lines.iter().map(String::len).sum::<usize>() <= 256 * 1024);
    assert!(!preview(&state)
        .lines
        .iter()
        .flat_map(|line| line.chars())
        .any(char::is_control));
}

#[test]
fn composed_preview_stacks_in_narrow_layouts_and_keeps_list_hits_separate() {
    let mut state = prepared();
    let now = Instant::now();
    select(&mut state, ClientEndpointId::Local, "pane_1", now);
    let (id, _) = start(&mut state, now + Duration::from_secs(1));
    state.complete_navigator_preview(&id, Ok(read("pane_1", "PREVIEW_MARKER\nsecond row")));
    for (width, height) in [(120, 30), (60, 24), (28, 16), (20, 11)] {
        let frame = state
            .compose(width, height)
            .expect("supported overlay geometry");
        if height > 11 {
            let marker = frame
                .cells
                .windows("PREVIEW_MARKER".len())
                .position(|cells| {
                    cells
                        .iter()
                        .map(|cell| cell.symbol.as_str())
                        .collect::<String>()
                        == "PREVIEW_MARKER"
                })
                .unwrap();
            let x = (marker % usize::from(width)) as u16;
            let y = (marker / usize::from(width)) as u16;
            assert!(!state
                .hits
                .navigator_rows
                .iter()
                .any(|(rect, _)| rect.contains((x, y).into())));
            let text = frame
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>();
            assert!(text.contains("PREVIEW_MARKER"));
        }
        assert!(!state.hits.navigator_rows.is_empty());
    }
}

#[tokio::test]
async fn runtime_reads_inactive_endpoint_without_input_and_rejects_stale_or_cancelled_results() {
    use crate::client::endpoint::{
        EndpointNegotiation, EndpointRegistry, EndpointTransport, ProfileId, SavedSshEndpoint,
    };
    use crate::client::endpoint_commands::EndpointCommands;
    use crate::client::navigator_preview::PreviewReader;
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::{Arc, Mutex};

    struct Capture(Arc<Mutex<Vec<ClientMessage>>>);
    impl EndpointTransport for Capture {
        fn send(&mut self, message: &ClientMessage) -> std::io::Result<()> {
            self.0.lock().unwrap().push(message.clone());
            Ok(())
        }
    }

    for mode in ["valid", "generation", "cancel"] {
        let mut state = prepared();
        let profile = SavedSshEndpoint {
            id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            label: "Remote".into(),
            target: "example".into(),
            session: "named-remote".into(),
            enabled: true,
        };
        let remote = ClientEndpointId::Ssh(profile.id.clone());
        state.set_endpoint_catalog(std::slice::from_ref(&profile));
        state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
        let mut projection = snapshot();
        projection.boot_id = "remote-boot".into();
        state.set_endpoint_snapshot_for_generation(&remote, 7, Box::new(projection));
        let supported = || {
            crate::server::client_commands::supported_client_shell_method_names()
                .iter()
                .map(|name| (*name).into())
                .collect()
        };
        state.set_endpoint_methods_for(&remote, Some(supported()));
        let local_messages = Arc::new(Mutex::new(Vec::new()));
        let remote_messages = Arc::new(Mutex::new(Vec::new()));
        let negotiation = || EndpointNegotiation::new(supported(), Vec::new());
        let mut endpoints =
            EndpointRegistry::new(Capture(local_messages.clone()), 1, negotiation());
        endpoints.insert(
            remote.clone(),
            Capture(remote_messages.clone()),
            7,
            negotiation(),
            false,
        );
        let path =
            std::env::temp_dir().join(format!("herdr-goto-{}-{mode}.sock", std::process::id()));
        let listener = crate::ipc::bind_private_local_listener(&path).unwrap();
        let (requested, request_seen) = std::sync::mpsc::sync_channel(1);
        let (release, released) = std::sync::mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let request: crate::api::schema::Request = serde_json::from_str(&line).unwrap();
            assert!(
                matches!(&request.method, crate::api::schema::Method::PaneRead(params)
                if params.source == ReadSource::Visible && params.format == ReadFormat::Text && params.strip_ansi && params.lines == Some(200) && params.pane_id == "pane_1")
            );
            requested.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            serde_json::to_writer(
                &mut stream,
                &crate::api::schema::SuccessResponse {
                    id: request.id,
                    result: read("pane_1", "REMOTE_MATCH"),
                },
            )
            .unwrap();
            stream.write_all(b"\n").unwrap();
        });
        let mut reader = PreviewReader::with_test_path(path.clone());
        reader.set_profiles(vec![profile]);
        let now = Instant::now();
        select(&mut state, remote.clone(), "pane_1", now);
        let (_, actions) = start(&mut state, now + Duration::from_secs(1));
        let mut commands = EndpointCommands::default();
        let mut scheduled = None;
        crate::client::shell_runtime::dispatch_client_shell_actions(
            actions,
            &mut commands,
            &mut endpoints,
            Some(&mut state),
            &mut Vec::new(),
            &mut scheduled,
            &mut reader,
        )
        .unwrap();
        request_seen.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(reader.is_busy());
        assert!(remote_messages.lock().unwrap().is_empty());
        assert_eq!(endpoints.active_id(), &ClientEndpointId::Local);
        assert!(!endpoints.connection(&remote).unwrap().surface_active);
        assert!(scheduled.is_none());
        // Foreground focus is sent immediately while the optional read is still blocked.
        let actions = state.focus_endpoint_target(ClientEndpointFocusTarget::Pane("pane_1".into()));
        crate::client::shell_runtime::dispatch_client_shell_actions(
            actions,
            &mut commands,
            &mut endpoints,
            Some(&mut state),
            &mut Vec::new(),
            &mut scheduled,
            &mut reader,
        )
        .unwrap();
        assert_eq!(local_messages.lock().unwrap().len(), 1);
        if mode == "generation" {
            endpoints.insert(
                remote.clone(),
                Capture(remote_messages.clone()),
                8,
                negotiation(),
                false,
            );
        } else if mode == "cancel" {
            state.handle_input_bytes(b"\x1b");
        }
        release.send(()).unwrap();
        let mut outcome = ClientShellInput::default();
        let deadline = Instant::now() + Duration::from_secs(3);
        while reader.is_busy() && Instant::now() < deadline {
            reader.poll(&mut state, &endpoints, Instant::now(), &mut outcome);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(!reader.is_busy());
        if mode == "valid" {
            assert_eq!(preview(&state).lines, ["REMOTE_MATCH"]);
        } else if mode == "generation" {
            assert!(preview(&state).lines.is_empty());
        } else {
            assert!(state.overlay.is_none());
        }
        assert!(remote_messages.lock().unwrap().is_empty());
        assert!(outcome.actions.is_empty());
        assert!(outcome.requests.is_empty());
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
