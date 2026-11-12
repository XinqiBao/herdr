# Fork feature records

This directory records the independently maintained patches for XinqiBao's fork.
The integration checkout's root `PATCHES.md` identifies the exact retained source
SHAs and official stable base; `FORK_WORKFLOW.md` defines ownership and delivery.

- [Fork infrastructure](features/fork-infra/README.md): tools, CI and guarded
  rolling publication.

Each worker owns `features/<feature>/` and updates useful intent, design rationale,
compatibility, verification and upgrade limits alongside code. The coordinator
curates this small common index and the aggregate ledger. Private host/session
details and transient work notes remain local. A feature record describes the
current delivery; Git history preserves earlier revisions.
