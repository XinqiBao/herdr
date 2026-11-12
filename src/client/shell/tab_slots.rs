//! Pair optional metadata with its exact frozen projection before presentation.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::protocol::endpoint::{EndpointTabSlot, EndpointTabSlots};
use crate::protocol::ClientShellSnapshot;

/// An exact companion/snapshot pair, validated before endpoint activation.
/// Ownership carries that result into installation without validating it again.
pub(crate) struct PreparedTabSnapshot(pub(crate) Box<ClientShellSnapshot>);
pub(super) type TabSlotMap = BTreeMap<String, EndpointTabSlot>;

#[derive(Clone, Debug, Default)]
pub(super) struct TabSlotProjection {
    pub(super) supported: bool,
    generation: Option<u64>,
    boot_id: Option<String>,
    retired_boots: HashSet<String>,
    newest_revision: u64,
    accepted_revision: Option<u64>,
    pending_slots: Option<EndpointTabSlots>,
    pending_snapshot: Option<(Box<ClientShellSnapshot>, bool)>,
    pub(super) slots: Arc<TabSlotMap>,
    structural_ids: Vec<String>,
    workspace_ids: Vec<String>,
    permutation: Vec<usize>,
    #[cfg(test)]
    operations: ProjectionOperations,
    #[cfg(test)]
    profile_times: (std::time::Duration, std::time::Duration),
}

#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProjectionOperations {
    validations: usize,
    workspace_entries: usize,
    tab_memberships: usize,
    order_rebuilds: usize,
    remapped_tabs: usize,
}

impl TabSlotProjection {
    #[cfg(test)]
    pub(super) fn new(supported: bool) -> Self {
        Self {
            supported,
            ..Self::default()
        }
    }

    pub(super) fn set_supported(&mut self, supported: bool) {
        let displayed_slots = if supported {
            std::mem::take(&mut self.slots)
        } else {
            Arc::default()
        };
        *self = Self {
            supported,
            slots: displayed_slots,
            ..Self::default()
        };
    }
    pub(super) fn ready(&self) -> bool {
        self.supported
            && self.accepted_revision == Some(self.newest_revision)
            && self.pending_snapshot.is_none()
            && self.pending_slots.as_ref().is_none_or(|slots| {
                self.boot_id.as_deref() == Some(&slots.boot_id)
                    && slots.revision <= self.newest_revision
            })
    }

    pub(super) fn receive_slots(
        &mut self,
        generation: u64,
        slots: EndpointTabSlots,
    ) -> Option<(Box<ClientShellSnapshot>, bool)> {
        if !self.supported || self.generation.is_some_and(|current| generation < current) {
            return None;
        }
        if self.generation != Some(generation) {
            self.reset_generation(Some(generation));
        }
        if self.retired_boots.contains(&slots.boot_id) {
            return None;
        }
        if self.boot_id.as_deref() == Some(&slots.boot_id) {
            if slots.revision < self.newest_revision {
                return None;
            }
            self.newest_revision = slots.revision;
        }
        if self.pending_slots.as_ref().is_some_and(|pending| {
            pending.boot_id == slots.boot_id && pending.revision > slots.revision
        }) {
            return None;
        }
        // A revision is immutable. A conflicting duplicate cannot make the old
        // accepted map appear current; preserve the display and gate actions.
        if self.boot_id.as_deref() == Some(&slots.boot_id)
            && self.accepted_revision == Some(slots.revision)
            && *self.slots != slots.slots
        {
            self.accepted_revision = None;
        }
        self.pending_slots = Some(slots);
        self.pending_snapshot.take()
    }

    fn reset_generation(&mut self, generation: Option<u64>) {
        let slots = std::mem::take(&mut self.slots);
        *self = Self {
            supported: self.supported,
            generation,
            slots,
            ..Self::default()
        };
    }

