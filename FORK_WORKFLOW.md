# Fork tooling and rolling delivery

This tooling targets `XinqiBao/herdr`; it grants no upstream publishing authority.
Feature work stays on its own branch. The owner integrates reviewed deliveries
into linear `xbao/main`, retaining source branches/worktrees and provenance. Push
retained feature branches to the personal fork for history; publish binaries only
from `xbao/main`. Never run official release recipes, change official distribution
channels, or auto-install a fork build.

## Team ownership and feature records

Each feature worker owns implementation, tests, coherent incremental Conventional
Commits and ordinary pushes to its assigned origin branch. Routine in-scope
commits do not require individual subject approval. Escalate material design,
dependency or scope changes to the coordinator; do not mutate other tasks,
mainline, tags or releases. Rewriting published history requires coordinated
authorization and an exact observed-SHA lease.

The coordinator owns direction, dependencies, independent quality review,
acceptance, integration and publication. Keep mainline linear: one squash delivery
per feature is the default; a short meaningful sequence is appropriate when it
improves review, maintenance or reverts. Preserve source history and record exact
branch, source SHA/range, upstream base, dependencies and integration resolutions.
Apply only the delta since the recorded source for subsequent same-base updates.

Workers maintain useful tracked documentation in `docs/fork/features/<feature>/`
with their code: intent, accepted design and rationale, interfaces/compatibility,
base/dependencies, relevant verification evidence, limitations and upgrade notes.
The coordinator alone curates the common [feature index](docs/fork/README.md),
`PATCHES.md` and this workflow. Keep private host/session information, raw chat,
temporary logs and current next-action state local. Do not publish `.context`
wholesale or rely on ignored notes as the sole feature specification.

Use flat sibling worktrees, one branch per independent feature, each based on an
explicit official stable tag. Declare dependencies; do not base independent work
on a moving aggregate. Each worktree owns a real, independent `.context/`, never
a symlink or automatically synchronized directory. On a future stable upgrade,
the owner first quiesces workers and preserves source SHAs/uncommitted work, then
rebases feature deltas with `git rebase --onto NEW_TAG OLD_TAG`, reconstructs the
linear aggregation from that same tag, and revalidates each feature and combined
product. Preserve branches/worktrees; rewritten fork pushes use exact SHA leases.

## Local tools

`mise.toml` pins just, Zig, Bun, CMake, Ninja, nextest and uv. Rust/Cargo remain
owned by rustup and `rust-toolchain.toml`; uv owns Python via `.python-version`.
First-party Python scripts use only the standard library, so no package manifest,
dependency lock or project virtual environment is required.

```sh
mise install
mise exec -- uv python install
mise exec -- uv --no-cache run --no-project -- just maintenance-test
mise exec -- uv --no-cache run --no-project -- just ci 'not binary(live_handoff)'
```

The filter matches upstream macOS CI; Linux uses all tests. Keep mutable Cargo
targets and Zig build caches per worktree. Native checks default to
ReleaseSafe/no-SIMD; production builds explicitly use ReleaseFast/SIMD. Pinned
versions alone are not a claim of a fully locked bootstrap on every platform.

## Publication contract

An owner dispatches `Fork rolling release` on `xbao/main` only. Both original and
rerun actors must be fork admins. The workflow reuses upstream Linux/macOS/Windows
checks, all five production targets and the app-local Windows ConPTY packager.
Official release/preview, website and issue mutation stay canonical-repo-only.

For the personal fork, macOS jobs use the official `macos-15-intel` runner as a
capacity fallback: checks and x86_64 builds run natively, while ARM64 uses the
existing Rust/Zig cross-build target and Apple SDK. Upstream runner selection is
unchanged. Final CI must validate the ARM64 cross-build; source support and local
workflow checks alone do not establish a successful live build or ARM64 runtime test.
The fork's macOS check job has a 30-minute budget for Intel cold compilation;
other check matrix rows retain the upstream 15-minute budget. Individual test
limits are unchanged.

Fork checks and production builds prefetch libghostty-vt packages with Zig 0.16
using the same target and build options as Cargo. `zig build --help` resolves the
lazy dependency graph without building artifacts. The fetch step has up to three
attempts within five minutes; an exhausted fetch stops the job. Acquisition
retries do not retry product compilation. Only `zig-pkg/` package sources are saved by the fetch action, under keys
that include host, target, build settings and dependency inputs. Zig's pinned
dependency hashes and per-job build directories remain in use.

`fork-base.json` declares the canonical repository, stable tag and full base SHA.
On a baseline upgrade, update this manifest deliberately. Preparation compares
its SHA with the tag's canonical upstream peel and requires it to be an ancestor
of the checked-out source; it never infers the base from arbitrary fork tags.

