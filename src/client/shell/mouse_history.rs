//! A bounded input reservation while the runtime resolves one wheel operation.
//! Queued events re-enter the existing input/lease engine; they are never guessed
//! to be terminal input after an uncertain result.
use super::*;
use crate::api::schema::{
    Method, PaneWheelDirection, PaneWheelParams, PaneWheelPixels, PaneWheelRoute,
};
use crate::raw_input::RawInputEvent;
use crossterm::event::{KeyEventKind, MouseEvent, MouseEventKind};

const MAX_EVENTS: usize = 256;
const MAX_TEXT_BYTES: usize = 64 * 1024;

pub(super) struct WheelCapture {
    generation: u64,
    endpoint_id: ClientEndpointId,
    boot_id: String,
    hit: PaneHit,
    queue: VecDeque<(RawInputEvent, Option<crate::input::mouse::HostPixels>)>,
    text_bytes: usize,
    direction: PaneWheelDirection,
    original_focus: Option<String>,
    pub(super) failed: bool,
}

impl ClientShellState {
    pub(crate) fn captured_input_is_current(
        &self,
        endpoint_id: &ClientEndpointId,
        boot_id: &str,
        message: &ClientMessage,
    ) -> bool {
        &self.active_endpoint_id == endpoint_id
            && self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot.boot_id == boot_id
                    && match message {
                        ClientMessage::ClientShellPaneInput { pane_id, .. } => {
                            snapshot.panes.iter().any(|pane| &pane.pane_id == pane_id)
                        }
                        _ => false,
                    }
            })
    }

    pub(super) fn reconcile_history_snapshot(&mut self, snapshot: &ClientShellSnapshot) {
        let invalid = self.wheel_capture.as_ref().is_some_and(|capture| {
            snapshot.boot_id != capture.boot_id
                || !snapshot
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == capture.hit.pane_id)
                || (snapshot.focused_pane_id != capture.original_focus
                    && snapshot.focused_pane_id.as_deref() != Some(capture.hit.pane_id.as_str()))
        });
        if invalid {
            self.cancel_history_capture();
        }
        if snapshot.focused_pane_id.as_deref() == self.history_focus_target.as_deref() {
            self.history_focus_target = None;
        }
    }
    pub(super) fn cancel_history_capture(&mut self) {
        self.wheel_capture = None;
        self.wheel_generation = self.wheel_generation.saturating_add(1);
        self.history_focus_target = None;
    }

    fn fail_history_capture(&mut self, outcome: &mut ClientShellInput) {
        if let Some(capture) = self.wheel_capture.as_mut() {
            capture.failed = true;
            capture.queue.clear();
            capture.text_bytes = 0;
        }
        outcome.repaint |= self.push_endpoint_notice(ClientEndpointNoticeKind::Rejected,
            "wheel_capture", "History input paused",
            "The scroll result is unknown. Input was held for safety. Press Esc or q to resume, or use prefix navigation.");
    }

    /// Returns the event only if normal input processing should continue.
    pub(super) fn reserve_history_input(
        &mut self,
        event: RawInputEvent,
        outcome: &mut ClientShellInput,
    ) -> Option<RawInputEvent> {
        if self.wheel_capture.is_none() && self.history_focus_target.is_some() {
            let navigation = match &event {
                RawInputEvent::Key(key) => {
                    key.kind != KeyEventKind::Release
                        && (self.config.keybinds.matches_prefix(key)
                            || crate::input::resolve_direct_binding(
                                &self.config.keybinds.keybinds,
                                key,
                            )
                            .is_some())
                }
                RawInputEvent::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                    !self.hits.panes.iter().any(|hit| {
                        Some(&hit.pane_id) == self.history_focus_target.as_ref()
                            && super::contains(hit.rect, (mouse.column, mouse.row))
                    })
                }
                _ => false,
            };
            if navigation {
                self.history_focus_target = None;
            }
        }
        let Some(capture) = self.wheel_capture.as_ref() else {
            return Some(event);
        };
        if self.active_endpoint_id != capture.endpoint_id
            || self.snapshot.as_deref().is_none_or(|s| {
                s.boot_id != capture.boot_id
                    || !s.panes.iter().any(|p| p.pane_id == capture.hit.pane_id)
            })
        {
            self.cancel_history_capture();
            return Some(event);
        }
        if let RawInputEvent::Key(key) = &event {
            if key.kind != KeyEventKind::Release
                && (self.config.keybinds.matches_prefix(key)
                    || crate::input::resolve_direct_binding(&self.config.keybinds.keybinds, key)
                        .is_some())
            {
                self.cancel_history_capture();
                return Some(event);
            }
            if capture.failed
                && key.kind != KeyEventKind::Release
                && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
            {
                self.cancel_history_capture();
                outcome.repaint = true;
                return None;
            }
        }
        match &event {
            RawInputEvent::Mouse(mouse) => {
                // Only a new click/wheel is deliberate navigation. Pointer
                // motion and a held gesture's release must not expose keys.
                if matches!(
                    mouse.kind,
                    MouseEventKind::Down(_)
                        | MouseEventKind::ScrollUp
                        | MouseEventKind::ScrollDown
                        | MouseEventKind::ScrollLeft
                        | MouseEventKind::ScrollRight
                ) && !super::contains(capture.hit.rect, (mouse.column, mouse.row))
                {
                    self.cancel_history_capture();
                    return Some(event);
                }
            }
            RawInputEvent::OuterFocusLost => {
                self.fail_history_capture(outcome);
                return Some(event);
            }
            RawInputEvent::Key(_) | RawInputEvent::Text(_) | RawInputEvent::Paste(_) => {}
            _ => return Some(event),
        }
        let bytes = match &event {
            RawInputEvent::Paste(text) => text.len(),
            RawInputEvent::Text(text) => text.as_str().len(),
            RawInputEvent::Key(key) => {
                key.generated_text.as_ref().map_or(0, String::len)
                    + key.vt_bytes().map_or(0, <[u8]>::len)
            }
            _ => 0,
        };
        let capture = self.wheel_capture.as_mut()?;
        if capture.failed {
            return None;
        }
        if capture.queue.len() >= MAX_EVENTS
            || capture.text_bytes.saturating_add(bytes) > MAX_TEXT_BYTES
        {
            self.fail_history_capture(outcome);
            return None;
        }
        capture.text_bytes += bytes;
        capture.queue.push_back((event, self.host_mouse_pixels));
        None
    }

    fn history_wheel_params(&self, hit: &PaneHit, mouse: MouseEvent) -> PaneWheelParams {
        let (column, row, pixels) = match self.pane_mouse_position(hit, mouse) {
            ClientMousePosition::Cell { column, row } => (column, row, None),
            ClientMousePosition::Pixels { x, y, column, row } => (
                column,
                row,
                Some(PaneWheelPixels {
                    x,
                    y,
                    cols: hit.inner_rect.width,
                    rows: hit.inner_rect.height,
                    width_px: hit.pixel_width,
                    height_px: hit.pixel_height,
                }),
            ),
        };
        PaneWheelParams {
            pane_id: hit.pane_id.clone(),
            direction: if mouse.kind == MouseEventKind::ScrollUp {
                PaneWheelDirection::Up
            } else {
                PaneWheelDirection::Down
            },
            lines: self.config.mouse_scroll_lines.min(u16::MAX as usize) as u16,
            column,
            row,
            pixels,
            modifiers: mouse.modifiers.bits(),
        }
    }

    pub(super) fn begin_history_wheel(
        &mut self,
        hit: &PaneHit,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if hit.popup
            || !matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            return false;
        }
        if self
            .copy_mode
            .as_ref()
            .is_some_and(|copy| copy.pane_id == hit.pane_id)
        {
            if self.focused_pane_id().as_deref() != Some(hit.pane_id.as_str()) {
                self.push_endpoint_method(
                    Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id: hit.pane_id.clone(),
                    }),
                    outcome,
                );
                self.history_focus_target = Some(hit.pane_id.clone());
            }
            self.mode = ClientShellMode::Copy;
            self.scroll_history_copy(mouse, outcome);
            return true;
        }
        let advertised = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .and_then(|endpoint| endpoint.methods.as_ref())
            .is_some_and(|methods| methods.contains("pane.wheel"));
        if !advertised {
            outcome.repaint |= self.push_endpoint_notice(ClientEndpointNoticeKind::Unsupported, "pane.wheel",
                "Automatic history capture unavailable", "This server keeps legacy wheel input. Enter copy mode explicitly before browsing history.");
            return false;
        }
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let boot_id = snapshot.boot_id.clone();
        let params = self.history_wheel_params(hit, mouse);
        // Close old forwarded leases once, at their recorded targets. Later held
        // repeats are processed in the new capture context instead of bypassing it.
        if self.replay_input_target.is_none() {
            self.release_input_leases(outcome);
        }
        // The ordinary input sink runs after endpoint actions. Preserve keys
        // preceding this wheel (including old lease releases) before its API
        // request, rather than leaving them in that later sink.
        for message in std::mem::take(&mut outcome.requests) {
            if matches!(message, ClientMessage::ClientShellPaneInput { .. }) {
                outcome.actions.push(ClientShellAction::CapturedInput {
                    endpoint_id: self.active_endpoint_id.clone(),
                    boot_id: boot_id.clone(),
                    message,
                });
            } else {
                outcome.requests.push(message);
            }
        }
        self.wheel_generation = self.wheel_generation.saturating_add(1);
        let generation = self.wheel_generation;
        self.wheel_capture = Some(WheelCapture {
            generation,
            endpoint_id: self.active_endpoint_id.clone(),
            boot_id,
            hit: hit.clone(),
            queue: VecDeque::new(),
            text_bytes: 0,
            direction: params.direction,
            original_focus: self.focused_pane_id(),
            failed: false,
        });
        if self.focused_pane_id().as_deref() != Some(hit.pane_id.as_str()) {
            self.history_focus_target = Some(hit.pane_id.clone());
            self.push_endpoint_method(
                Method::PaneFocus(crate::api::schema::PaneTarget {
                    pane_id: hit.pane_id.clone(),
                }),
                outcome,
            );
        }
        if !self.push_endpoint_method_with_kind(
            Method::PaneWheel(params),
            PendingEndpointKind::PaneWheel { generation },
            outcome,
        ) {
            self.fail_history_capture(outcome);
        }
        true
    }

    pub(super) fn complete_history_wheel(
        &mut self,
        generation: u64,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) {
        let Some(capture) = self.wheel_capture.as_ref() else {
            return;
        };
        if capture.generation != generation || capture.failed {
            return;
        }
        if self.active_endpoint_id != capture.endpoint_id
            || self.snapshot.as_deref().is_none_or(|snapshot| {
                snapshot.boot_id != capture.boot_id
                    || !snapshot
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == capture.hit.pane_id)
            })
        {
            self.cancel_history_capture();
            return;
        }
        let wheel = match result {
            Ok(crate::api::schema::ResponseResult::PaneWheel { wheel })
                if wheel.pane_id == capture.hit.pane_id
                    && wheel.cols > 0
                    && wheel.rows > 0
                    && wheel.after.offset_from_bottom <= wheel.after.max_offset_from_bottom
                    && wheel.route != PaneWheelRoute::Unknown =>
            {
                wheel
            }
            _ => {
                self.fail_history_capture(outcome);
                return;
            }
        };
        let mut capture = self.wheel_capture.take().expect("checked reservation");
        capture.hit.mouse_reporting = wheel.route == PaneWheelRoute::MouseReport;
        capture.hit.inner_rect.width = wheel.cols;
        capture.hit.inner_rect.height = wheel.rows;
        capture.hit.scroll = Some(crate::pane::ScrollMetrics {
            offset_from_bottom: wheel.after.offset_from_bottom as usize,
            max_offset_from_bottom: wheel.after.max_offset_from_bottom as usize,
            viewport_rows: usize::from(wheel.rows),
        });
        let enter = wheel.route == PaneWheelRoute::HostScroll
            && (capture.direction == PaneWheelDirection::Up || wheel.before.offset_from_bottom > 0);
        if enter {
            self.release_input_leases(outcome);
            let mut hit = capture.hit.clone();
            hit.inner_rect.width = wheel.cols;
            hit.inner_rect.height = wheel.rows;
            hit.scroll = Some(crate::pane::ScrollMetrics {
                offset_from_bottom: wheel.before.offset_from_bottom as usize,
                max_offset_from_bottom: wheel.after.max_offset_from_bottom as usize,
                viewport_rows: usize::from(wheel.rows),
            });
            self.enter_copy_mode_from_hit(hit, outcome);
            if self.focused_pane_id().as_deref() != Some(wheel.pane_id.as_str()) {
                self.history_focus_target = Some(wheel.pane_id.clone());
            }
            if let Some(copy) = self.copy_mode.as_mut() {
                copy.content_revision = wheel.content_revision;
                copy.alternate_screen_active = wheel.alternate_screen_active;
                copy.offset_from_bottom = wheel.after.offset_from_bottom as usize;
                copy.cursor.row = wheel
                    .after
                    .max_offset_from_bottom
                    .saturating_sub(wheel.after.offset_from_bottom)
                    .saturating_add(u64::from(wheel.rows.saturating_sub(1)))
                    .min(u64::from(u32::MAX)) as u32;
            }
            self.pane_scroll_targets.insert(
                wheel.pane_id.clone(),
                wheel.after.offset_from_bottom as usize,
            );
        }
        // Re-enter normal input processing with the reserved identity, not a
        // possibly older focus projection. A later wheel remains a distinct request.
        let saved_pixels = self.host_mouse_pixels;
        while let Some((event, pixels)) = capture.queue.pop_front() {
            self.host_mouse_pixels = pixels;
            self.replay_input_target = Some(capture.hit.pane_id.clone());
            let mut next = match event {
                RawInputEvent::Mouse(mouse) => {
                    let mut next = ClientShellInput::default();
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) {
                        self.begin_history_wheel(&capture.hit, mouse, &mut next);
                    } else {
                        // Reuse selection/gesture/focus handling with the original
                        // hit target, even across a temporary projection gap.
                        let previous_hits =
                            std::mem::replace(&mut self.hits.panes, vec![capture.hit.clone()]);
                        self.handle_mouse(mouse, &mut next);
                        self.hits.panes = previous_hits;
                    }
                    next
                }
                event => self.handle_raw_events(vec![event]),
            };
            self.replay_input_target = None;
            self.host_mouse_pixels = saved_pixels;
            for message in next.requests.drain(..) {
                outcome.actions.push(ClientShellAction::CapturedInput {
                    endpoint_id: capture.endpoint_id.clone(),
                    boot_id: capture.boot_id.clone(),
                    message,
                });
            }
            outcome.actions.append(&mut next.actions);
            outcome.repaint |= next.repaint;
            if let Some(pending) = self.wheel_capture.as_mut() {
                pending.queue.append(&mut capture.queue);
                pending.text_bytes = capture.text_bytes;
                break;
            }
            if enter && self.copy_mode.is_none() {
                break;
            }
        }
        self.host_mouse_pixels = saved_pixels;
        for message in outcome.requests.drain(..) {
            outcome.actions.push(ClientShellAction::CapturedInput {
                endpoint_id: capture.endpoint_id.clone(),
                boot_id: capture.boot_id.clone(),
                message,
            });
        }
        outcome.repaint = true;
    }

    pub(super) fn scroll_history_copy(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let Some(copy) = self.copy_mode.as_ref() else {
            return;
        };
        let lines = self.config.mouse_scroll_lines.max(1);
        let offset = if mouse.kind == MouseEventKind::ScrollUp {
            copy.offset_from_bottom
                .saturating_add(lines)
                .min(copy.max_offset_from_bottom)
        } else {
            copy.offset_from_bottom.saturating_sub(lines)
        };
        self.set_history_copy_offset(offset, outcome);
    }

    pub(super) fn set_history_copy_offset(
        &mut self,
        offset: usize,
        outcome: &mut ClientShellInput,
    ) {
        let Some(copy) = self.copy_mode.as_mut() else {
            return;
        };
        copy.offset_from_bottom = offset.min(copy.max_offset_from_bottom);
        let top = copy
            .max_offset_from_bottom
            .saturating_sub(copy.offset_from_bottom)
            .min(u32::MAX as usize) as u32;
        copy.cursor.row = copy.cursor.row.clamp(
            top,
            top.saturating_add(u32::from(copy.geometry.1.saturating_sub(1))),
        );
        let pane_id = copy.pane_id.clone();
        self.push_pane_scroll_offset(pane_id, offset, outcome);
        self.sync_copy_selection();
        outcome.repaint = true;
    }
}