    pub(super) fn prepare_snapshot(
        &mut self,
        generation: Option<u64>,
        snapshot: Box<ClientShellSnapshot>,
        acknowledge: bool,
    ) -> Option<Box<ClientShellSnapshot>> {
        if !self.supported {
            return Some(snapshot);
        }
        if self
            .generation
            .zip(generation)
            .is_some_and(|(current, next)| next < current)
        {
            return None;
        }
        if self.generation != generation {
            self.reset_generation(generation);
        }
        if self.retired_boots.contains(&snapshot.boot_id) {
            return None;
        }
        if self.boot_id.as_deref() != Some(&snapshot.boot_id) {
            if let Some(previous) = self.boot_id.replace(snapshot.boot_id.clone()) {
                self.retired_boots.insert(previous);
            }
            self.newest_revision = snapshot.revision;
            self.accepted_revision = None;
        }
        if snapshot.revision < self.newest_revision {
            return None;
        }
        self.newest_revision = snapshot.revision;
        let Some(slots) = self.pending_slots.as_ref().filter(|slots| {
            slots.boot_id == snapshot.boot_id && slots.revision == snapshot.revision
        }) else {
            self.pending_snapshot = Some((snapshot, acknowledge));
            return None;
        };
        #[cfg(test)]
        {
            self.operations.validations += 1;
            self.operations.workspace_entries += snapshot.workspaces.len();
        }
        #[cfg(test)]
        let validation_started = std::time::Instant::now();
        let valid = valid_slots(&snapshot, &slots.slots, || {
            #[cfg(test)]
            {
                self.operations.tab_memberships += 1;
            }
        });
        #[cfg(test)]
        {
            self.profile_times.0 += validation_started.elapsed();
        }
        if !valid {
            self.pending_snapshot = Some((snapshot, acknowledge));
            return None;
        }
        Some(snapshot)
    }

    pub(super) fn receive_snapshot(
        &mut self,
        generation: Option<u64>,
        snapshot: Box<ClientShellSnapshot>,
        acknowledge: bool,
    ) -> Option<Box<ClientShellSnapshot>> {
        let mut snapshot = self.prepare_snapshot(generation, snapshot, acknowledge)?;
        if !self.supported {
            return Some(snapshot);
        }
        #[cfg(test)]
        let remap_started = std::time::Instant::now();
        let slots = self.pending_slots.as_ref()?;
        let topology_changed = self.workspace_ids.len() != snapshot.workspaces.len()
            || self
                .workspace_ids
                .iter()
                .zip(&snapshot.workspaces)
                .any(|(id, workspace)| id != &workspace.workspace_id)
            || self.structural_ids.len() != snapshot.tabs.len()
            || self
                .structural_ids
                .iter()
                .zip(&snapshot.tabs)
                .any(|(id, tab)| id != &tab.tab_id);
        if topology_changed || *self.slots != slots.slots {
            #[cfg(test)]
            {
                self.operations.order_rebuilds += 1;
            }
            self.structural_ids = snapshot.tabs.iter().map(|tab| tab.tab_id.clone()).collect();
            self.workspace_ids = snapshot
                .workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone())
                .collect();
            self.permutation = (0..snapshot.tabs.len()).collect();
            let workspace_order: HashMap<_, _> = snapshot
                .workspaces
                .iter()
                .enumerate()
                .map(|(index, workspace)| (workspace.workspace_id.as_str(), index))
                .collect();
            self.permutation.sort_by_key(|index| {
                let tab = &snapshot.tabs[*index];
                // Workspace order remains the server's order; only tabs within
                // each workspace receive numeric presentation order.
                (
                    workspace_order[tab.workspace_id.as_str()],
                    slots.slots[&tab.tab_id].slot,
                )
            });
            self.slots = Arc::new(slots.slots.clone());
        }
        // Reuse the permutation for output-only snapshots. No per-render sort.
        #[cfg(test)]
        {
            self.operations.remapped_tabs += snapshot.tabs.len();
        }
        let mut tabs: Vec<_> = std::mem::take(&mut snapshot.tabs)
            .into_iter()
            .map(Some)
            .collect();
        snapshot.tabs = self
            .permutation
            .iter()
            .filter_map(|index| tabs[*index].take())
            .collect();
        self.accepted_revision = Some(snapshot.revision);
        self.pending_snapshot = None;
        #[cfg(test)]
        {
            self.profile_times.1 += remap_started.elapsed();
        }
        Some(snapshot)
    }
}