Every platform must have the same exact source, Actions run/attempt and production
settings. SHA-256, target, fork build ID and ConPTY contents are checked before
mutation. Published `BUILD_INFO` files and `PUBLISH_INFO.json` record provenance,
including the official base SHA; `SHA256SUMS` covers the five downloads. Re-run all
jobs if a partial rerun leaves artifacts from mixed attempts; these are rejected.

## Replacement and failure recovery

Repository-level concurrency serializes this publisher. Before mutation it
captures the previous tag object/commit, release metadata, latest-release ID and
all 12 assets with verified API/download digests. Existing orphan tags, drafts,
immutable releases, incomplete or unexpected asset inventories require manual
reconciliation. The last-good backup must upload successfully as Actions artifact
`xbao-last-good-<run>-<attempt>` (30-day retention) before publication can begin.

Each attempt uses a unique annotated tag object containing its run/attempt. Tag
update and rollback use exact object-SHA leases, including an empty lease when
creating the first tag. This distinguishes publishers of the same source commit.
The helper rechecks tag ownership and release identity before mutations. Recovery
also rejects unrecognized metadata or asset bytes. Ordinary upload, verification
or metadata-edit errors attempt to
restore the saved asset bytes/labels, metadata, latest pointer and finally tag.
An applied write whose response was lost is inspected before choosing recovery.
The workflow still fails after a successful restore; its journal records the result.

**Replacement is not atomic.** Public assets can be temporarily mixed or absent;
download only after a successful publisher run. GitHub provides no transaction or
CAS for release assets/metadata. Quiesce all external/manual tag and release edits
during publication/recovery: a check-to-write race cannot be eliminated. Unknown
metadata/bytes, changed identity or a newer tag stops automatic recovery. Runner
cancellation/termination, network loss, immutable-release changes or recovery
errors can leave partial state. Asset IDs/timestamps cannot be restored, only
the saved content/labels and writable metadata. Backup upload is durable only for
its retention period; retain it separately if longer recovery is needed.

## Native manual recovery

Stop publishers and manual writers first. Inspect the failed run and optional
`xbao-recovery-journal-<run>-<attempt>` artifact. Download the **pre-mutation** backup:

```sh
gh run download RUN_ID --repo XinqiBao/herdr \
  --name xbao-last-good-RUN_ID-ATTEMPT --dir recovery
git ls-remote origin refs/tags/xbao-latest 'refs/tags/xbao-latest^{}'
gh api repos/XinqiBao/herdr/releases --paginate
cat recovery/state.json
```

Compare current tag object, annotated run/attempt, release ID, metadata and asset
digests with the backup and journal. Do not overwrite a newer/unrecognized state.
The journal may be absent after cancellation; inspect the annotated tag directly
with `git fetch --no-tags origin refs/tags/xbao-latest` and
`git cat-file -p FETCH_HEAD`. Record the exact observed object as `OWNED_OBJECT`.

For an existing previous release, verify each backup file against
`previous_digests` in `state.json`, then restore with native commands:

```sh
gh release upload xbao-latest recovery/assets/* --clobber --repo XinqiBao/herdr
jq '.previous.release | {tag_name,target_commitish,name,body,draft,prerelease}' \
  recovery/state.json > recovery/metadata.json
gh api repos/XinqiBao/herdr/releases/PREVIOUS_RELEASE_ID \
  --method PATCH --input recovery/metadata.json
```

Restore any saved asset labels using `gh release upload 'path#saved label'`
with `--clobber`, and restore the saved latest pointer via
`gh api repos/XinqiBao/herdr/releases/PREVIOUS_LATEST_ID --method PATCH -f make_latest=true`.
If the rolling release was not previously latest, mark it with
`gh release edit xbao-latest --latest=false --repo XinqiBao/herdr` first. If no
previous release existed, delete only the failed run's identified new release
using `gh api repos/XinqiBao/herdr/releases/FAILED_RELEASE_ID --method DELETE`.
Reverify asset digests, labels and metadata before restoring the tag.

An old annotated object can be reinserted locally with
`git hash-object -t tag -w recovery/previous-tag.txt`; verify the returned hash
against `previous.tag.object`. Its target commit must remain available locally.
For a previous tag, use the saved raw object SHA, not its peeled commit:

```sh
git push --force-with-lease="refs/tags/xbao-latest:OWNED_OBJECT" \
  origin PREVIOUS_TAG_OBJECT:refs/tags/xbao-latest
```

If the tag was previously absent, use the same exact lease with
`origin :refs/tags/xbao-latest` instead. A rejected lease is a stop condition.
Re-read tag, release and latest state after recovery; do not claim success from
an interrupted command's exit status alone.
