# Agent AX Client

This crate applies a validated product-neutral agent session to AX and returns
the native actor locator. Products retain admission, authorization, durable
intent, and lifecycle ownership; this package owns only the AX wire transport.

It accepts the shared `SessionSpec`, creates a matching AX gateway and task,
waits for the actor to become running under a hard deadline, and removes both
objects idempotently when the owning product stops the session.

Run `cargo test --manifest-path platform/agent/ax-client/Cargo.toml`.

## Source authority

AX is authoritative for this crate. The protobuf input mirrors
`pkg/apis/v1alpha1/ax.proto`; update both together.
