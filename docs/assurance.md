# Assurance and compliance traceability

This is a design-level mapping. Every implementation item below is planned. Test IDs refer to the [validation plan](validation.md). Expand the matrix into implementation references, review evidence, responsible owners and selected clauses as each target is agreed. Verify editions and applicability against the controlling contract, licensed standards and lab procedures.

| Source or objective | Proposed control and owner | Acceptance evidence |
| --- | --- | --- |
| ISO/IEC 27037, NIST SP 800-86, SWGDE computer acquisitions | Acquisition broker and case service preserve authorization, method, source state, identity, hashes and custody | V01–V05, approved acquisition SOP, write-blocker qualification and example custody record |
| ISO/IEC 27041, 27042, 27043 | Recovery and reporting preserve traceable methods, versions, limitations, repeatable results and validation evidence | V04, V10, V12, known-answer results and independent method review |
| ISO/IEC 27050 | Case and retention services preserve identification, handling, disposition and scoped export | V05, V06, V09, release receipts and hold exercises |
| ISO/IEC 17025 | Lab quality lead qualifies exact workflow, examiner competence, equipment, repeatability and uncertainty | V01, V02, V10, V12, calibrated test environment, SOP, validation and corrective-action records |
| ISO/IEC 27001, 27034 and 25010 | Security and engineering leads own risk treatment, secure development and product quality criteria | Threat model, requirements-to-test history, vulnerability response and V07, V08, V11–V12 |
| NIST SP 800-101 Rev. 1, SWGDE mobile, ENFSI and applicable UK NPCC/College of Policing guidance | Mobile and lab leads qualify authorization, minimization, collection procedure and observed changes | V01, V02, V10, V12, device/OS build matrix and documented mobile workflows |
| NIST SP 800-53 Rev. 5 AC-3, AC-5, AC-6, IA-2 | Case service enforces access, least privilege, separation of duties and hardware MFA | V05–V07, role tests, signed approvals and enrollment review |
| NIST SP 800-53 Rev. 5 AU-2, AU-3, AU-8, AU-9, AU-10, AU-12 | Ledger and witness enforce attributable audit records, timing, verification and rollback detection | V04–V05, V08, checkpoint receipts, audit schema and signer-rotation exercise |
| NIST SP 800-53 Rev. 5 SC-8, SC-12, SC-13, SC-28, SI-7 | Provider and deployment leads enforce approved cryptography, key management, encrypted transit/storage and integrity | V03–V04, V08, module certificate and environment record, key and transport tests |
| NIST SP 800-53 Rev. 5 CM-2, CM-6, SA-10, SR-4, SR-11 | Release and operations leads maintain hardened baselines, reviewed changes, provenance and component checks | V08, V11, build and provisioning evidence |
| NIST SP 800-171 Rev. 3 | Deployment owner maps applicable CUI requirements for access, identification, audit, media, system integrity and supply chain to product and inherited controls | System security plan, V04–V09, V11, scoped assessment and deployment evidence |
| NIST SP 800-88 Rev. 2 | Retention and sanitization owner selects an approved device-specific method and retains independent verification evidence | V09, approved disposition program, device method and completion/failure record |
| GDPR privacy by design, where applicable | Data owner configures lawful scope, minimization, purpose, access, retention and security with jurisdiction-specific review | V05–V06, V09, processor and retention records, documented handling decisions |
| FIPS 140-3 CMVP | Provider owner pins exact certificate, version, operating environment, configuration and approved services | V08, provider startup and fail-closed tests, certificate/security policy and provisioning checklist |
| Common Criteria CC:2022 | Assurance lead defines the target of evaluation, operational assumptions, security target and intended assurance components | Boundary and interface descriptions, development and lifecycle records, independent test artifacts and vulnerability analysis, V04–V08 and V11 |
| NIST CFTT and CFReDS | Validation lead uses known-answer imaging, recovery and interoperability fixtures | V01–V03, V10, V12, fixture manifests and repeatable results |

[ISO/IEC security catalogue](https://www.iso.org/committee/45306/x/catalogue/), [NIST SP 800-86](https://csrc.nist.gov/pubs/sp/800/86/final), [NIST SP 800-101 Rev. 1](https://csrc.nist.gov/pubs/sp/800/101/r1/final), [NIST SP 800-53 Rev. 5](https://csrc.nist.gov/pubs/sp/800/53/r5/upd1/final), [NIST SP 800-171 Rev. 3](https://csrc.nist.gov/pubs/sp/800/171/r3/final), [NIST SP 800-88 Rev. 2](https://csrc.nist.gov/pubs/sp/800/88/r2/final), [ENFSI digital technology manual](https://enfsi.eu/wp-content/uploads/2016/09/1._forensic_examination_of_digital_technology_0.pdf), [College of Policing forensic practice](https://www.college.police.uk/app/investigation/forensics), [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679), [Common Criteria CC:2022](https://www.commoncriteriaportal.org/cc/index.cfm)

## Build and release evidence

- Pin compiler, linker, SDK, dependencies, base images and CI actions. Vendor approved dependencies with licenses for an offline build. Review incoming dependencies and advisories in a connected build environment, export signed material and verify it before import. Record patches and exception expiry.
- Require reviewed changes, protected branches, signed release commits and an auditable signing ceremony. Build in isolated ephemeral environments. Produce a CycloneDX SBOM including native, transitive and runtime components. Publish signed artifact digests, provenance, vulnerability and license disposition, and verification instructions. Target SLSA Build Level 3 with builder-generated, protected provenance and verified builder identity. Evaluate reproducibility independently, including comparing unsigned deterministic payloads when platform signing adds timestamps.
- Package CLI, OS installers and offline update media from reviewed source. Verify certificates, required OS hardening and installer behavior. Record exact STIG/CIS revisions, permitted deviations, trust roots, dependencies and OS applicability for each released profile.
- Make independent penetration testing and documented remediation a gate for the high-assurance distribution. Include policy bypass, update paths, key custody, tenant isolation, rollback and all interfaces carrying untrusted data. Repeat after material trust-boundary changes.
- Keep signed test vectors, failure traces, dependency snapshots, tool versions, vulnerability evidence, third-party assessments and the configured operations manuals with each release record. Capture residual risks and formal acceptance by the deployment owner.

[SLSA v1.2 Build requirements](https://slsa.dev/spec/v1.2/build-requirements), [CycloneDX specification](https://cyclonedx.org/specification/overview/)

## External assessment

The proposed design has no certification, accreditation, deployment authorization, FIPS module selection or assessment result. CMVP validates specified cryptographic modules in specified operating environments. Common Criteria certification requires evaluation under a recognized scheme with an accredited laboratory and a defined target of evaluation. ISO/IEC 17025 accreditation applies to the laboratory and its accredited scope. Legal admissibility and suitability depend on the jurisdiction, examiner procedure and evidence in the individual case.
