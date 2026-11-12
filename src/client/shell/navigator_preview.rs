//! Read-only, debounced terminal text for the Goto overlay.

use std::time::{Duration, Instant};

use super::*;
use crate::api::schema::{ReadFormat, ReadSource, ResponseResult};

const PREVIEW_DELAY: Duration = Duration::from_millis(100);
const MAX_PREVIEW_LINES: usize = 200;
const MAX_PREVIEW_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreviewTarget {
    pub(super) endpoint_id: ClientEndpointId,
    pub(super) generation: Option<u64>,
    pub(super) boot_id: String,
    pub(super) pane_id: String,
    status: ClientEndpointStatus,
    unavailable: Option<&'static str>,
}

#[derive(Debug, Default)]
pub(super) struct NavigatorPreview {
    initialized: bool,
    target: Option<PreviewTarget>,
    selection: Option<ClientNavigatorTarget>,
    due: Option<Instant>,
    #[cfg(test)]
    pub(super) row_resolutions: usize,
    pub(super) lines: Vec<String>,
    pub(super) message: Option<&'static str>,
}

pub(super) struct PendingNavigatorPreview {
    target: PreviewTarget,
    request_id: String,
}

/// Compare only inputs to navigator membership/order, without formatting or allocating rows.
/// Called on metadata delivery while Goto is open; output revisions and tokens are irrelevant.
pub(super) fn selection_metadata_changed(
    previous: Option<&ClientShellSnapshot>,
    next: &ClientShellSnapshot,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    previous.boot_id != next.boot_id
        || previous.workspaces.len() != next.workspaces.len()
        || previous
            .workspaces
            .iter()
            .zip(&next.workspaces)
            .any(|(a, b)| {
                a.workspace_id != b.workspace_id || a.label != b.label || a.branch != b.branch
            })
        || previous.tabs.len() != next.tabs.len()
        || previous.tabs.iter().zip(&next.tabs).any(|(a, b)| {
            a.tab_id != b.tab_id
                || a.workspace_id != b.workspace_id
                || a.label != b.label
                || a.custom_label != b.custom_label
        })
        || previous.panes.len() != next.panes.len()
        || previous.panes.iter().zip(&next.panes).any(|(a, b)| {
            a.pane_id != b.pane_id
                || a.tab_id != b.tab_id
                || a.label != b.label
                || a.cwd != b.cwd
                || a.foreground_cwd != b.foreground_cwd
        })
        || previous.agents.len() != next.agents.len()
        || previous.agents.iter().zip(&next.agents).any(|(a, b)| {
            a.pane_id != b.pane_id
                || a.name != b.name
                || a.agent != b.agent
                || a.display_agent != b.display_agent
                || a.title != b.title
                || a.terminal_title_stripped != b.terminal_title_stripped
                || a.agent_status != b.agent_status
        })
}

impl ClientShellState {
    fn preview_target(&self, selected: Option<&ClientNavigatorTarget>) -> Option<PreviewTarget> {
        let ClientNavigatorTarget::Pane {
            endpoint_id,
            pane_id,
        } = selected?
        else {
            return None;
        };
        let endpoint = self
            .endpoints
            .iter()
            .find(|e| &e.endpoint_id == endpoint_id)?;
        let snapshot = endpoint.snapshot.as_deref();
        let unavailable = if endpoint.status != ClientEndpointStatus::Online {
            Some("Preview unavailable: server offline")
        } else if snapshot.is_none_or(|s| !s.panes.iter().any(|p| &p.pane_id == pane_id)) {
            Some("Preview unavailable: terminal no longer exists")
        } else {
            None
        };
        Some(PreviewTarget {
            endpoint_id: endpoint_id.clone(),
            generation: endpoint.snapshot_generation,
            boot_id: snapshot.map(|s| s.boot_id.clone()).unwrap_or_default(),
            pane_id: pane_id.clone(),
            status: endpoint.status,
            unavailable,
        })
    }

