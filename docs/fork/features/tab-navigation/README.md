# Tab navigation

Independent feature based on upstream stable v0.9.5
(`2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`).

## Last visited tab

Press the same configured prefix twice to return to the most recently visited
surviving tab in the current workspace. With Ctrl+A as prefix, Ctrl+A Ctrl+A
toggles the last two tabs. `previous_tab` and `next_tab` still select neighbors.
Each attached client has independent server-owned history for each workspace.
Public CLI/API focus retains its existing session-wide behavior and records each
affected client's own previous tab. Preview, background creation, same-tab focus,
failed requests and temporary default-focus projection do not create visits.
History is ephemeral and resets on restart, handoff or reconnect.

The fork default requires no configuration migration. To restore literal prefix
behavior, use:

```toml
[keys]
double_prefix = "send_prefix"
```

`last_tab` and `send_prefix` are also independently configurable keybindings. The
latter sends the prefix that entered prefix mode; a direct binding sends the
primary configured prefix. With multiple prefixes, repeating the same one invokes
`double_prefix`; pressing two different prefixes sends the second key literally.
For [Ctrl+A, Ctrl+B], AA/BB toggle and AB/BA send literal B/A respectively.
An explicit `send_prefix` binding sends one balanced Press/Release tap to the
same pane; physical binding releases cannot leave the synthetic prefix held.
Copy mode retains its existing explicit exit and focus-parking behavior.

The additive advertised JSON API method `tab.last` takes `workspace_id`. A shell
request resolves its own server-owned history; a public request resolves the
independent default history. No surviving previous tab is a successful no-op.
An endpoint lacking this method consumes the shortcut and shows a local notice.
It remains connected and receives no accidental prefix byte.

## Stable slots

Slots are positive, recyclable user numbers separate from stable public IDs and
`Tab.number`. Closing 2 leaves 3 unchanged; creating a tab fills the smallest gap
and displays [1,2,3]. Titles retain a visible numeric prefix. Numeric shortcuts
address slots; next/previous follow numeric display order. Slot changes do not
retarget history, panes, launch environment IDs or Goto activation.

Storage order remains independent. The published `tab.move(insert_index)` keeps
its vector insertion semantics, response and event order, including for old
clients. A slot-aware TUI displays numeric order and blocks drag/move reordering
with an explicit renumber notice. Use `prefix+.` (`keys.renumber_tab`) to assign a
free positive 32-bit slot. Explicit renumber rejects occupied numbers without
swapping; assigning the current slot is a no-op. Larger slots remain reachable
through next/previous navigation. The additive JSON methods are
`tab.renumber(tab_id, slot)` and `tab.focus_slot(workspace_id, slot)`. The latter
resolves the server's current slot atomically, so displayed indices never become
legacy positional targets. `TabInfo.slot` is optional JSON metadata;
`TabInfo.number` retains its published identity meaning.

Slots persist with each tab. Old snapshots and malformed/zero/duplicate slots
are repaired deterministically: reserve all first valid unique slots, then fill
missing values in saved structural order. Restore resolves the saved active tab
and terminal histories before accounting for any skipped tabs. Downgrades ignore
slots; a subsequent upgrade reconstructs missing values and cannot promise the
previous custom numbers after an old version saves the session.

The optional `tab_slots` capability supplies `endpoint.tab-slots.v1` alongside
each emitted frozen snapshot with the same boot ID and projection revision.
Slot-only changes advance the projection even when frozen fields are identical.
Clients validate full tab coverage, membership and unique positive slots, then
install the pair atomically. An advertised but pending/stale companion gates
numeric navigation and keeps the last coherent presentation; only endpoints
without the capability use legacy positional behavior. Numeric order is cached
on topology/slot changes. No per-render metadata API requests are made.
The activation gate validates each exact pair once and transfers the prepared
snapshot into installation. Workspace membership is indexed once per validation;
output-only snapshots reuse numeric order and the displayed slot map. Applying
the cached permutation to fresh tab data remains linear in the tab count.
Desktop scroll indicators occupy separate cells from slot digits; narrow tab
switchers and compact status use the same accepted slot metadata.

## Compatibility and validation

Generation-1 codecs, frozen snapshots, enum tags, method fixtures and public ID
meaning remain unchanged. Last-tab adds an advertised JSON method, using the
existing response envelope. Success hooks separate synchronous handlers from
deferred worktree completion. Request acceptance is not a committed visit.

Stage 1 passed native `just ci 'not binary(live_handoff)'`: formatting, clippy,
3,839 tests (12 skipped), 150 maintenance tests, workflow/architecture and bundled
integration tests. This includes 15 navigation/input regressions and unchanged
frozen codec/method contracts. Documentation contracts passed (7 tests).
Windows cross-lint was attempted but requires an unconfigured local Windows SDK;
the isolated runtime smoke is recorded below.
Stage 2 passed full native Linux `just ci all()`: formatting, clippy, 4,071 tests
(15 ignored), including the 24-test `live_handoff` binary, 150 maintenance tests,
workflow/architecture and integration asset checks. Documentation contracts passed
(7 tests). The generated API schema and independent additive method digests are
current; published codec and method fixtures are unchanged.
Focused explicit-prefix (1), slots (25), tab/navigation (141), endpoint (266),
public pane-move (3) and advertised-method (2) checks passed. These overlap the
full suite and are not additional test totals. They cover balanced prefix-mode/direct
binding taps, held-input cleanup and legacy literal-byte encoding; duplicate or
pending metadata gates; one-validation/cache operation bounds; wide/narrow slot
labels; and failed imported-runtime skip/history alignment with strict handoff
rejection, shell/argv/moved-pane gap allocation, launch identity, metadata-only
seen/focus behavior, and retained visible slot maps across inactive cache updates.
Initial fixture compilation, capability expectation and lint failures were fixed
before the final full run. Negotiated client-path scaling and isolated named-session
runtime smoke passed, as did independent frozen-object review. The upstream main
render scaling cases passed; its complete recipe remains limited by unavailable
CoW retention in one native graphics profile. See the [validation record](validation.md)
for measurements, runtime transitions and precise limits.
Cross-platform release validation and combined Goto/copy-mode checks belong to
integration. No claim of released, installed or deployed behavior is made here.

When upgrading the stable base, recheck client location/default focus authority,
response lifetimes, deferred completion fences, topology reconciliation and
key-ownership behavior. Preserve the structural/numeric order split and compare
all published endpoint contracts before transplanting the feature.
