# Goto terminal preview

Goto shows a read-only preview of the selected terminal before the user commits
navigation. Open Goto with the configured `goto` binding (default: prefix then
`g`), filter or move through its entries, and press Enter to activate a target.
Closing Goto leaves the original view selected. Workspace entries have no
terminal preview; pane entries show visible terminal text. The preview sits
beside the list when space permits and stacks below it in narrow layouts.

## Provenance and scope

- Official base: Herdr `v0.9.5`,
  `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`.
- Accepted implementation: `9b230227bc7721258e416ffc72c2c5980b9ceced`
  (`feat: preview terminal text in goto`) on `xbao/feat/goto`.
- Independent feature: no copy-mode, tab-slot or fork CI dependency. Integration
  with later navigation features must be reviewed separately.
- Upstream discussion: <https://github.com/herdrdev/herdr/discussions/3242>.

This directory records the accepted feature contract and maintenance boundaries.
It is not an installation instruction or a claim about the latest rolling release.

## Protected behavior

Selection in Goto is provisional. Preview reads must preserve active workspace,
tab and pane, terminal geometry, agent seen state, and PTY input. Enter uses the
existing endpoint-qualified activation route; a preview never sends a focus or
input command. Preview hit areas remain separate from list hit areas.

Each selection epoch is distinct, including A → B → A. Responses are accepted
only for the pending request, endpoint connection generation, cached server boot
identity and current pane target. Closing/reopening the overlay, removing a
target, endpoint retirement, availability changes and reconnects invalidate old
results. A response must also contain the requested pane's Visible/Text read.

Filtering can change the effective fallback selection without keyboard input.
Relevant metadata from any endpoint therefore reconciles selection immediately,
before the runtime polls completed reads. This ordering prevents a response for
the previous selection from being accepted between metadata delivery and the
next preview timer tick. Empty results can acquire a preview when a matching pane
appears. Query membership, ordering and effective agent status changes participate
in this invalidation; ordinary terminal output revisions do not.

The preview is a sampled read for a selection, not a continuously refreshed
terminal stream. Output alone does not schedule another read or rebuild preview
selection. Moving away and returning, or reopening, starts a new epoch.

## Transport, bounds and compatibility

Preview uses the existing public `pane.read` JSON method with `source=visible`,
`format=text`, ANSI stripping and a 200-line request. It does not extend a frozen
endpoint codec, add protocol variants, or change an existing API method shape.
Local API routing and enabled saved SSH profiles are supported. The preview
worker is separate from interactive endpoint commands, so reading an inactive
endpoint does not activate it or occupy the interactive command queue.

| Bound | Contract |
| --- | --- |
| Selection debounce | 100 ms |
| Concurrency | One actual worker/request, including cleanup; no FIFO backlog |
| UI timeout | Unavailable after 2 seconds while awaiting the worker |
| Socket response read | 2-second deadline after setup/write |
| Response frame | At most 1 MiB |
| Displayed text | At most 200 lines and 256 KiB of sanitized content |

Control characters are removed and the text limit preserves UTF-8 boundaries.
Empty terminals and unavailable previews have explicit local messages. An older
server without `pane.read`, an offline endpoint, or a failed read disables only
the preview; it does not reject a compatible endpoint connection.

The UI timeout does not release an executing worker. SSH setup retains the
existing bounded noninteractive discovery deadline (15 seconds); bridge cleanup
must finish before another worker starts. On a failed cached SSH read, diagnostic
collection may add the existing bounded one-second wait. Only the explicit
stale-executable metadata marker invalidates the executable cache. A later read
can rediscover the executable; there is no immediate retry or parallel recovery.
Other failures and successful reads leave that cache intact.

Two routing limits remain:

- A direct `--remote` connection may not provide a usable public API target;
  preview availability is not guaranteed for that path.
- The public read response cannot atomically prove server boot identity across
  a restart. Client generation/boot fences reject observed stale connections,
  but cannot prove that an unseen restart did not occur during a public read.

## Verification record

Verification below describes the accepted implementation, not a test run caused
by this documentation change. Tests and implementation live in
[the client preview reader](../../../../src/client/navigator_preview.rs),
[shell preview state](../../../../src/client/shell/navigator_preview.rs),
[preview regression tests](../../../../src/client/shell/tests/navigator_preview.rs)
and [server preservation tests](../../../../src/server/headless/tests/mod.rs).

- Four project regressions failed before the selection fix: filtered fallback,
  cross-endpoint fallback, asynchronous round-trip and empty-filter recovery.
  The final focused preview run passed 18 tests. Additional coverage includes
  response arrival before tick, query changes, reconnect/availability epochs,
  cancellation, bounds, sanitization, layout and output-only no-rebuild behavior.
- A real local socket with a bridge diagnostic double verifies that only a
  failed read using explicitly stale cached SSH metadata invalidates the cache.
  This is behavioral seam coverage, not a live remote executable-move test.
- The server regression
  `public_visible_preview_read_preserves_focus_geometry_seen_state_and_pty_input`
  checks the read-only runtime contract.
- Native fmt and all-target clippy passed. The final native nextest run completed
  with 3,843 passed and 13 skipped, excluding `live_handoff` as upstream macOS CI
  does. One skip is the manual metadata profile. An earlier partial suite failed
  the unchanged shutdown-log test; its isolated run and final no-fail-fast suite
  passed. The original failure's cause was not established.
- Maintenance (150), workflow (5), architecture (6), integration assets
  (18 + 9 + 28), and docs contracts (7) passed.

The widened path is metadata delivery while Goto is open: a comparison of the
endpoint's workspace/tab/pane/agent membership and ordering inputs, O(W + T + P + A)
per delivery per attached client. It allocates no rows unless relevant metadata
changes. Render/layout loops gain no terminal access, I/O or locking. A test
counter verifies that output, surface, timer and compose updates do not add
preview selection resolutions.

A fixed 120×30 native debug profile used 5,000 deliveries per sample, median of
three samples, with 1 and 15 populated panes/agents. Open-versus-closed Goto added
0.392/1.527 µs per output-only delivery and 4.597/22.261 µs for relevant metadata
including row resolution. These include existing snapshot/cache work and support
the metadata bound; they are not exact pre-patch deltas, release throughput,
many-client scaling or live SSH measurements.

Feature-local validation did not establish Windows, Linux, live SSH or attached
multi-client runtime behavior. Combined product/platform delivery is a separate
integration responsibility.

## Upstream upgrades and removal

When transplanting onto a newer stable base, review endpoint snapshot delivery,
effective selection/filter ordering, status projection, catalog retirement,
reader polling order, public `pane.read` behavior and SSH metadata recovery.
Keep invalidation event-driven: do not rebuild navigator rows every frame or
for every output revision. Re-run focused stale-result and preservation tests;
profile 1/15-pane scaling if metadata or presentation work widens.

Later tab-navigation integration must preserve stable endpoint-qualified targets
when display ordering changes. Preview remains provisional and must not enter
last-tab history; only confirmed activation may do so. Optional metadata must
not turn display indices into persistent tab identities.

Remove the fork delta when an upstream stable release supplies equivalent
read-only previews, selection invalidation and bounded transport behavior. Check
the protected regressions before removing either implementation or coverage.
