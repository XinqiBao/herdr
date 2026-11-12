# Validation record

Implementation reviewed at `fca417314251a7773fcd913d7b3df876784f6a0e`, following
the separate explicit-prefix release fix `4c99ee55`. This record adds evidence
without changing that source tree.

## Native checks

Linux `just ci all()` passed: 4,071 tests, 15 ignored, including the 24-test
`live_handoff` binary; formatting, clippy, 150 maintenance tests, workflow,
architecture and integration assets passed. Documentation contracts passed
(7 tests). Generated API schema and independent additive method digests passed;
frozen codecs and the published method fixture did not change. Initial fixture,
capability-expectation and lint failures were corrected before the complete run.

Focused regressions prove one validation per exact accepted pair, one indexed
workspace-membership check per tab, cached permutation/map reuse on output-only
projections, and retained visible metadata across newer inactive cache entries.
They also cover balanced explicit-prefix taps, malformed/pending metadata,
creation and moved-pane gaps, identity/launch environment, numeric UI labels,
failed-import restore alignment and strict handoff rejection.

## Negotiated projection profile

`just bench-tab-slots` passed in release mode with 40 measured output-only
iterations after seeding the caches, fixed 120×40 geometry and populated terminal
cores. Workspace and tab cardinalities vary independently. The harness traverses
welcome JSON/capability negotiation, production companion/snapshot framing and
decoding, preactivation validation, cached ordering and installation. One or three
server clients are attached through test writer channels. These are in-process
client-path measurements, not network latency or end-to-end PTY throughput.

Each cell below is median/p95 microseconds. Client stages are per client;
unchanged catalog capture occurs once before fanout.

| Workspaces | Tabs/workspace | Clients | Catalog capture | Validation | Cached remap |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 0.04/0.16 | 0.40/1.01 | 0.15/0.49 |
| 1 | 1 | 3 | 0.04/0.07 | 0.39/0.47 | 0.16/0.19 |
| 1 | 15 | 1 | 0.04/0.11 | 3.34/4.02 | 0.82/1.18 |
| 1 | 15 | 3 | 0.06/0.11 | 3.31/4.31 | 0.84/1.32 |
| 15 | 1 | 1 | 0.12/0.20 | 3.51/4.12 | 0.96/1.28 |
| 15 | 1 | 3 | 0.12/0.18 | 3.61/4.14 | 0.87/1.09 |

The initial catalog rebuild took 1.13–6.79µs across these cases. Unchanged output
revisions retained the same catalog Arc. Complete validation still visits every
tab and performs ordered-map lookups; membership uses a workspace hash index.
Remapping fresh tab data is linear. Sorting and map cloning occur only when
topology or slots change; installation shares the visible map by Arc.

The following production stages are reported separately as median microseconds.
Snapshot/companion wire costs include JSON and framing; decode includes both
controls. Installation includes active snapshot application.

| Workspaces | Tabs/workspace | Clients | Projection | Snapshot wire | Slots wire | Decode | Install |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 5.31 | 4.77 | 1.23 | 8.38 | 2.65 |
| 1 | 1 | 3 | 5.05 | 4.85 | 1.21 | 8.27 | 2.70 |
| 1 | 15 | 1 | 39.23 | 11.78 | 7.16 | 23.50 | 9.77 |
| 1 | 15 | 3 | 39.71 | 12.13 | 7.24 | 24.14 | 9.63 |
| 15 | 1 | 1 | 42.58 | 17.45 | 7.35 | 33.19 | 15.02 |
| 15 | 1 | 3 | 41.90 | 17.54 | 7.10 | 32.25 | 14.76 |

A separate phase changes a slot and measures actual server `render_and_stream`
fanout, including catalog invalidation and render work. Median/p95 milliseconds:

| Workspaces | Tabs/workspace | One client | Three clients |
| ---: | ---: | ---: | ---: |
| 1 | 1 | 1.19/1.37 | 3.78/4.32 |
| 1 | 15 | 1.31/1.60 | 5.02/5.43 |
| 15 | 1 | 1.30/1.60 | 4.06/4.53 |

These samples support the cache and cardinality claims. They do not establish a
stable-versus-candidate performance comparison or replace combined release smoke.

## Upstream render profile

`just bench-render-scale` ran seven ignored profiles: six passed. The primary
fixed-geometry profile completed its 1/15/50 cases. For 1→15 background workspaces,
combined median was 1,099→1,194µs (1.09×); for 1→15 active panes it was
1,152→1,297µs (1.13×). The negotiated profile above supplies the companion path
that this existing benchmark does not exercise.

The complete recipe **did not pass**: the untouched native-file graphics profile
requires CoW source retention and rejected decoded fallback. An independent
private `/var/tmp` `cp --reflink=always` probe returned `Operation not supported`.
No unrelated source or assertion was changed. This filesystem limitation leaves
that graphics scenario unqualified.

## Isolated runtime smoke

A disposable named session ran the checkout's debug binary, with separate sockets
and a test-only config containing `prefix = "ctrl+a"`; no double-prefix override
or daily configuration edit was needed. Actual TUI input and rendered active-tab
styling confirmed last-tab toggling in both directions, numeric next/previous
wraparound and exact numeric shortcut selection.

Closing slot 2 and creating a tab reused slot 2 with a new public number 4.
Renumbering original tab 1 to slot 20 preserved its public identity. API structural
numbers stayed `[1,3,4]`, while the TUI displayed slots `[2,3,20]`. Occupied-slot
renumbering failed and `tab.focus_slot(20)` focused the original tab. A cold named
session restart preserved slots, stable IDs and structural order. The exact test
session was stopped/deleted, its socket absence verified, and only its newly
created outer pane was closed. Daily services were not replaced or stopped.

Independent frozen source review passed. Windows SDK/cross-platform runtime and
combined Goto/copy-mode release validation remain integration responsibilities.
