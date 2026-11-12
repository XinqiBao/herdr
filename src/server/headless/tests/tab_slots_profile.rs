//! Non-gating in-process profile of the negotiated JSON client path. Populated
//! terminal cores use fixed geometry; no wall-clock assertion belongs in CI.
use super::*;
use std::time::Instant;

use crate::client::{ClientShellConfig, ClientShellState};
use crate::protocol::endpoint::{self, EndpointServerWelcome};

fn report(name: &str, samples: &mut [Duration]) {
    samples.sort();
    eprint!(
        " {name}={:.2}/{:.2}us",
        samples[samples.len() / 2].as_secs_f64() * 1e6,
        samples[samples.len() * 95 / 100].as_secs_f64() * 1e6,
    );
}

#[tokio::test]
#[ignore = "non-gating profile; run just bench-tab-slots"]
async fn tab_slots_negotiated_projection_profile() {
    use crate::client::endpoint::{
        decode_endpoint_control, EndpointControlMessage, EndpointNegotiation,
    };
    const SAMPLES: usize = 40;
    const COLS: u16 = 120;
    const ROWS: u16 = 40;
    for (workspace_count, tabs_per_workspace) in [(1, 1), (1, 15), (15, 1)] {
        for client_count in [1, 3] {
            let mut server = test_headless_server();
            server.app.state.workspaces = (0..workspace_count)
                .map(|index| {
                    let mut workspace =
                        crate::workspace::Workspace::test_new(&format!("profile-{index}"));
                    for _ in 1..tabs_per_workspace {
                        workspace.test_add_tab(None);
                    }
                    let panes: Vec<_> = workspace.tabs.iter().map(|tab| tab.root_pane).collect();
                    for pane in panes {
                        workspace.insert_test_runtime(
                            pane,
                            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                                COLS,
                                ROWS,
                                b"populated terminal\r\nprofile data\r\n",
                            ),
                        );
                    }
                    workspace
                })
                .collect();
            server.app.state.ensure_test_terminals();
            server.app.state.active = Some(0);
            server.app.state.mode = crate::app::Mode::Terminal;
            let connections: Vec<_> = (0..client_count)
                .map(|index| connect_test_shell(&mut server, index as u64 + 1, COLS, ROWS))
                .collect();
            for (control, _) in &connections {
                client_shell_snapshot(control);
            }
            // Traverse the production welcome JSON and capability negotiation,
            // rather than silently measuring the legacy positional client path.
            let welcome = EndpointServerWelcome::compatible(
                crate::server::client_commands::supported_client_shell_method_names()
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            );
            let welcome: EndpointServerWelcome =
                serde_json::from_slice(&serde_json::to_vec(&welcome).unwrap()).unwrap();
            let negotiation = EndpointNegotiation::new(welcome.methods, welcome.capabilities);
            assert!(negotiation.supports_capability(endpoint::TAB_SLOTS_CAPABILITY));
            let id = crate::client::endpoint::ClientEndpointId::Local;
            let mut shells: Vec<_> = (0..client_count)
                .map(|_| {
                    let mut shell = ClientShellState::new(ClientShellConfig::from_config(
                        &crate::config::Config::default(),
                    ));
                    shell.set_endpoint_tab_slots_supported(
                        &id,
                        negotiation.supports_capability(endpoint::TAB_SLOTS_CAPABILITY),
                    );
                    shell
                })
                .collect();
            let mut catalog = super::super::tab_slots::TabSlotCatalog::default();
            let started = Instant::now();
            let initial = catalog.capture(&server.app.state);
            let rebuild = started.elapsed();
            let mut timings: [Vec<Duration>; 8] = std::array::from_fn(|_| Vec::new());
            // Iteration zero seeds order/map caches. Measured iterations model
            // output-driven revisions with unchanged topology/slot metadata.
            for revision in 1..=SAMPLES + 1 {
                let pane = server.app.state.workspaces[0].tabs[0].root_pane;
                write_shared_test_pane(&mut server, pane, b"output\r\n");
                let started = Instant::now();
                let slots = catalog.capture(&server.app.state);
                let capture = started.elapsed();
                assert!(Arc::ptr_eq(&initial, &slots));
                if revision > 1 {
                    timings[0].push(capture);
                }
                for (index, shell) in shells.iter_mut().enumerate() {
                    let started = Instant::now();
                    let (snapshot, _) = super::super::client_shell_snapshot(
                        &server.app,
                        "profile",
                        revision as u64,
                        None,
                        server.clients[&(index as u64 + 1)].shell_location.as_ref(),
                    );
                    let projection = started.elapsed();
                    let started = Instant::now();
                    let snapshot_message = endpoint::snapshot_message(&snapshot).unwrap();
                    let snapshot_bytes =
                        HeadlessServer::frame_server_message(&snapshot_message).unwrap();
                    let serialize_snapshot = started.elapsed();
                    let started = Instant::now();
                    let slots_message =
                        endpoint::tab_slots_message(&snapshot.boot_id, snapshot.revision, &slots)
                            .unwrap();
                    let slots_bytes = HeadlessServer::frame_server_message(&slots_message).unwrap();
                    let serialize_slots = started.elapsed();
                    let started = Instant::now();
                    let ServerMessage::EndpointControl { kind, data } =
                        read_server_message(slots_bytes)
                    else {
                        panic!("slots control")
                    };
                    let EndpointControlMessage::TabSlots(metadata) =
                        decode_endpoint_control(&kind, &data).unwrap()
                    else {
                        panic!("negotiated slots")
                    };
                    shell.set_endpoint_tab_slots_for_generation(&id, 1, metadata);
                    let ServerMessage::EndpointControl { kind, data } =
                        read_server_message(snapshot_bytes)
                    else {
                        panic!("snapshot control")
                    };
                    let EndpointControlMessage::Snapshot(snapshot) =
                        decode_endpoint_control(&kind, &data).unwrap()
                    else {
                        panic!("frozen snapshot")
                    };
                    let decode = started.elapsed();
                    let before = shell.tab_slot_profile_times();
                    let prepared = shell
                        .prepare_endpoint_slot_snapshot(&id, 1, snapshot, true)
                        .unwrap();
                    let after = shell.tab_slot_profile_times();
                    let started = Instant::now();
                    shell.install_prepared_endpoint_snapshot(&id, Some(1), prepared, true);
                    shell.apply_cached_endpoint_snapshot(&id);
                    let install = started.elapsed();
                    if revision > 1 {
                        for (samples, elapsed) in timings.iter_mut().skip(1).zip([
                            projection,
                            serialize_snapshot,
                            serialize_slots,
                            decode,
                            after.0 - before.0,
                            after.1 - before.1,
                            install,
                        ]) {
                            samples.push(elapsed);
                        }
                    }
                }
            }
            eprint!("\ntab_slots negotiated=true geometry={COLS}x{ROWS} workspaces={workspace_count} tabs_per_workspace={tabs_per_workspace} populated_panes={} attached_clients={client_count} samples={SAMPLES} median/p95 rebuild={:.2}us", workspace_count * tabs_per_workspace, rebuild.as_secs_f64() * 1e6);
            for (name, samples) in [
                "capture",
                "projection",
                "snapshot_wire",
                "slots_wire",
                "decode",
                "validate",
                "remap",
                "install",
            ]
            .into_iter()
            .zip(&mut timings)
            {
                report(name, samples);
            }
            let mut fanout = Vec::new();
            for sample in 0..SAMPLES {
                server.app.state.workspaces[0]
                    .renumber_tab(0, if sample % 2 == 0 { 20 } else { 1 })
                    .unwrap();
                for client in server.clients.values_mut() {
                    client.request_recompute();
                }
                let started = Instant::now();
                server.render_and_stream();
                fanout.push(started.elapsed());
                for (control, render) in &connections {
                    control.try_iter().for_each(drop);
                    render.try_iter().for_each(drop);
                }
            }
            report("slot_mutation_fanout", &mut fanout);
            eprintln!();
            shutdown_test_runtimes(&mut server);
        }
    }
}
