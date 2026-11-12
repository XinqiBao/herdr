# Fork feature records

This directory records the independently maintained patches for XinqiBao's fork.
The integration checkout's root `PATCHES.md` identifies the exact retained source
SHAs and official stable base; `FORK_WORKFLOW.md` defines ownership and delivery.

- [Copy mode](features/copy-mode/README.md): viewport navigation, automatic mouse
  history capture and guarded application input ownership.
- [Goto preview](features/goto/README.md): read-only terminal preview before
  selecting a pane.
- [Tab navigation](features/tab-navigation/README.md): last-visited history,
  persistent numeric slots and [validation evidence](features/tab-navigation/validation.md).
- [Fork infrastructure](features/fork-infra/README.md): tools, CI and guarded
  rolling publication.

Each worker owns `features/<feature>/` and updates useful intent, design rationale,
compatibility, verification and upgrade limits alongside code. The coordinator
curates this small common index and the aggregate ledger. Private host/session
details and transient work notes remain local. A feature record describes the
current delivery; Git history preserves earlier revisions.
