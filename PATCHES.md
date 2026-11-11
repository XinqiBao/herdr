# Fork infrastructure

Base: official Herdr v0.9.5 (`2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`).
This independent branch contains only fork tooling. See [FORK_WORKFLOW.md](FORK_WORKFLOW.md).
Workers retain coherent incremental commits on their assigned feature branches;
the coordinator normally squashes a feature for linear integration, with a short
meaningful sequence allowed when useful. See the [feature index](docs/fork/README.md)
and [infrastructure record](docs/fork/features/fork-infra/README.md) for durable
intent, compatibility and validation rationale.

## Builds and publishing

- Run upstream CI on xbao branches and make CI/artifact workflows reusable.
- Keep upstream Rust, Zig, platform build and Windows ConPTY packaging machinery.
- Mark fork binaries with the base version and exact source identity.
- Publish one rolling xbao-latest release from xbao/main, only after all checks
  and all five platform builds succeed; validate artifact provenance/checksums.
- Keep official publishing workflows and distribution channels unchanged.
- Guard inherited website/issue side effects on the canonical repository.
- Back up the last-good rolling delivery before replacement; use exact tag leases
  and conservative recovery on ordinary failures. Replacement is not atomic.
- Pin non-Python tools with mise and Python with uv; keep rustup as Rust authority.
- Remove these adjustments when upstream provides equivalent fork support.
