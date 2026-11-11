use super::*;

fn setup() -> (
    HeadlessServer,
    Vec<String>,
    Vec<std::sync::mpsc::Receiver<Vec<u8>>>,
) {
    let mut server = test_headless_server();
    server.app.state.workspaces =
        vec![crate::workspace::Workspace::test_adversarial_identity_state()];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let ids = (0..server.app.state.workspaces[0].tabs.len())
        .map(|idx| server.app.public_tab_id(0, idx).unwrap())
        .collect();
    let (first_control, first_render) = connect_test_shell(&mut server, 71, 80, 24);
    let (second_control, second_render) = connect_test_shell(&mut server, 72, 80, 24);
    (
        server,
        ids,
        vec![first_control, first_render, second_control, second_render],
    )
}

fn request(
    server: &mut HeadlessServer,
    client: Option<u64>,
    method: api::schema::Method,
) -> serde_json::Value {
    let (respond_to, response) = std::sync::mpsc::channel();
    let msg = api::ApiRequestMessage {
        request: api::schema::Request {
            id: "navigation".into(),
            method,
        },
        respond_to,
        response_write_complete: None,
    };
    if let Some(client) = client {
        server.handle_client_shell_api_request(client, msg);
    } else {
        server.handle_api_request_with_shutdown_check(msg);
    }
    serde_json::from_str(&response.try_recv().expect("synchronous response")).unwrap()
}

fn focus(tab_id: &str) -> api::schema::Method {
    api::schema::Method::TabFocus(api::schema::TabTarget {
        tab_id: tab_id.into(),
    })
}

fn last(server: &mut HeadlessServer, client: Option<u64>) -> serde_json::Value {
    let workspace_id = server.app.public_workspace_id(0);
    request(
        server,
        client,
        api::schema::Method::TabLast(api::schema::WorkspaceTarget { workspace_id }),
    )
}

#[tokio::test]
async fn tab_navigation_last_tracks_only_committed_client_visits() {
    let (mut server, ids, _connections) = setup();
    for id in [&ids[0], &ids[1], &ids[2]] {
        request(&mut server, Some(71), focus(id));
    }
    request(&mut server, Some(72), focus(&ids[0]));
    request(&mut server, Some(72), focus(&ids[2]));
    request(&mut server, Some(71), focus(&ids[2]));
    assert!(last(&mut server, Some(71)).get("result").is_some());
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[1].as_str())
    );
    last(&mut server, Some(71));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[2].as_str())
    );
    last(&mut server, Some(72));
    assert_eq!(
        server.shell_tab_id_for_client(72).as_deref(),
        Some(ids[0].as_str())
    );
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_public_focus_commits_each_clients_own_previous_tab() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[0]));
    request(&mut server, Some(72), focus(&ids[2]));
    request(&mut server, None, focus(&ids[1]));
    last(&mut server, Some(71));
    last(&mut server, Some(72));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(72).as_deref(),
        Some(ids[2].as_str())
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_projection_does_not_retarget_public_last_history() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, None, focus(&ids[0]));
    request(&mut server, None, focus(&ids[1]));
    request(&mut server, Some(71), focus(&ids[2]));
    last(&mut server, None);
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(72).as_deref(),
        Some(ids[0].as_str())
    );
    last(&mut server, None);
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[1].as_str())
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_initial_default_is_seeded_before_client_projection() {
    for same_tab_public_focus in [false, true] {
        let (mut server, ids, _connections) = setup();
        let initial = server.shell_tab_id_for_client(71).unwrap();
        if same_tab_public_focus {
            request(&mut server, None, focus(&initial));
        }
        let others = ids.iter().filter(|id| **id != initial).collect::<Vec<_>>();
        request(&mut server, Some(71), focus(others[0]));
        request(&mut server, None, focus(others[1]));
        last(&mut server, None);
        assert_eq!(
            server.shell_tab_id_for_client(71).as_deref(),
            Some(initial.as_str())
        );
        shutdown_test_runtimes(&mut server);
    }
}

