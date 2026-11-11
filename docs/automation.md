# Agent and automation interface

The same case service handles examiner requests, offline recipes and requests from approved local AI tools. Phase 0 defines behavior. The protocol and any MCP integration are planned.

| Mode | Input | Execution |
| --- | --- | --- |
| Solo | Named examiner selects a case and recipe | Deterministic scan, validation and derivative storage within the case. Sensitive operations still require a second authorized person |
| Autonomous | Examiner approves a versioned recipe and grants a service identity a case-scoped run | The scheduler runs only listed steps, with bounded retries, time, bytes, memory, output and concurrency |
| Custom agent | Local model or external orchestrator with a scoped integration identity | Propose a typed plan or submit allowed jobs inside an active grant. Integration is local in the air gap and uses the approved lab gateway on the isolated network |

## Operations

| Operation | Inputs | Output |
| --- | --- | --- |
| `inspect_case` | Granted case ID, optional cursor | Policy-filtered metadata and opaque object IDs |
| `propose_plan` | Case ID, immutable evidence IDs, recipe ID and typed parameters | Canonical plan digest, required permissions, resource estimate and known limits |
| `start_job` | Plan digest, scoped grant and idempotency key | Job ID and a durable accepted or denied decision |
| `job_status`, `cancel_job` | Job ID and case scope | Bounded state and counters, or an audited cancellation request |
| `list_findings` | Case and job IDs, explicit field projection and page size | Policy-filtered candidate IDs, ranges, confidence and validation reasons |

Operations carry a protocol version and structured result codes. Accept schema-validated data with strict size limits and reject unknown action types, parameter fields and unsupported versions. The gateway resolves paths and storage identities server-side. An integration may not request a shell, raw source handle, arbitrary read or write path, dynamic executable plugin, credential, unrestricted evidence bytes or external URL. The default finding projection contains bounded counts, recovery status, allowed technical metadata and opaque handles. Disclosing text, previews or evidence bytes requires an explicitly approved projection and sink that satisfies the handling policy. Model prompts, completions, embeddings and local inference caches inherit evidence handling labels if they contain or derive from case content.

## Delegation and replay

1. The examiner signs in with MFA and issues a revocable grant to a distinct integration identity. Bind tenant, case, evidence IDs, permitted recipes, quotas, output fields, expiry, policy version and sponsor. Job acceptance and result reads both enforce the grant. Revocation stops queued work and disables future I/O for active work within a tested revocation window.
2. The service compiles a plan with pinned engine, decoder and ruleset versions. Each step specifies read ranges or a bounded selection, format validators, output quotas, and partial-result policy. Deny cyclic plans and unbounded fan-out. Model-generated instructions and parsed evidence text are data. Neither can add tools, alter a grant, interpret policy, or change the approved plan.
3. Store derived candidates and provenance in the same case. Each result includes method version, absolute source extents, completeness, validator status and a reasoned confidence score. Deduplication preserves all observations. An optional model can suggest triage tags stored as separate, unverified annotations.
4. The service logs the grant, plan digest, job state, policy decisions, request ID and actor or sponsor. It records provenance of the local model version and configuration when used. Replay executes the recorded accepted plan without invoking the model. Timestamps, randomized ciphertext, signatures and operator events have separate nondeterministic records. Use deterministic finding manifests as the unit of comparison.

Sensitive operations have a separate human workflow. Autonomous work can continue within approved scope after device acquisition. A person may stop a job or narrow its scope. Resume uses the recorded grant and plan and rechecks current policy, inputs and quota. Models and their adapters never hold approval credentials, operator PINs or evidence vault keys.

## Runtime boundaries

The air-gap interface uses local IPC or stdio. Launch an approved model worker with no inherited secrets, network, filesystem authority beyond its signed model package, or access to unrelated jobs. A third-party agent launched outside that sandbox is a distinct egress risk and receives only the approved projection through the same gateway. A cloud model has no integration path in either government deployment profile. Lab integrations use short-lived authenticated workload credentials on the approved private segment. Both profiles enforce server-side quotas and audit outcomes when clients disconnect.
