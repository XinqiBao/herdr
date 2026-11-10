# Personal patches

Base: upstream Herdr v0.9.5. This branch contains only the Goto feature.

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
