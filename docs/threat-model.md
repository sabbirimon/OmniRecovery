# STRIDE threat model

## Assets and trust boundaries

Assets include original media, image extents, source metadata, derived findings, case authority records, retention holds, approval assertions, trust roots, encryption keys, audit history and release artifacts. Assume devices, evidence bytes, metadata, user-supplied archives, plugins, models, generated plans, UI requests, import media, and backup media can be malicious or compromised.

| Boundary | Crossing | Required control |
| --- | --- | --- |
| B1 | Human or agent to case service | MFA or delegated workload identity, case grant and replay checks |
| B2 | Untrusted devices to acquisition helper | Qualified hardware path, read-only handle and command allowlist |
| B3 | Vault readers to parser, decoder, UI or local model | Narrow bytes or projected data, sandbox, no egress and quotas |
| B4 | Case service to storage, crypto provider, witness and backup | Encryption, distinct tenant credentials, signed checkpoints, restore ceremony |
| B5 | Approved export or maintenance to external media | Exact-intent approval, registered route, classification and durable receipt |
| B6 | Builder, signer and offline media to deployment | Verified package, provenance, policy and anti-rollback controls |

## Threats and evidence of mitigation

| ID | STRIDE | Scenario | Mitigation and verification |
| --- | --- | --- | --- |
| T01 | Spoofing | Stolen session, duplicate identity, replayed approval or false service principal | Hardware MFA, named enrollment, workload credentials, one-use assertions and exact scope. Test same-person aliases, revoked tokens, tenant swaps, replay and stale revocation (V05) |
| T02 | Tampering | Source is altered by automount, helper commands, wrong-disk writes or hot-swap | Disable automount, qualify blocker, validate backing storage, pin handle and use allowlisted commands. Verify pre/post hashes and observed writes, including mounted and reconnected devices (V01–V02) |
| T03 | Tampering | Corrupt image, reorder GCM chunks, swap files across cases, reuse nonce on retry or truncate acquisition | AEAD metadata binding, unique keys on resumed segments, signed range manifest and independent verification. Inject mutations and crash before and after each commit boundary (V03–V04) |
| T04 | Repudiation | Insider rolls back the store, forks the ledger, changes signer or denies release | Versioned signatures, actor and approver assertions, external witness and destination receipts. Restore a stale snapshot, simulate competing heads and verify historical signer state (V04–V05) |
| T05 | Information disclosure | Agent is prompted by evidence, hostile parser exfiltrates, logs leak credentials, cross-tenant object lookups reveal data | Typed job boundary, trusted projection, process isolation, no network, opaque IDs, key separation and redacted logging. Seed canaries in evidence and credentials. Probe sockets, files, logs, dumps and cross-tenant timing or cache paths (V06–V08) |
| T06 | Denial of service | Decompression bomb, pathological parser, media timeouts, full destination, unavailable HSM or witness | Bounded workers, per-tenant admission, reserved audit capacity, safe cancellation and readable degraded state. Exercise disk-full, slow devices, invalid formats, quota races and dependency loss (V03–V04, V07) |
| T07 | Elevation of privilege | Device helper or native parser escape, policy bypass through alternate client, malicious dependency | Short-lived helper, strict IPC, nonprivileged sandbox, signed packages and independently reviewed security boundaries. Attempt alternate entry points, exploit fuzz findings and test post-install sandbox restrictions (V05–V08, V11) |
| T08 | Tampering / information disclosure | Malicious update or trust bundle installs unsigned code, weakens policy or downgrades crypto | Detached trust roots, signed and versioned metadata, pinned operating mode and offline verification. Test revoked keys, stale release, wrong profile, missing provenance and fallback (V08, V11) |
| T09 | Tampering / repudiation | Legal hold races with deletion or stale restored backups resurrect or lose evidence | Serialized retention enforcement, held-key checks, restore reconciliation and independent disposition evidence. Race deletion against hold and verify every replica and escrow state (V09) |
| T10 | Information disclosure / tampering | Release recipient, removable disk or package changes after approval | Bind export package digest, destination and recipient to the signed operation. Stage ciphertext, validate the pinned destination at publication and record uncertain deliveries. Test symlink and hardware swaps, duplicate retries, incompatible labels and package tampering (V05, V09) |
| T11 | Tampering | Malicious media induces a false positive that is presented as a recovered original | Validate structure and decode in isolation, preserve byte provenance, distinguish repairs and expose reason codes. Test known-answer and adversarial corpora (V10) |

## Residual risk and scope

- A compromised kernel, hypervisor, device firmware or signer can subvert product controls. Qualification includes boot and firmware trust, physical handling and approved media. Network tests cover the OS profile and the product's descendants.
- Two colluding qualified operators can authorize an in-scope action. Independent audit, limited duties and access reviews remain organizational controls.
- A checksum cannot establish when bytes changed before acquisition. Source provenance, capture method and examiner observations remain essential.
- Offline deployments have bounded time and revocation knowledge. Display the source and freshness of time, revocation and checkpoints. Loss of the necessary trust evidence puts sensitive operations into a degraded state.
- Host administrators and authorized recipients can capture plaintext once it is displayed. Deployment clearance, trusted endpoints and physical controls govern those risks. Log and constrain every available export route.
- SSD controllers, backups and independently held copies can defeat deletion claims. Retain device-specific disposition and verification evidence, including incomplete outcomes.

The [validation plan](validation.md) defines the linked test IDs. Independent review is required before changing trust boundaries or releasing a high-assurance profile.