    // Resolve fallback selection on input or relevant metadata, never during rendering.
    pub(super) fn refresh_navigator_preview(
        &mut self,
        now: Instant,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::Navigator(n)) = self.overlay.as_mut() else {
            self.pending_navigator_preview = None;
            return;
        };
        #[cfg(test)]
        {
            n.preview.row_resolutions += 1;
        }
        let rows = render::client_navigator_rows(&self.endpoints, &self.active_endpoint_id, n);
        let selected = aggregate_navigation::selected_navigator_target(&rows, n);
        self.update_navigator_preview_target(selected, now, outcome);
    }

    fn update_navigator_preview_target(
        &mut self,
        selection: Option<ClientNavigatorTarget>,
        now: Instant,
        outcome: &mut ClientShellInput,
    ) {
        let target = self.preview_target(selection.as_ref());
        let Some(ClientShellOverlay::Navigator(n)) = self.overlay.as_mut() else {
            return;
        };
        n.preview.selection = selection.clone();
        if n.preview.initialized && n.preview.target == target {
            return;
        }
        // Selection epochs are distinct even for A -> B -> A. The runtime keeps its busy slot.
        self.pending_navigator_preview = None;
        n.preview = NavigatorPreview {
            initialized: true,
            selection,
            #[cfg(test)]
            row_resolutions: n.preview.row_resolutions,
            message: target
                .as_ref()
                .and_then(|target| target.unavailable)
                .or(Some(if target.is_some() {
                    "Loading preview…"
                } else {
                    "Select a terminal to preview"
                })),
            due: target
                .as_ref()
                .filter(|target| target.unavailable.is_none())
                .map(|_| now + PREVIEW_DELAY),
            target,
            lines: Vec::new(),
        };
        outcome.repaint = true;
    }

    pub(crate) fn tick_navigator_preview(
        &mut self,
        now: Instant,
        reader_busy: bool,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::Navigator(n)) = self.overlay.as_ref() else {
            self.pending_navigator_preview = None;
            return;
        };
        // Snapshot and status changes can invalidate a preview without keyboard input.
        let invalidated = n.preview.target.as_ref().is_some_and(|target| {
            self.endpoints
                .iter()
                .find(|e| e.endpoint_id == target.endpoint_id)
                .is_none_or(|e| {
                    e.snapshot_generation != target.generation
                        || e.snapshot
                            .as_deref()
                            .is_none_or(|s| s.boot_id != target.boot_id)
                        || e.status != target.status
                })
        });
        if !n.preview.initialized {
            self.refresh_navigator_preview(now, outcome);
        } else if invalidated {
            // Membership changes reconcile at metadata delivery, before a reader can complete.
            // Timers only update the selected endpoint's availability/connection identity.
            let selection = n.preview.selection.clone();
            self.update_navigator_preview_target(selection, now, outcome);
        }
        if self.pending_navigator_preview.is_some() || reader_busy {
            return;
        }
        let Some(ClientShellOverlay::Navigator(n)) = self.overlay.as_mut() else {
            return;
        };
        if n.preview.due.is_none_or(|due| now < due) {
            return;
        }
        n.preview.due = None;
        let Some(target) = n.preview.target.clone() else {
            return;
        };
        let request_id = format!("client-shell-preview:{}", self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        outcome.actions.push(ClientShellAction::NavigatorPreview {
            endpoint_id: target.endpoint_id.clone(),
            generation: target.generation,
            boot_id: target.boot_id.clone(),
            pane_id: target.pane_id.clone(),
            request_id: request_id.clone(),
        });
        self.pending_navigator_preview = Some(PendingNavigatorPreview { target, request_id });
    }

    pub(crate) fn accepts_navigator_preview(
        &self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        boot_id: &str,
        request_id: &str,
    ) -> bool {
        self.pending_navigator_preview
            .as_ref()
            .is_some_and(|pending| {
                pending.target.endpoint_id == *endpoint_id
                    && pending
                        .target
                        .generation
                        .is_none_or(|expected| expected == generation)
                    && pending.target.boot_id == boot_id
                    && pending.request_id == request_id
            })
    }

    pub(crate) fn reject_navigator_preview(&mut self, request_id: &str) -> bool {
        self.complete_navigator_preview(
            request_id,
            Err(ClientShellEndpointError {
                code: None,
                message: "Preview unavailable".into(),
            }),
        )
    }

    pub(crate) fn complete_navigator_preview(
        &mut self,
        request_id: &str,
        result: Result<ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        if self
            .pending_navigator_preview
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return false;
        }
        let Some(pending) = self.pending_navigator_preview.take() else {
            return false;
        };
        let current = self.overlay.as_ref().and_then(|overlay| match overlay {
            ClientShellOverlay::Navigator(n) => self.preview_target(n.preview.selection.as_ref()),
            _ => None,
        });
        if current.as_ref() != Some(&pending.target) || pending.target.unavailable.is_some() {
            return false;
        }
        let Some(ClientShellOverlay::Navigator(n)) = self.overlay.as_mut() else {
            return false;
        };
        match result {
            Ok(ResponseResult::PaneRead { read })
                if read.pane_id == pending.target.pane_id
                    && read.source == ReadSource::Visible
                    && read.format == ReadFormat::Text =>
            {
                n.preview.lines = read
                    .text
                    .lines()
                    .take(MAX_PREVIEW_LINES)
                    .scan(0usize, |bytes, line| {
                        if *bytes >= MAX_PREVIEW_BYTES {
                            return None;
                        }
                        let sanitized: String = line
                            .chars()
                            .filter(|c| !c.is_control())
                            .take_while(|c| {
                                if *bytes + c.len_utf8() > MAX_PREVIEW_BYTES {
                                    return false;
                                }
                                *bytes += c.len_utf8();
                                true
                            })
                            .collect();
                        Some(sanitized)
                    })
                    .collect();
                n.preview.message = n.preview.lines.is_empty().then_some("Terminal is empty");
            }
            Err(error) if error.code.as_deref() == Some("unsupported_method") => {
                n.preview.message = Some("Preview unavailable: pane.read unsupported");
            }
            _ => n.preview.message = Some("Preview unavailable: terminal read failed"),
        }
        true
    }
}
