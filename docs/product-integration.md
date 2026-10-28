# Product integration contract

AX is the product-neutral execution authority for isolated agent workloads.
Products own admission, identity, durable domain data, prompts, model budget,
and lifecycle decisions such as saving, publishing, or closing a coaching
session. AX owns declarative task intent, actor reconciliation, workspace
attachment, routing, suspension, resumption, and cleanup.

The versioned public contract is `ax.io/v1alpha1`, defined in
[`pkg/apis/v1alpha1/ax.proto`](../pkg/apis/v1alpha1/ax.proto). The Sites and
Coach fixtures under [`examples/consumers`](../examples/consumers/) are
machine-decoded and validated by the normal AX types in CI.

## Task intent

A product creates a `Task` only after its own admission succeeds. The task must
declare:

- an immutable workload image selected by digest;
- a bounded command and resource request/limit;
- named `Workspace` references and explicit mount paths;
- opaque product session authority as environment values or mounted
  capabilities; and
- `debug: false` unless a separately authorized diagnostic explicitly needs
  guest process and file access.

Environment values are construction inputs. A product must never put provider
credentials, pricing tables, payment credentials, or a commercial ledger in a
task. Model calls use the separately authorized model-gateway capability and
private endpoint supplied by the product.

## Session authority

AX does not create product sessions or decide whether they remain authorized.
The product owns a stable session identity and a changing incarnation fence,
such as a lease identifier, generation, and fencing token. The workload uses
those values when calling the product or model gateway. A stale incarnation
must fail closed at those authorities even if its actor is still reachable.

The product records the AX atespace, task name, and observed actor identity with
its session. `status.phase: Running` and an unchanged actor name do not prove a
template-affecting update reached the live process. For image, command,
environment, workspace, mount, or capability changes, the consumer must prove a
new or matching actor incarnation and a runtime-visible marker. If the deployed
AX version cannot prove template reconciliation, delete the task, wait for
`NotFound`, recreate it, and wait for readiness.

## Image and workspace boundary

The image contains generic agent runtime machinery. Sites-specific publishing,
preview, Save/Release behavior, and prompts remain supplied by Sites. Coaching
record policy, prompts, and durable coaching state remain supplied by Coach.
AX may mount durable workspaces and coordinate checkpoint/resume, but the
product remains authoritative for its database and retained domain records.

The image reference must be a registry digest. A mutable tag does not provide
sufficient source-to-runtime evidence. Release evidence records the AX source
commit, workload-image digest, Task intent, actor incarnation, worker
assignment, readiness result, routed behavior, cleanup result, and recovery
selector.

## Evolution policy

`v1alpha1` changes remain additive where possible. Existing field numbers and
wire meanings are preserved; removed protobuf fields are reserved. A breaking
change requires a new API version, documented consumer migration, and
compatibility tests for both Sites and Coach. Product-specific fields do not
belong in the AX schema: pass opaque capabilities or add a reusable generic
primitive instead.

## Failure and retry semantics

- Product admission failure creates no Task.
- Task creation uses a stable product-derived name so uncertainty is resolved
  by `GetTask`, not by creating a second task.
- A conflicting existing Task fails closed until the product proves that its
  intent and incarnation match.
- Deletes are observed through `Terminating` until `NotFound`; recreation does
  not race cleanup.
- Route readiness is distinct from application readiness.
- Suspension/checkpoint preserves declared workspace state, not process memory,
  credentials, or product database ownership.
- Cleanup failure is observable and retried idempotently; it never reauthorizes
  a closed product session.
