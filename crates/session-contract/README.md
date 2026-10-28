# Agent Session Contract

This crate is the product-neutral contract between an admitting product and the
ShineTribe agent runtime boundary. Products authorize their own opaque resource
references and select an immutable runtime, attachments, and named routes. The
platform owns session identity, canonical room identity, leases, generations,
fencing, lifecycle, and route publication.

The crate must not depend on `apps/sites` or contain product database access.
Contract evolution is additive within a schema version; a changed accepted spec
creates a new fenced generation rather than mutating a running generation.

Run `cargo test --manifest-path platform/agent/session-contract/Cargo.toml`.
