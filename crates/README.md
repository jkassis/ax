# AX Rust crates

These crates are the Rust client boundary for AX-owned task/session integration.
They were imported from `jkassis/sites` commit
`c5c96bff86af27ad15afea39040c5bb902a8074d` during repository separation.

- `agent-ax-client` owns the typed AX gRPC client and uses the canonical AX
  schema copied from `pkg/apis/v1alpha1/ax.proto` at build time.
- `agent-session-contract` owns the product-neutral session specification and
  receipt types shared by AX clients.

Run `cargo test --workspace` from the repository root. Keep the Rust schema copy
byte-for-byte aligned with the canonical Go schema; CI verifies this invariant.
