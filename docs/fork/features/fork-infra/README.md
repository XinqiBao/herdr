# Fork tooling and guarded rolling delivery

This feature supplies reproducible tool selection, five-platform build provenance
and guarded replacement of the personal fork's `xbao-latest` release. It reuses
upstream build/check recipes and Windows ConPTY packaging. The implementation is
maintained on `xbao/infra`; integration and publication remain owner-controlled.
Common contribution and integration policy belongs in the fork's common docs.

## Tool ownership and dependency acquisition

[`mise.toml`](../../../../mise.toml) pins just, Zig, Bun, CMake, Ninja, nextest and
uv. Rust/Cargo belong to rustup and `rust-toolchain.toml`; uv owns Python through
[`.python-version`](../../../../.python-version). First-party Python helpers use
the standard library, so this feature adds no empty package manifest or dependency
environment. Tool pins do not establish a fully locked bootstrap for every host.

```sh
mise install
mise exec -- uv python install
mise exec -- uv --no-cache run --no-project -- just maintenance-test
```

Keep Cargo targets and mutable Zig build caches independent per worktree/job.
Native checks use ReleaseSafe without SIMD; production builds explicitly select
ReleaseFast with SIMD. The fork-only
[`fetch-zig-deps` action](../../../../.github/actions/fetch-zig-deps/action.yml)
resolves the configured VT dependency graph before compilation for checks,
Windows ConPTY packaging and all five production targets. Zig 0.16's
`build --help` discovers required lazy packages using the same target/options;
`--fetch=needed` alone does not discover that complete graph. Dependency hashes
and versions remain unchanged.

Acquisition has at most three attempts within a five-minute workflow-step budget
and fails closed. These retries do not retry product compilation. Only
`vendor/libghostty-vt/zig-pkg` package sources are cached, with host, target,
configuration and dependency inputs in the key; graph resolution still runs on
cache hits. Persistent DNS/download failures remain job failures.

Fork macOS checks and production builds select the supported `macos-15-intel`
runner to avoid ARM runner capacity shortages. Checks and x86_64 builds are native;
ARM64 uses the existing Rust/Zig cross-build and Apple SDK. Canonical upstream
runner choices are preserved. Fork macOS checks have a 30-minute budget; other
check matrix rows retain 15 minutes. A cross-build success does not establish
native ARM64 runtime behavior.

## Publication gates and provenance

[`fork-release.yml`](../../../../.github/workflows/fork-release.yml) accepts an
owner dispatch only in `XinqiBao/herdr` on `xbao/main`. Both the original actor
and rerun actor must be repository admins. Repository concurrency serializes
publisher runs. Publication requires successful checks and the complete build
matrix; failed or skipped jobs cannot publish. Official stable/preview channels,
website deployment and issue mutation remain canonical-repository-only. This
feature does not install a binary or alter a running Herdr session.

[`fork-base.json`](../../../../fork-base.json) records the canonical repository,
official tag and full commit SHA. Preparation verifies the canonical tag peel
and requires the base to be an ancestor of the selected source. Upgrade the
manifest deliberately when changing official baselines.

[`fork_release.py`](../../../../scripts/fork_release.py) requires these assets:

- `herdr-linux-x86_64` and `herdr-linux-aarch64`
- `herdr-macos-x86_64` and `herdr-macos-aarch64`
- `herdr-windows-x86_64.zip`, including `herdr.exe` and app-local ConPTY runtime

All platforms must match the exact source SHA, Actions run/attempt, target and
production settings. Build identity, SHA-256 and archive contents are validated
before remote mutation. Five `BUILD_INFO` files, `PUBLISH_INFO.json` and
`SHA256SUMS` accompany the five downloads. Mixed-attempt artifacts are rejected;
a partial rerun may therefore require rerunning the complete workflow.

## Replacement, recovery and limits

[`fork_publish.py`](../../../../scripts/fork_publish.py) snapshots the previous
tag object/commit, release metadata, latest-release identity and all twelve
assets, checking API/download digests and the expected inventory. Orphan tags,
unexpected drafts, immutable releases and incompatible inventories stop before
replacement. A verified last-good backup must upload successfully as the Actions
artifact `xbao-last-good-<run>-<attempt>` before mutation; retention is 30 days.

Each attempt creates a unique annotated tag carrying its run/attempt. Exact raw
object-SHA leases protect tag updates and rollback, distinguishing even attempts
publishing the same source SHA. Forward ownership checks cover tag and release
identity; recovery additionally rejects unrecognized metadata or asset bytes.
After creating a new draft, a bounded five-read visibility wait preserves its
returned ID and checks ownership/latest state; exhaustion fails closed.

Ordinary upload, verification and metadata-edit failures attempt to restore the
known previous assets/labels, writable metadata, latest pointer and tag. Applied
writes with lost responses are inspected before recovery proceeds. A successful
restore still leaves the workflow failed. Unknown state or newer ownership stops
automatic recovery and requires operator review of the backup and journal.

Replacement is **not atomic**: public assets can be mixed or absent temporarily.
GitHub provides no transactional/CAS replacement for assets or release metadata;
external/manual writers must be quiesced. Cancellation, runner loss, persistent
network failure or failed recovery can leave partial state. Asset IDs/timestamps
cannot be restored. Retain backups separately if recovery must outlive artifact
retention. Native manual recovery steps live in
[`FORK_WORKFLOW.md`](../../../../FORK_WORKFLOW.md); inspect current ownership
before restoring anything and stop on a rejected exact tag lease.

## Validation evidence and remaining boundaries

The delivered implementation passed 26 fork Python helper tests, 150 existing
maintenance tests and 12 workflow tests, plus actionlint on the affected
workflows. The helpers exercise a fake command/API backend without remote
mutation: ordinary failures, applied-write/lost-response outcomes, first
publication, newer same-source publishers, unknown edits and failed recovery.
Canonical-base tests cover annotated/lightweight tag peel, wrong SHA and
non-ancestor rejection. Workflow contracts cover failed/skipped prerequisite
jobs and backup-upload gating. Fetch-action behavioral tests exercise transient
and permanent failures, exact configuration flags and CRLF handling; their fake
executable cases are skipped on Windows, where workflow contracts still run.

The owner reported successful publication and independent verification of
[run 43016889389](https://github.com/XinqiBao/herdr/actions/runs/43016889389),
following successful five-platform builds/checks. That is live delivery evidence,
not a guarantee against future runner shortages, dependency outages or arbitrary
interruption. Local fake-backend tests cannot establish every GitHub consistency
or failure outcome; native runtime qualification remains separate from builds
and archive/provenance verification.
