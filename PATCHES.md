# Xbao fork delivery ledger

Official base: `herdrdev/herdr` tag `v0.9.5`, commit
`2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`. This fork publishes only through its
own rolling `xbao-latest` release and leaves official update channels unchanged.
See [FORK_WORKFLOW.md](FORK_WORKFLOW.md) and the [feature index](docs/fork/README.md).

Feature branches retain incremental development history and independent worktrees.
Mainline stays linear, normally one squash per feature; a short meaningful
sequence is allowed. Integration commits record `Feature-Branch`, `Feature-Commit`
and `Upstream-Base`. Subsequent updates apply only the delta since the recorded
source, or reconstruct all reviewed base-relative patches after a baseline upgrade.

| Feature | Retained source branch | Integrated source |
| --- | --- | --- |
| Copy mode and mouse history | `xbao/feat/copy-mode` | `b6192872ae5d8a6dd6a6bfd091423a46e5b63328` |
| Goto terminal preview | `xbao/feat/goto` | `e20e5212b44b40a9426ce8e68fb99980ecd5cc01` |
| Fork infrastructure | `xbao/infra` | `5d709929368cc4d84ea16920e9121223755f3539` |

## Copy mode and mouse history

H/M/L select visible top/middle/bottom text; Ctrl+e/y scroll one line while
preserving text position and selection anchors. Host-history mouse browsing on
capable servers captures input before asynchronous focus/wheel completion, so
immediate Ctrl+D cannot leak into the child. Reaching live bottom keeps copy mode;
existing explicit Esc/q/copy exits remain. The additive `pane.wheel` API resolves
HostScroll, MouseReport and AlternateScroll atomically at the runtime.

Application wheels retain exact ordering and input ownership. Failed/cancelled
later wheels close already-forwarded keys/mouse gestures once at eligible original
targets, discard uncertain new input and never replay it to a replacement
connection. Application input for B parks A's retained copy session independent
of focus/response order; retired motion/search callbacks cannot mutate A.
Old endpoints retain explicit copy and legacy wheel behavior with a limitation
notice; automatic immediate-key capture requires advertised `pane.wheel`.
See the [feature record](docs/fork/features/copy-mode/README.md).

## Goto terminal preview

Preview visible terminal text through existing local/saved-SSH public reads,
without focus, geometry, seen-state or input changes. Selection stays provisional
until activation; debounce/bounded reads and stable selection epochs reject late
responses. Output-only revisions do not rebuild selection. Preview is not a visit
for last-tab history. Direct remote clients without a matching API target cannot
preview; public reads cannot atomically bind boot identity across server restart.
See the [feature record](docs/fork/features/goto/README.md).

## Fork infrastructure

Reuse upstream tests, five platform build targets and Windows app-local ConPTY
packaging. CI embeds source/build identity and validates exact run/attempt,
production settings, target and digests before publication. Hash-pinned Zig
dependency prefetch/cache/retries precede compilation. Fork macOS uses Intel
runners with the existing ARM64 cross-build and an adequate cold-check budget.
Official website/issue/release side effects remain canonical-repository-only.

One guarded fork workflow replaces `xbao-latest` after complete checks/builds,
retaining verified last-good assets and exact tag leases for failure recovery.
Replacement is not atomic; consume downloads after successful publication and
verify manifests. mise owns non-Python tools and uv, uv owns Python, rustup owns
Rust. See [workflow and recovery](FORK_WORKFLOW.md) and the
[feature record](docs/fork/features/fork-infra/README.md).

## Candidate verification

Copy's final independent client/runtime set passed 412 tests, fmt/all-target
clippy and six architecture checks; independent exact-source review accepted both
client follow-ups. Earlier API tests and isolated recording PTY/TUI evidence are
described with their limits in its feature record. Goto and infrastructure retain
their earlier native/contract and five-platform delivery evidence. These results
do not substitute for the new aggregate's combined/runtime/platform acceptance.

Combined acceptance and rolling publication are pending for this candidate.
The rolling release's `PUBLISH_INFO.json` and per-platform `BUILD_INFO` files,
rather than a moving branch name, identify the actually published source.
