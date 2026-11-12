//! User slots never participate in public identity or structural tab ordering.
use std::collections::HashSet;

use super::Workspace;

impl Workspace {
    pub(crate) fn next_tab_slot(&self) -> std::io::Result<u32> {
        let occupied: HashSet<_> = self.tabs.iter().map(|tab| tab.slot).collect();
        smallest_free(&occupied).ok_or_else(|| std::io::Error::other("tab slots exhausted"))
    }

    pub(crate) fn tab_slots_changed(&mut self) {
        self.tab_slots_revision = self.tab_slots_revision.wrapping_add(1);
    }

    /// Atomic metadata-only mutation. Same-slot requests succeed without invalidation.
    pub(crate) fn renumber_tab(&mut self, index: usize, slot: u32) -> Result<bool, &'static str> {
        if slot == 0 {
            return Err("slot must be a positive 32-bit integer");
        }
        let Some(tab) = self.tabs.get(index) else {
            return Err("tab no longer exists");
        };
        if tab.slot == slot {
            return Ok(false);
        }
        if self.tabs.iter().any(|tab| tab.slot == slot) {
            return Err("slot is already occupied");
        }
        self.tabs[index].slot = slot;
        self.tab_slots_changed();
        Ok(true)
    }
}

fn smallest_free(occupied: &HashSet<u32>) -> Option<u32> {
    (1..=u32::MAX).find(|slot| !occupied.contains(slot))
}

/// Reserve later valid values before repairing missing/invalid/duplicate entries.
/// Saved order remains untouched, including entries whose panes later fail restore.
pub(crate) fn repair_tab_slots(slots: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut occupied = HashSet::new();
    let mut result: Vec<_> = slots
        .into_iter()
        .map(|slot| {
            if slot > 0 && occupied.insert(slot) {
                slot
            } else {
                0
            }
        })
        .collect();
    let mut candidate = 1;
    for slot in &mut result {
        if *slot == 0 {
            while occupied.contains(&candidate) {
                candidate = candidate.checked_add(1).expect("restorable slot capacity");
            }
            *slot = candidate;
            occupied.insert(candidate);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_slots_reuse_gaps_without_reusing_identity_or_reordering() {
        let mut ws = Workspace::test_new("slots");
        ws.test_add_tab(None);
        ws.test_add_tab(None);
        let survivor = (ws.tabs[2].number, ws.tabs[2].root_pane);
        assert!(ws.close_tab(1));
        ws.test_add_tab(None);
        assert_eq!(
            ws.tabs.iter().map(|tab| tab.slot).collect::<Vec<_>>(),
            [1, 3, 2]
        );
        assert_eq!((ws.tabs[1].number, ws.tabs[1].root_pane), survivor);
        assert_eq!(ws.tabs[2].number, 4);
        let active = ws.tabs[ws.active_tab].root_pane;
        assert!(ws.move_tab(0, 3));
        assert_eq!(
            ws.tabs.iter().map(|tab| tab.slot).collect::<Vec<_>>(),
            [3, 2, 1]
        );
        assert_eq!(ws.tabs[ws.active_tab].root_pane, active);
        ws.assert_invariants_for_test();
    }

    #[test]
    fn tab_slots_renumber_is_atomic_metadata_with_no_implicit_swap() {
        let mut ws = Workspace::test_adversarial_identity_state();
        let before: Vec<_> = ws
            .tabs
            .iter()
            .map(|tab| (tab.number, tab.root_pane, tab.slot))
            .collect();
        let active = ws.active_tab;
        let launch_env = ws.launch_env_for_new_pane(ws.tabs[0].number, 91, vec![]);
        let revision = ws.tab_slots_revision;
        assert!(ws.renumber_tab(0, 0).is_err());
        assert!(ws.renumber_tab(0, ws.tabs[1].slot).is_err());
        assert_eq!(ws.renumber_tab(0, ws.tabs[0].slot), Ok(false));
        assert_eq!(ws.tab_slots_revision, revision);
        assert_eq!(
            ws.tabs
                .iter()
                .map(|tab| (tab.number, tab.root_pane, tab.slot))
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(ws.renumber_tab(0, u32::MAX), Ok(true));
        assert_eq!(ws.active_tab, active);
        assert_eq!(ws.tabs[0].number, before[0].0);
        assert_eq!(ws.tabs[0].root_pane, before[0].1);
        assert_eq!(
            ws.launch_env_for_new_pane(ws.tabs[0].number, 91, vec![]),
            launch_env
        );
        assert_eq!(ws.tab_slots_revision, revision + 1);
        ws.assert_invariants_for_test();
    }

    #[test]
    fn tab_slots_repair_reserves_later_valid_values() {
        assert_eq!(
            repair_tab_slots([0, 3, 3, 0, 1, u32::MAX]),
            [2, 3, 4, 5, 1, u32::MAX]
        );
        assert_eq!(repair_tab_slots([0, 0, 0]), [1, 2, 3]);
    }

    #[tokio::test]
    async fn tab_slots_shell_and_argv_creation_fill_gap_keep_public_identity() {
        for argv in [false, true] {
            let mut ws = Workspace::test_new("creation");
            ws.test_add_tab(None);
            ws.test_add_tab(None);
            assert!(ws.close_tab(1));
            let number = ws.next_public_tab_number;
            let command = crate::app::exiting_test_command();
            let cwd = std::env::current_dir().unwrap();
            let (index, _terminal, runtime) = if argv {
                ws.create_tab_argv_command(
                    24,
                    80,
                    cwd,
                    &[command.into()],
                    vec![],
                    0,
                    crate::terminal_theme::TerminalTheme::default(),
                    None,
                )
            } else {
                ws.create_tab(
                    24,
                    80,
                    cwd,
                    0,
                    crate::terminal_theme::TerminalTheme::default(),
                    None,
                    crate::pane::PaneShellConfig::new(
                        command,
                        crate::config::ShellModeConfig::NonLogin,
                    ),
                    vec![],
                )
            }
            .unwrap();
            assert_eq!(ws.tabs[index].slot, 2);
            assert_eq!(ws.tabs[index].number, number);
            assert_eq!(
                ws.launch_env_for_new_pane(number, 91, vec![]),
                crate::pane::PaneLaunchEnv::from_extra(vec![]).with_identity(
                    ws.id.clone(),
                    super::super::public_tab_id_for_number(&ws.id, number),
                    super::super::public_pane_id_for_number(&ws.id, 91)
                )
            );
            ws.assert_invariants_for_test();
            runtime.shutdown();
        }
    }
}
