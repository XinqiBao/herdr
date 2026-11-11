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
Copy mode retains its existing explicit exit and focus-parking behavior.

The additive advertised JSON API method `tab.last` takes `workspace_id`. A shell
request resolves its own server-owned history; a public request resolves the
independent default history. No surviving previous tab is a successful no-op.
An endpoint lacking this method consumes the shortcut and shows a local notice.
It remains connected and receives no accidental prefix byte.

## Stable slots: accepted next stage

Slots are positive, recyclable user numbers separate from stable public IDs and
`Tab.number`. Closing 2 leaves 3 unchanged; creating a tab fills the smallest gap
and displays [1,2,3]. Titles retain a visible numeric prefix. Numeric shortcuts
address slots; next/previous follow numeric display order. Slot changes do not
retarget history, panes, launch environment IDs or Goto activation.

Storage order remains independent. The published `tab.move(insert_index)` keeps
its vector insertion semantics, response and event order, including for old
clients. A slot-aware TUI displays numeric order and blocks drag/move reordering
with an explicit renumber notice. Explicit renumber rejects occupied numbers
without swapping. The slots implementation and its validation are pending.

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
isolated runtime smoke remains pending.
Cross-platform release validation and combined Goto/copy-mode checks belong to
integration. No claim of released, installed or deployed behavior is made here.

When upgrading the stable base, recheck client location/default focus authority,
response lifetimes, deferred completion fences, topology reconciliation and
key-ownership behavior. Preserve the structural/numeric order split and compare
all published endpoint contracts before transplanting the feature.