#[tokio::test]
async fn tab_navigation_forced_close_replacement_is_current_without_a_visit() {
    let (mut server, ids, _connections) = setup();
    // Structural A,B,C; visits B,A,C; closing C forces B but last must choose A.
    for idx in [1, 0, 2] {
        request(&mut server, None, focus(&ids[idx]));
    }
    request(
        &mut server,
        None,
        api::schema::Method::TabClose(api::schema::TabTarget {
            tab_id: ids[2].clone(),
        }),
    );
    assert_eq!(
        server
            .default_tab_history
            .current(&server.app.public_workspace_id(0)),
        Some(ids[1].as_str())
    );
    last(&mut server, None);
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_failed_preparation_restores_all_workspace_targets_by_id() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[1]));
    let mut other = crate::workspace::Workspace::test_new("other");
    other.test_add_tab(None);
    server.app.state.workspaces.push(other);
    server.app.state.ensure_test_terminals();
    server.reconcile_client_shell_locations();
    let other_original = server.app.public_tab_id(1, 0).unwrap();
    let other_prepared = server.app.public_tab_id(1, 1).unwrap();
    let dead = server.app.state.workspaces[0].tabs[0].layout.pane_ids()[0];
    server
        .app
        .event_tx
        .try_send(AppEvent::PaneDied {
            pane_id: dead,
            exit_reason: crate::platform::ChildExitReason::Exited,
        })
        .unwrap();
    let response = request(
        &mut server,
        Some(71),
        api::schema::Method::CommandInvoke(api::schema::CommandInvokeParams {
            command_id: "missing".into(),
            workspace_id: None,
            tab_id: Some(other_prepared),
            pane_id: None,
            selection: None,
        }),
    );
    assert!(response.get("error").is_some());
    assert!(
        server.app.parse_tab_id(&ids[0]).is_none(),
        "queued event must change tab indices before rollback"
    );
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[1].as_str())
    );
    assert_eq!(
        server
            .app
            .public_tab_id(1, server.app.state.workspaces[1].active_tab_index())
            .as_deref(),
        Some(other_original.as_str())
    );
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_pane_focus_and_removed_targets_use_stable_ids() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[0]));
    let pane_id = server
        .app
        .public_pane_id(0, server.app.state.workspaces[0].tabs[2].root_pane)
        .unwrap();
    request(
        &mut server,
        Some(71),
        api::schema::Method::PaneFocus(api::schema::PaneTarget { pane_id }),
    );
    last(&mut server, Some(71));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    request(
        &mut server,
        Some(71),
        api::schema::Method::TabClose(api::schema::TabTarget {
            tab_id: ids[2].clone(),
        }),
    );
    server.reconcile_client_shell_locations();
    last(&mut server, Some(71));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_deferred_failure_does_not_commit_and_success_does() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[0]));
    request(&mut server, Some(71), focus(&ids[1]));
    for succeeded in [false, true] {
        let boot_id = server.client_shell_boot_id.clone();
        let client = server.clients.get_mut(&71).unwrap();
        client.shell_endpoint_command_in_flight = true;
        client.shell_endpoint_command_surface_revision = Some(client.shell_projection_revision);
        client.shell_deferred_navigation_response = Some(Vec::new());
        let data = if succeeded {
            serde_json::json!({"id":"deferred", "result":{"type":"worktree_opened", "tab":{"tab_id":ids[2]}}})
        } else {
            serde_json::json!({"id":"deferred", "error":{"code":"failed", "message":"failed"}})
        };
        server.handle_server_event(ServerEvent::ClientShellEndpointResponseChunkReady {
            client_id: 71,
            boot_id,
            request_id: "deferred".into(),
            final_chunk: true,
            data: serde_json::to_vec(&data).unwrap(),
        });
        assert_eq!(
            server.shell_tab_id_for_client(71).as_deref(),
            Some(ids[if succeeded { 2 } else { 1 }].as_str())
        );
        last(&mut server, Some(71));
        assert_eq!(
            server.shell_tab_id_for_client(71).as_deref(),
            Some(ids[if succeeded { 1 } else { 0 }].as_str())
        );
        if !succeeded {
            request(&mut server, Some(71), focus(&ids[1]));
        }
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_superseded_deferred_activation_does_not_commit() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[0]));
    request(&mut server, Some(71), focus(&ids[1]));
    let boot_id = server.client_shell_boot_id.clone();
    let client = server.clients.get_mut(&71).unwrap();
    client.shell_endpoint_command_in_flight = true;
    client.shell_endpoint_command_surface_revision = Some(client.shell_projection_revision);
    client.shell_projection_revision += 1;
    client.shell_deferred_navigation_response = Some(Vec::new());
    server.handle_server_event(ServerEvent::ClientShellEndpointResponseChunkReady {
        client_id: 71,
        boot_id,
        request_id: "superseded".into(),
        final_chunk: true,
        data: serde_json::to_vec(
            &serde_json::json!({"result":{"type":"worktree_opened", "tab":{"tab_id": ids[2]}}}),
        )
        .unwrap(),
    });
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[1].as_str())
    );
    last(&mut server, Some(71));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_client_preparation_and_public_focus_preserve_domains() {
    let (mut server, ids, _connections) = setup();
    assert!(request(&mut server, Some(71), focus(&ids[0]))
        .get("result")
        .is_some());
    assert!(request(&mut server, Some(72), focus(&ids[2]))
        .get("result")
        .is_some());
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(72).as_deref(),
        Some(ids[2].as_str())
    );
    server.promote_client_to_foreground(71);
    assert_eq!(
        server.shell_tab_id_for_client(72).as_deref(),
        Some(ids[2].as_str())
    );
    assert!(request(&mut server, None, focus(&ids[1]))
        .get("result")
        .is_some());
    for client in [71, 72] {
        assert_eq!(
            server.shell_tab_id_for_client(client).as_deref(),
            Some(ids[1].as_str())
        );
    }
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn tab_navigation_failed_command_after_preparation_has_error_response() {
    let (mut server, ids, _connections) = setup();
    request(&mut server, Some(71), focus(&ids[0]));
    request(&mut server, Some(71), focus(&ids[1]));
    let response = request(
        &mut server,
        Some(71),
        api::schema::Method::CommandInvoke(api::schema::CommandInvokeParams {
            command_id: "missing-command".into(),
            tab_id: Some(ids[2].clone()),
            workspace_id: None,
            pane_id: None,
            selection: None,
        }),
    );
    assert!(response.get("error").is_some(), "{response}");
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[1].as_str())
    );
    last(&mut server, Some(71));
    assert_eq!(
        server.shell_tab_id_for_client(71).as_deref(),
        Some(ids[0].as_str())
    );
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}
