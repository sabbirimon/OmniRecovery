# Roadmap and build plan

Each phase ends with an artifact inventory, actual test results, known limitations and open items for the next phase. A feature becomes supported only when its documented acceptance cases pass for the intended deployment.

## Repository layout

The repository contains Phase 0 design artifacts, a Rust CLI and library in `src/`, and synthetic integration tests. The following workspace layout will grow with the phases.

| Area | Planned ownership |
| --- | --- |
| `crates/` | Source I/O, evidence formats and vault, independent verification, custody, policy, recovery and worker supervision |
| `apps/` | CLI, desktop UI and separately packaged lab services and mobile companions |
| `adapters/` | Privileged OS acquisition, authorized mobile transports, format bridges and later WASM extensions |
| `tests/` and `fuzz/` | Generated test data, controlled interoperability, device and security test runners, parser fuzz targets |
| `packaging/` | OS build and installer recipes, offline update tooling, signed Linux field-kit images, trust and verification tooling |
| `docs/` | Design, validation, examiner and developer guides, control evidence and operations manuals |

## Gates

| Phase | Work | Exit evidence |
| --- | --- | --- |
| 0 | Requirements, ADRs, threat model, capability matrix, build/CI design, assurance mapping and validation plan | Review target profile, license, test hardware and certification goals. Resolve the identity and storage risks that affect Phase 1 |
| 1 | Read-only I/O, source and destination identity, raw and E01 imaging, vault, verification, signed custody and CLI. Establish identity and approval enforcement before live acquisition | Synthetic and known-answer image suite, crash-injection and restore proof, blocker evidence on supported hardware, independent E01 round trips and an attributable case workflow |
| 2 | FAT/exFAT/NTFS/ext-family recovery and JPEG/PNG/MP4/MP3 carving with structural validation | Filesystem and format fixtures, false-positive measurements and attributable partial results |
| 3 | Guided and expert desktop modes, isolated previews, case search, reports and examiner workflows | End-to-end case replay, approved release packages and accessibility tests. JSON/CSV/HTML/PDF outputs with verifiable manifests. Assess CASE/UCO, VERIS and tool interoperability against actual consumer formats |
| 4 | Android USB acquisition, SQLite/WAL artifacts and F2FS analysis | Versioned device/app matrix with observed side effects, credential-gated access and known-answer comparisons |
| 5 | APFS/HFS+, iOS logical backup and authorized encrypted-volume adapters | Cross-tool and hardware evidence, password handling review and compatibility matrix |
| 6 | Fragment reconstruction, reference-assisted media repair, timeline, hashing and optional local model triage | Provenance and replay tests. Measure recall and false positives by scenario and isolate model/decoder inputs |
| 7 | Companion apps, WASM SDK and optional GPU processing | Validate mobile permission boundaries, portable plugin sandbox, and CPU/GPU output equivalence or explicit differences |
| 8 | Hardening, lab and field-kit qualification, independent validation and release packaging | Signed artifacts and reproducibility evidence, offline upgrade/recovery exercise, penetration-test remediation and configured-profile documentation |

### Android-first release path

1. Start with synthetic immutable images and logical exports. Prove read-only raw and Android sparse inspection, candidate validation and pre/post comparisons. Pin the Rust toolchain and dependencies. Add signed manifest test vectors with the vault work.
2. Build an encrypted chunk store and transactional ledger. Test truncation, disk-full, kill-and-resume, orphan recovery, checkpoint loss and restore before accepting any real case.
3. Introduce named identity, authorization records, explicit storage registration and two-person gates. Validate hardware MFA, scope binding and policy enforcement on the intended first profile. Prove the original device and destination stay distinct.
4. Qualify raw-device and EWF intake, then release authorized Android USB collection and accessible media, backup and SQLite artifacts before expanding device targets. Use consented lab hardware and known-answer pre/post data. Preserve completeness and observed source mutation disclosures. A solo development setup can exercise synthetic gates. Live high-assurance qualification requires two operators and the applicable hardware.

### CI and distribution design

- The local Git repository includes a GitHub Actions workflow for formatting, lints and synthetic tests on Linux, macOS and Windows. No remote hosting is configured. Add ARM64 and host-specific device jobs after the corresponding runner and hardware are available.
- Later presubmit checks add license and dependency policy and bounded fuzzing. Nightly jobs expand corpus coverage and use dedicated hardware for device tests. Release jobs are independently authorized and verify signing, reproducibility, SBOM, provenance and hardened-profile checks.
- Candidate packages include signed Windows installers, macOS signed/notarized desktop distributions, Linux AppImage/deb/rpm/Flatpak subject to adapter limitations, an offline CLI bundle and a signed field-kit boot image. A sandboxed package may require an external qualified acquisition helper. Evaluate supported platform versions and package signing infrastructure before advertising distribution targets.