fn valid_slots(
    snapshot: &ClientShellSnapshot,
    slots: &BTreeMap<String, EndpointTabSlot>,
    mut membership: impl FnMut(),
) -> bool {
    if slots.len() != snapshot.tabs.len() {
        return false;
    }
    let mut occupied = HashSet::new();
    let mut ids = HashSet::new();
    let workspaces: HashSet<_> = snapshot
        .workspaces
        .iter()
        .map(|workspace| workspace.workspace_id.as_str())
        .collect();
    snapshot.tabs.iter().all(|tab| {
        membership();
        ids.insert(&tab.tab_id)
            && slots.get(&tab.tab_id).is_some_and(|slot| {
                slot.slot > 0
                    && slot.workspace_id == tab.workspace_id
                    && workspaces.contains(slot.workspace_id.as_str())
                    && occupied.insert((&slot.workspace_id, slot.slot))
            })
    })
}

impl super::ClientShellState {
    #[cfg(test)]
    pub(crate) fn tab_slot_profile_times(&self) -> (std::time::Duration, std::time::Duration) {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .map(|endpoint| endpoint.tab_slots.profile_times)
            .unwrap_or_default()
    }

    pub(super) fn tab_slot_action_blocked(&mut self, action: crate::input::KeybindAction) -> bool {
        use crate::input::KeybindAction;
        let Some(slots) = self.active_tab_slots() else {
            return false;
        };
        let moving = matches!(
            action,
            KeybindAction::MoveTabPrevious | KeybindAction::MoveTabNext
        );
        if moving {
            self.tab_slot_move_notice();
            return true;
        }
        if !slots.ready()
            && matches!(
                action,
                KeybindAction::SwitchTab(_)
                    | KeybindAction::PreviousTab
                    | KeybindAction::NextTab
                    | KeybindAction::RenumberTab
            )
        {
            self.push_endpoint_notice(
                super::ClientEndpointNoticeKind::Rejected,
                "tab_slots_pending",
                "Tab slots pending",
                "Waiting for matching tab metadata",
            );
            return true;
        }
        false
    }

