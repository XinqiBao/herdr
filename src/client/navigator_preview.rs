//! One bounded public API read at a time, independent of interactive endpoint commands.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::endpoint::{ClientEndpointId, EndpointRegistry, SavedSshEndpoint};
use super::shell::{ClientShellEndpointError, ClientShellInput, ClientShellState};
use crate::api::schema::{Method, PaneReadParams, ReadFormat, ReadSource, Request, ResponseResult};

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

struct PendingRead {
    endpoint_id: ClientEndpointId,
    generation: u64,
    boot_id: String,
    request_id: String,
    started: Instant,
    timeout_reported: bool,
    receiver: Receiver<Result<ResponseResult, ClientShellEndpointError>>,
}

#[derive(Default)]
pub(super) struct PreviewReader {
    profiles: Vec<SavedSshEndpoint>,
    local_api_path: Option<std::path::PathBuf>,
    pending: Option<PendingRead>,
    #[cfg(test)]
    direct_test_path: Option<std::path::PathBuf>,
}

impl PreviewReader {
    pub(super) fn new(profiles: Vec<SavedSshEndpoint>, remote_client: bool) -> Self {
        let api_path = crate::api::socket_path();
        let matching_local =
            crate::server::socket_paths::derive_client_socket_from_api_socket(&api_path)
                == crate::server::socket_paths::client_socket_path();
        Self {
            profiles,
            local_api_path: (!remote_client && matching_local).then_some(api_path),
            pending: None,
            #[cfg(test)]
            direct_test_path: None,
        }
    }

    pub(super) fn set_profiles(&mut self, profiles: Vec<SavedSshEndpoint>) {
        self.profiles = profiles;
    }

    pub(super) fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub(super) fn with_test_path(path: std::path::PathBuf) -> Self {
        Self {
            local_api_path: Some(path.clone()),
            direct_test_path: Some(path),
            ..Self::default()
        }
    }

    pub(super) fn start(
        &mut self,
        endpoint_id: ClientEndpointId,
        generation: u64,
        boot_id: String,
        request_id: String,
        pane_id: String,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let profile = match &endpoint_id {
            ClientEndpointId::Local if self.local_api_path.is_some() => None,
            ClientEndpointId::Local => return false,
            ClientEndpointId::Ssh(id) => {
                let Some(profile) = self
                    .profiles
                    .iter()
                    .find(|p| &p.id == id && p.enabled)
                    .cloned()
                else {
                    return false;
                };
                Some(profile)
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let read_id = request_id.clone();
        let local_path = self.local_api_path.clone();
        #[cfg(test)]
        let (profile, local_path) = if let Some(path) = &self.direct_test_path {
            (None, Some(path.clone()))
        } else {
            (profile, local_path)
        };
        let task = std::thread::Builder::new()
            .name("goto-preview".into())
            .spawn(move || {
                let result =
                    read_preview(profile, local_path, &read_id, pane_id).map_err(|error| {
                        ClientShellEndpointError {
                            code: (error.kind() == std::io::ErrorKind::Unsupported)
                                .then(|| "unsupported_method".into()),
                            message: error.to_string(),
                        }
                    });
                let _ = sender.send(result);
            });
        if task.is_err() {
            return false;
        }
        self.pending = Some(PendingRead {
            endpoint_id,
            generation,
            boot_id,
            request_id,
            started: Instant::now(),
            timeout_reported: false,
            receiver,
        });
        true
    }

    pub(super) fn poll(
        &mut self,
        shell: &mut ClientShellState,
        endpoints: &EndpointRegistry,
        now: Instant,
        outcome: &mut ClientShellInput,
    ) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err(ClientShellEndpointError {
                code: None,
                message: "Preview worker stopped".into(),
            }),
            Err(mpsc::TryRecvError::Empty) => {
                if !pending.timeout_reported
                    && now.saturating_duration_since(pending.started) >= READ_TIMEOUT
                {
                    outcome.repaint |= shell.reject_navigator_preview(&pending.request_id);
                    pending.timeout_reported = true;
                }
                return;
            }
        };
        let Some(pending) = self.pending.take() else {
            return;
        };
        if endpoints.accepts(&pending.endpoint_id, pending.generation)
            && shell.accepts_navigator_preview(
                &pending.endpoint_id,
                pending.generation,
                &pending.boot_id,
                &pending.request_id,
            )
        {
            outcome.repaint |= shell.complete_navigator_preview(&pending.request_id, result);
        } else {
            outcome.repaint |= shell.reject_navigator_preview(&pending.request_id);
        }
    }
}

