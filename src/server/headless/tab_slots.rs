use std::collections::BTreeMap;
use std::sync::Arc;

use crate::protocol::endpoint::EndpointTabSlot;

#[derive(Default)]
pub(super) struct TabSlotCatalog {
    revisions: Vec<(String, u64)>,
    slots: Arc<BTreeMap<String, EndpointTabSlot>>,
}

impl TabSlotCatalog {
    /// Run once before fanout. Output/geometry-only revisions reuse the catalog;
    /// no tab collection or sorting takes place on that path.
    pub(super) fn capture(
        &mut self,
        state: &crate::app::AppState,
    ) -> Arc<BTreeMap<String, EndpointTabSlot>> {
        if self.revisions.len() != state.workspaces.len()
            || self
                .revisions
                .iter()
                .zip(&state.workspaces)
                .any(|((id, revision), ws)| id != &ws.id || *revision != ws.tab_slots_revision)
        {
            self.revisions = state
                .workspaces
                .iter()
                .map(|ws| (ws.id.clone(), ws.tab_slots_revision))
                .collect();
            self.slots = Arc::new(
                state
                    .workspaces
                    .iter()
                    .flat_map(|ws| {
                        ws.tabs.iter().map(|tab| {
                            (
                                crate::workspace::public_tab_id_for_number(&ws.id, tab.number),
                                EndpointTabSlot {
                                    workspace_id: ws.id.clone(),
                                    slot: tab.slot,
                                },
                            )
                        })
                    })
                    .collect(),
            );
        }
        Arc::clone(&self.slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_slots_catalog_reuses_output_projection_and_invalidates_metadata() {
        let mut state = crate::app::AppState::test_new();
        state
            .workspaces
            .push(crate::workspace::Workspace::test_new("slots"));
        let mut cache = TabSlotCatalog::default();
        let first = cache.capture(&state);
        state.workspaces[0].tabs[0].set_custom_name("title".into());
        assert!(Arc::ptr_eq(&first, &cache.capture(&state)));
        state.workspaces[0].renumber_tab(0, 8).unwrap();
        let changed = cache.capture(&state);
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(changed.values().next().unwrap().slot, 8);
        assert!(Arc::ptr_eq(&changed, &cache.capture(&state)));
    }
}