    pub(super) fn tab_slot_move_notice(&mut self) {
        let binding = self
            .config
            .keybinds
            .keybinds
            .renumber_tab
            .prefix_rhs_label()
            .map(|rhs| format!("prefix+{rhs}"))
            .unwrap_or_else(|| "the renumber tab action".into());
        self.push_endpoint_notice(
            super::ClientEndpointNoticeKind::Rejected,
            "tab_slot_move",
            "Tabs use numeric order",
            format!("Use {binding} to renumber into a free slot"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(revision: u64) -> Box<ClientShellSnapshot> {
        let mut snapshot = super::super::tests::snapshot();
        snapshot.revision = revision;
        for (id, number) in [("tab_3", 3), ("tab_4", 4)] {
            let mut tab = snapshot.tabs[0].clone();
            tab.tab_id = id.into();
            tab.number = number;
            tab.focused = false;
            snapshot.tabs.push(tab);
        }
        Box::new(snapshot)
    }

    fn slots(revision: u64) -> EndpointTabSlots {
        EndpointTabSlots {
            boot_id: "boot-1".into(),
            revision,
            slots: [("tab_1", 1), ("tab_3", 3), ("tab_4", 2)]
                .into_iter()
                .map(|(id, slot)| {
                    (
                        id.into(),
                        EndpointTabSlot {
                            workspace_id: "ws_1".into(),
                            slot,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn tab_slots_pair_both_orders_and_preserve_ids_and_labels() {
        for metadata_first in [false, true] {
            let mut state = TabSlotProjection::new(true);
            let paired = if metadata_first {
                assert!(state.receive_slots(1, slots(1)).is_none());
                state.receive_snapshot(Some(1), snapshot(1), true).unwrap()
            } else {
                assert!(state.receive_snapshot(Some(1), snapshot(1), true).is_none());
                let (snapshot, acknowledge) = state.receive_slots(1, slots(1)).unwrap();
                assert!(acknowledge);
                state
                    .receive_snapshot(Some(1), snapshot, acknowledge)
                    .unwrap()
            };
            assert!(state.ready());
            assert_eq!(
                paired
                    .tabs
                    .iter()
                    .map(|tab| tab.tab_id.as_str())
                    .collect::<Vec<_>>(),
                ["tab_1", "tab_4", "tab_3"]
            );
            assert_eq!(
                paired.tabs.iter().map(|tab| tab.number).collect::<Vec<_>>(),
                [1, 4, 3]
            );
            assert_eq!(paired.focused_tab_id.as_deref(), Some("tab_1"));
            assert_eq!(paired.tabs[0].label, "1");
        }
    }

    #[test]
    fn tab_slots_newer_snapshot_gates_until_exact_pair_and_rejects_old_metadata() {
        let mut state = TabSlotProjection::new(true);
        state.receive_slots(1, slots(1));
        state.receive_snapshot(Some(1), snapshot(1), true).unwrap();
        let order_allocation = state.permutation.as_ptr();
        assert!(state
            .receive_snapshot(Some(1), snapshot(3), false)
            .is_none());
        assert!(!state.ready());
        assert!(state.receive_slots(1, slots(2)).is_none());
        assert_eq!(state.slots["tab_3"].slot, 3);
        let (pending, acknowledge) = state.receive_slots(1, slots(3)).unwrap();
        assert!(!acknowledge);
        state
            .receive_snapshot(Some(1), pending, acknowledge)
            .unwrap();
        assert!(state.ready());
        assert_eq!(
            state.permutation.as_ptr(),
            order_allocation,
            "output revisions reuse numeric order"
        );
        assert!(state.receive_slots(1, slots(1)).is_none());
        assert!(state.receive_snapshot(Some(1), snapshot(1), true).is_none());
        assert!(state.ready());
    }

    #[test]
    fn tab_slots_reject_partial_foreign_zero_and_duplicate_catalogs() {
        for corrupt in 0..4 {
            let mut state = TabSlotProjection::new(true);
            let mut bad = slots(1);
            match corrupt {
                0 => {
                    bad.slots.remove("tab_3");
                }
                1 => bad.slots.get_mut("tab_3").unwrap().workspace_id = "foreign".into(),
                2 => bad.slots.get_mut("tab_3").unwrap().slot = 0,
                _ => bad.slots.get_mut("tab_3").unwrap().slot = 1,
            }
            state.receive_slots(1, bad);
            assert!(state.receive_snapshot(Some(1), snapshot(1), true).is_none());
            assert!(!state.ready());
            let (pending, acknowledge) = state.receive_slots(1, slots(1)).unwrap();
            assert!(state
                .receive_snapshot(Some(1), pending, acknowledge)
                .is_some());
        }
    }

    #[test]
    fn tab_slots_restart_and_generation_reject_reused_old_id_pairs() {
        let mut state = TabSlotProjection::new(true);
        state.receive_slots(1, slots(8));
        state.receive_snapshot(Some(1), snapshot(8), true).unwrap();
        let mut restarted = snapshot(1);
        restarted.boot_id = "boot-2".into();
        assert!(state.receive_snapshot(Some(2), restarted, true).is_none());
        assert!(!state.ready());
        assert_eq!(
            state.slots["tab_3"].slot, 3,
            "retain coherent displayed labels while pending"
        );
        assert!(state.receive_slots(1, slots(8)).is_none());
        let mut metadata = slots(1);
        metadata.boot_id = "boot-2".into();
        metadata.slots.get_mut("tab_3").unwrap().slot = 9;
        let (pending, acknowledge) = state.receive_slots(2, metadata).unwrap();
        state
            .receive_snapshot(Some(2), pending, acknowledge)
            .unwrap();
        assert!(state.ready());
        assert_eq!(state.slots["tab_3"].slot, 9);
    }

    #[test]
    fn tab_slots_absent_capability_retains_legacy_structural_order() {
        let mut state = TabSlotProjection::default();
        let original = snapshot(1);
        let installed = state
            .receive_snapshot(Some(1), original.clone(), true)
            .unwrap();
        assert_eq!(installed, original);
        assert!(!state.ready());
    }

    #[test]
    fn tab_slots_pre_activation_validates_once_and_install_reuses_exact_pair() {
        use super::super::*;
        let mut shell = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        let id = ClientEndpointId::Local;
        shell.set_endpoint_tab_slots_supported(&id, true);
        let mut previous_map = None;
        for revision in 1..=3 {
            shell.set_endpoint_tab_slots_for_generation(&id, 1, slots(revision));
            let prepared = shell
                .prepare_endpoint_slot_snapshot(&id, 1, snapshot(revision), true)
                .unwrap();
            let projection = &shell.endpoints[0].tab_slots;
            assert_eq!(projection.operations.validations, revision as usize);
            assert_eq!(projection.operations.workspace_entries, revision as usize);
            assert_eq!(projection.operations.tab_memberships, revision as usize * 3);
            assert_eq!(projection.operations.order_rebuilds, 1);
            assert_eq!(projection.operations.remapped_tabs, revision as usize * 3);
            let displayed_map = &projection.slots["tab_1"] as *const _;
            if let Some(previous) = previous_map {
                assert_eq!(displayed_map, previous);
            }
            previous_map = Some(displayed_map);
            let before = projection.operations.clone();
            shell.install_prepared_endpoint_snapshot(&id, Some(1), prepared, true);
            let projection = &shell.endpoints[0].tab_slots;
            assert_eq!(projection.operations, before);
            assert_eq!(&projection.slots["tab_1"] as *const _, displayed_map);
        }
    }

    #[test]
    fn tab_slots_workspace_membership_work_is_linear_and_complete() {
        for count in [1, 15] {
            let mut snapshot = snapshot(1);
            let workspace = snapshot.workspaces[0].clone();
            let tab = snapshot.tabs[0].clone();
            snapshot.workspaces = (0..count)
                .map(|index| {
                    let mut ws = workspace.clone();
                    ws.workspace_id = format!("ws_{index}");
                    ws
                })
                .collect();
            snapshot.tabs = (0..count)
                .map(|index| {
                    let mut tab = tab.clone();
                    tab.workspace_id = format!("ws_{index}");
                    tab.tab_id = format!("tab_{index}");
                    tab
                })
                .collect();
            let metadata = EndpointTabSlots {
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
                                slot: 1,
                            },
                        )
                    })
                    .collect(),
            };
            let mut state = TabSlotProjection::new(true);
            state.receive_slots(1, metadata);
            state
                .receive_snapshot(Some(1), snapshot.clone(), true)
                .unwrap();
            assert_eq!(state.operations.workspace_entries, count);
            assert_eq!(state.operations.tab_memberships, count);
            assert_eq!(state.operations.validations, 1);
            snapshot.tabs[count - 1].workspace_id = "absent".into();
            assert!(state.receive_snapshot(Some(1), snapshot, true).is_none());
            assert!(!state.ready());
        }
    }

    #[test]
    fn tab_slots_conflicting_duplicate_gates_actions_until_valid_pair() {
        let mut state = TabSlotProjection::new(true);
        state.receive_slots(1, slots(1));
        state.receive_snapshot(Some(1), snapshot(1), true).unwrap();
        let mut conflict = slots(1);
        conflict.slots.get_mut("tab_3").unwrap().slot = 9;
        state.receive_slots(1, conflict);
        assert!(!state.ready());
        assert_eq!(state.slots["tab_3"].slot, 3);
        state.receive_slots(1, slots(2));
        state.receive_snapshot(Some(1), snapshot(2), true).unwrap();
        assert!(state.ready());
    }

    #[test]
    fn tab_slots_inactive_cache_cannot_relabel_the_retained_visible_snapshot() {
        use super::super::*;
        let mut shell = shell_with_slots();
        let id = ClientEndpointId::Local;
        let presented = shell.presented_tab_slots.clone().unwrap();
        let mut next = slots(2);
        next.slots.get_mut("tab_1").unwrap().slot = 20;
        shell.set_endpoint_tab_slots_for_generation(&id, 1, next);
        let prepared = shell
            .prepare_endpoint_slot_snapshot(&id, 1, snapshot(2), false)
            .unwrap();
        shell.install_prepared_endpoint_snapshot(&id, Some(1), prepared, false);
        assert_eq!(shell.snapshot.as_ref().unwrap().revision, 1);
        assert_eq!(shell.endpoints[0].snapshot.as_ref().unwrap().revision, 2);
        assert!(Arc::ptr_eq(
            shell.presented_tab_slots.as_ref().unwrap(),
            &presented
        ));
        assert_eq!(shell.presented_tab_slots.as_ref().unwrap()["tab_1"].slot, 1);
        shell.apply_cached_endpoint_snapshot(&id);
        assert_eq!(shell.snapshot.as_ref().unwrap().revision, 2);
        assert_eq!(
            shell.presented_tab_slots.as_ref().unwrap()["tab_1"].slot,
            20
        );
    }

    fn shell_with_slots() -> super::super::ClientShellState {
        use super::super::*;
        let mut shell = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        shell.set_endpoint_tab_slots_supported(&ClientEndpointId::Local, true);
        shell.set_endpoint_tab_slots_for_generation(&ClientEndpointId::Local, 1, slots(1));
        shell.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 1, snapshot(1));
        shell
    }

    #[test]
    fn tab_slots_input_uses_exact_slot_and_numeric_neighbor_ids() {
        use crate::api::schema::Method;
        use crate::input::KeybindAction;
        let mut shell = shell_with_slots();
        assert!(
            matches!(shell.endpoint_method_for_action(KeybindAction::SwitchTab(2)), Some(Method::TabFocusSlot(params)) if params.slot == 3 && params.workspace_id == "ws_1")
        );
        assert!(
            matches!(shell.endpoint_method_for_action(KeybindAction::NextTab), Some(Method::TabFocus(params)) if params.tab_id == "tab_4")
        );
        assert!(
            matches!(shell.endpoint_method_for_action(KeybindAction::PreviousTab), Some(Method::TabFocus(params)) if params.tab_id == "tab_3")
        );
        assert!(shell
            .endpoint_method_for_action(KeybindAction::MoveTabNext)
            .is_none());
        assert!(shell.visible_endpoint_notice.is_some());
    }

    #[test]
    fn tab_slots_pending_keeps_presentation_and_consumes_numeric_input_without_fallback() {
        use crate::input::{KeybindAction, KeybindMatch};
        let mut shell = shell_with_slots();
        let before = shell.snapshot.clone();
        shell.set_endpoint_snapshot_for_generation(
            &super::super::ClientEndpointId::Local,
            1,
            snapshot(2),
        );
        assert_eq!(shell.snapshot, before);
        let mut outcome = super::super::ClientShellInput::default();
        shell.record_binding(
            KeybindMatch::Action(KeybindAction::SwitchTab(0)),
            &mut outcome,
        );
        assert!(outcome.actions.is_empty());
        assert!(outcome.requests.is_empty());
        assert!(shell
            .endpoint_method_for_action(KeybindAction::NextTab)
            .is_none());
        assert!(shell
            .endpoint_method_for_action(KeybindAction::PreviousTab)
            .is_none());
        assert_eq!(shell.snapshot, before);
    }

    #[test]
    fn tab_slots_renumber_dialog_preserves_title_and_dispatches_explicit_slot() {
        use super::super::*;
        let mut shell = shell_with_slots();
        let original_label = shell.snapshot.as_ref().unwrap().tabs[0].label.clone();
        let mut outcome = ClientShellInput::default();
        shell.open_renumber_tab_overlay(&mut outcome);
        let Some(ClientShellOverlay::Rename(rename)) = shell.overlay.as_mut() else {
            panic!("renumber dialog");
        };
        rename.input = TextEditor::new("12", false);
        shell.save_rename_overlay(&mut outcome);
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::TabRenumber(params) if params.slot == 12 && params.tab_id == "tab_1"))
        );
        assert_eq!(
            shell.snapshot.as_ref().unwrap().tabs[0].label,
            original_label
        );
    }
}