fn read_preview(
    profile: Option<SavedSshEndpoint>,
    local_path: Option<std::path::PathBuf>,
    request_id: &str,
    pane_id: String,
) -> std::io::Result<ResponseResult> {
    // Setup discovery already has a 15s noninteractive SSH deadline. Keep the real busy slot
    // until the bridge is dropped, including after the UI's shorter timeout or cancellation.
    let bridge = profile
        .as_ref()
        .map(|profile| {
            crate::remote::SavedSshApiBridge::start(
                profile.id.as_str(),
                &profile.target,
                &profile.session,
                true,
            )
        })
        .transpose()?;
    let socket_path = bridge
        .as_ref()
        .map(|bridge| bridge.socket_path().to_owned())
        .or(local_path)
        .ok_or_else(|| std::io::Error::other("Preview API target unavailable"))?;
    read_preview_request(
        &socket_path,
        request_id,
        pane_id,
        bridge
            .as_ref()
            .map(|bridge| bridge as &dyn PreviewApiBridge),
    )
}

trait PreviewApiBridge {
    fn used_cached_metadata(&self) -> bool;
    fn reported_failure(&self) -> Option<std::io::Error>;
    fn invalidate_metadata(&self);
}

impl PreviewApiBridge for crate::remote::SavedSshApiBridge {
    fn used_cached_metadata(&self) -> bool {
        self.used_cached_metadata
    }

    fn reported_failure(&self) -> Option<std::io::Error> {
        self.reported_failure()
    }

    fn invalidate_metadata(&self) {
        self.invalidate_metadata();
    }
}

fn read_preview_request(
    socket_path: &std::path::Path,
    request_id: &str,
    pane_id: String,
    bridge: Option<&dyn PreviewApiBridge>,
) -> std::io::Result<ResponseResult> {
    let request = Request {
        id: request_id.to_owned(),
        method: Method::PaneRead(PaneReadParams {
            pane_id,
            source: ReadSource::Visible,
            lines: Some(200),
            format: ReadFormat::Text,
            strip_ansi: true,
            intent: Default::default(),
        }),
    };
    let result = (|| {
        let mut stream = crate::ipc::connect_local_stream(socket_path)?;
        let mut encoded = serde_json::to_vec(&request).map_err(std::io::Error::other)?;
        encoded.push(b'\n');
        // A small fixed read request cannot fill the socket; polling then bounds partial replies.
        stream.write_all(&encoded)?;
        crate::ipc::set_local_stream_polling(&mut stream, true)?;
        decode_preview_response(
            PreviewDeadlineReader {
                stream,
                deadline: Instant::now() + READ_TIMEOUT,
            },
            request_id,
        )
    })();
    if result.is_err() {
        if let Some(bridge) = bridge.filter(|bridge| bridge.used_cached_metadata()) {
            if bridge
                .reported_failure()
                .as_ref()
                .is_some_and(crate::remote::SavedSshApiBridge::stale_metadata_failure)
            {
                // Match CLI recovery: only the explicit stale-executable marker retires cache.
                // A later selection rediscovers; do not retry or release the real worker slot.
                bridge.invalidate_metadata();
            }
        }
    }
    result
}

fn decode_preview_response(reader: impl Read, request_id: &str) -> std::io::Result<ResponseResult> {
    let mut response = Vec::new();
    let mut reader = BufReader::new(reader.take(MAX_RESPONSE_BYTES + 1));
    reader.read_until(b'\n', &mut response)?;
    if response.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(std::io::Error::other("Preview response too large"));
    }
    super::endpoint_commands::parse_response(request_id, &response).map_err(|error| {
        let kind = if matches!(
            error.code.as_deref(),
            Some("unknown_method" | "method_not_found" | "unsupported_method")
        ) {
            std::io::ErrorKind::Unsupported
        } else {
            std::io::ErrorKind::Other
        };
        std::io::Error::new(kind, error.message)
    })
}

struct PreviewDeadlineReader {
    stream: crate::ipc::LocalStream,
    deadline: Instant,
}

