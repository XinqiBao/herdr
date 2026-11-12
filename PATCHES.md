# Personal patches

Base: upstream Herdr v0.9.5. This fork carries independent personal changes;
it does not publish to the official update channels.

## Copy-mode viewport navigation

- `H`, `M`, `L`: move to the top, middle (upper middle for even heights),
  or bottom visible text row of the current pane viewport, at column zero as in
  tmux. Rows covered by the mode bar are excluded without scrolling the pane.
- `Ctrl+e` / `Ctrl+y`: scroll toward newer / older text by one row, keeping
  the cursor on its text unless it must move to stay within the viewport.
  Both character and line selections keep their anchors.
- Client-only implementation in `src/client/shell/copy_mode.rs`; no server or
  protocol changes. Search prompts retain their normal text-editing behavior.
- Upstream discussion: https://github.com/herdrdev/herdr/discussions/1322.
- Remove when an upstream stable version provides equivalent navigation.
## Goto terminal preview

- Preview the selected pane's visible text without changing focus, geometry, or
  agent seen state. Reads use the existing public API for local and saved SSH
  endpoints; keyboard navigation remains provisional until Enter.
- Reconcile preview selection when matching metadata changes, including other
  endpoints, and discard old selection epochs before accepting async reads.
  Output-only revisions do not rebuild preview selection. An explicit stale SSH
  executable report invalidates cached metadata so a later read can rediscover it.
- Upstream discussion: https://github.com/herdrdev/herdr/discussions/3242.
- Remove when an upstream stable version provides equivalent read-only previews.