impl Read for PreviewDeadlineReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Preview timed out",
                ));
            }
            match crate::ipc::poll_local_stream_read_count(&mut self.stream, bytes)? {
                crate::ipc::LocalStreamReadCount::Data(count) => return Ok(count),
                crate::ipc::LocalStreamReadCount::Closed => return Ok(0),
                crate::ipc::LocalStreamReadCount::Pending => {
                    std::thread::sleep(Duration::from_millis(2))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interprocess::local_socket::traits::Listener as _;

    #[test]
    fn preview_failure_invalidates_only_explicitly_stale_cached_ssh_metadata() {
        use std::cell::Cell;

        struct Bridge {
            cached: bool,
            failure: Option<&'static str>,
            reports: Cell<usize>,
            invalidated: Cell<bool>,
        }
        impl PreviewApiBridge for Bridge {
            fn used_cached_metadata(&self) -> bool {
                self.cached
            }
            fn reported_failure(&self) -> Option<std::io::Error> {
                self.reports.set(self.reports.get() + 1);
                self.failure.map(std::io::Error::other)
            }
            fn invalidate_metadata(&self) {
                self.invalidated.set(true);
            }
        }

        for (index, (success, cached, failure, invalidate, reports)) in [
            (
                false,
                true,
                Some("herdr-machine-metadata-stale-v1"),
                true,
                1,
            ),
            (false, true, Some("Permission denied"), false, 1),
            (false, true, None, false, 1),
            (
                false,
                false,
                Some("herdr-machine-metadata-stale-v1"),
                false,
                0,
            ),
            (
                true,
                true,
                Some("herdr-machine-metadata-stale-v1"),
                false,
                0,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let path = std::env::temp_dir().join(format!(
                "herdr-goto-cache-{}-{index}.sock",
                std::process::id()
            ));
            let listener = crate::ipc::bind_private_local_listener(&path).unwrap();
            let server = std::thread::spawn(move || {
                let mut stream = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let request: Request = serde_json::from_str(&line).unwrap();
                assert!(matches!(request.method, Method::PaneRead(_)));
                if success {
                    stream
                        .write_all(b"{\"id\":\"cache-test\",\"result\":{\"type\":\"ok\"}}\n")
                        .unwrap();
                }
            });
            let bridge = Bridge {
                cached,
                failure,
                reports: Cell::new(0),
                invalidated: Cell::new(false),
            };
            let result = read_preview_request(&path, "cache-test", "pane_1".into(), Some(&bridge));
            assert_eq!(result.is_ok(), success);
            assert_eq!(bridge.invalidated.get(), invalidate);
            assert_eq!(bridge.reports.get(), reports);
            server.join().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn preview_timeout_retains_the_real_busy_slot_until_worker_cleanup() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut reader = PreviewReader {
            pending: Some(PendingRead {
                endpoint_id: ClientEndpointId::Local,
                generation: 1,
                boot_id: "boot".into(),
                request_id: "old".into(),
                started: Instant::now() - READ_TIMEOUT,
                timeout_reported: false,
                receiver,
            }),
            ..PreviewReader::default()
        };
        let mut shell = ClientShellState::new(super::super::shell::ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        let endpoints = EndpointRegistry::empty();
        let mut outcome = ClientShellInput::default();
        reader.poll(&mut shell, &endpoints, Instant::now(), &mut outcome);
        assert!(reader.is_busy());
        assert!(!reader.start(
            ClientEndpointId::Local,
            1,
            "boot".into(),
            "new".into(),
            "w1:p1".into()
        ));
        assert!(sender.send(Ok(ResponseResult::Ok {})).is_ok());
        reader.poll(&mut shell, &endpoints, Instant::now(), &mut outcome);
        assert!(!reader.is_busy());
    }

    #[test]
    fn public_preview_response_is_bounded_before_json_decoding() {
        let response = std::io::Cursor::new(vec![b'x'; MAX_RESPONSE_BYTES as usize + 2]);
        let error = decode_preview_response(response, "bounded").unwrap_err();
        assert!(error.to_string().contains("too large"), "{error}");
    }

    #[test]
    fn partial_preview_response_obeys_one_deadline() {
        let path =
            std::env::temp_dir().join(format!("herdr-goto-partial-{}.sock", std::process::id()));
        let listener = crate::ipc::bind_private_local_listener(&path).unwrap();
        let (release, released) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            stream.write_all(b"{").unwrap();
            released.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        let mut stream = crate::ipc::connect_local_stream(&path).unwrap();
        crate::ipc::set_local_stream_polling(&mut stream, true).unwrap();
        let mut reader = BufReader::new(PreviewDeadlineReader {
            stream,
            deadline: Instant::now() + Duration::from_millis(20),
        });
        let error = reader.read_until(b'\n', &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        release.send(()).unwrap();
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
