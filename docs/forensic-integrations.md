# Open-source forensic integrations

OmniRecovery can take authorized exports or raw images, run a selected upstream analysis tool, and save the reports with hashes. The core repository stays under its MIT license. Install analysis tools separately under their upstream licenses.

## Workflow and validation

1. Capture or import the source through OmniRecovery. Preserve the original, confirm its integrity, and record any missing or inaccessible data.
2. Choose the analysis tool according to the source and evidence question. Install a reviewed release or pinned checkout. For ALEAPP and iLEAPP, use a dedicated Python environment and retain the entrypoint digest with the dependency inventory.
3. The adapter creates a private staging copy and checks it against the source. Reports and their provenance are saved in a separate acquisition folder with a SHA-256 manifest.
4. Review findings alongside their source. Preserve the tool’s original report and record which files, filesystem structures, timestamps or parser assumptions support each conclusion.
5. Verify the report folder before export. Use the custody commands to sign the manifest and retain a separate checkpoint when required.

## Available adapters

In the desktop app, open **Forensics → Mobile artifact analysis**. Choose the source type, export folder, trusted parser checkout, and Python executable. Enter the trusted SHA-256 for `aleapp.py` or `ileapp.py`, then select **Analyze artifacts** and a destination folder. For Sleuth Kit, select a raw image under **Evidence source**, set its filesystem start sector and sector size, then choose **List deleted entries**. Choose a listed file and select **Recover selected entry** to extract a candidate with `icat`. The report records the filesystem address, the listed path, and the source hash. For the same selected raw image, **List partitions** runs TestDisk and **Carve file candidates** runs PhotoRec. Use **FQLite report** to import a CASE JSON-LD export from FQLite.

| Tool | Source | Result |
| --- | --- | --- |
| ALEAPP | Saved Android filesystem export | Upstream artifact reports with the entrypoint digest and source manifest digest |
| iLEAPP | Saved iOS filesystem export or compatible unencrypted iTunes/Finder backup directory | Upstream artifact reports with the entrypoint digest and source manifest digest |
| The Sleuth Kit `fls` and `icat` | Regular raw disk or partition image | Deleted metadata and selected file candidates with source SHA-256 and sector geometry |
| TestDisk | Regular raw image | Partition inventory from the copied image |
| PhotoRec | Regular raw image | Hashed whole-image signature matches. File allocation and deletion status are unknown |
| FQLite | CASE JSON-LD report from the FQLite desktop export | Original report, report hash, and a preview of reported tables and offsets |

```sh
omnirecovery analyze-leapp aleapp /cases/android-export /cases/reports \
  --checkout /opt/aleapp --python /opt/aleapp-venv/bin/python \
  --script-sha256 ENTRYPOINT_SHA256

omnirecovery analyze-leapp ileapp /cases/iphone-backup /cases/reports \
  --format itunes --checkout /opt/ileapp --python /opt/ileapp-venv/bin/python \
  --script-sha256 ENTRYPOINT_SHA256

omnirecovery tsk-deleted /cases/card.dd /cases/reports \
  --sector-size 512 --sector-offset 0

omnirecovery tsk-recover /cases/card.dd /cases/reports \
  --sector-size 512 --sector-offset 0 --address FILESYSTEM_ADDRESS \
  --source-sha256 SHA256_FROM_THE_LISTING

omnirecovery testdisk-list /cases/card.dd /cases/reports
omnirecovery photorec-carve /cases/card.dd /cases/reports
omnirecovery import-fqlite /cases/sqlite-report.jsonld /cases/reports
omnirecovery verify-acquisition /cases/reports/OmniRecovery-acquisition-EXAMPLE
```

The destination parent must already exist and have room for the input copy and results. The LEAPP adapter expects trusted upstream code and its installed dependencies. The recorded digest covers the entrypoint. Dependency verification and operating-system isolation belong to the analysis environment. In air-gapped work, provision dependencies offline and enforce the network policy at the operating-system level. Use a local backup copy when handling iTunes encryption. The adapter has no passphrase forwarding.

[Android ALEAPP](https://github.com/abrignoni/ALEAPP) · [iOS iLEAPP](https://github.com/abrignoni/iLEAPP) · [Sleuth Kit fls](https://www.sleuthkit.org/sleuthkit/man/fls.html)

Sleuth Kit saves the selected candidate as `data/recovered.bin` and records its original listed path in `data/omnirecovery-analysis.json`. Deleted extents may hold reused data. TestDisk saves `data/testdisk-partitions.txt`. PhotoRec saves files under `data/carved.*` for review and may report false matches. FQLite import preserves the upstream report and its claimed SQLite source metadata. Running database recovery and verifying its source hashes remain separate steps.

[Sleuth Kit icat](https://www.sleuthkit.org/sleuthkit/man/icat.html) · [CGSecurity scripted usage](https://www.cgsecurity.org/wiki/Scripted_run) · [FQLite CASE exporter](https://github.com/pawlaszczyk/fqlite/blob/master/src/fqlite/export/CASEExport.java)

## 0.3.10 validation

- ALEAPP reported the seeded model from a synthetic Android `vendor/build.prop`. OmniRecovery recorded 125 report files with hashes. [Tested ALEAPP revision](https://github.com/abrignoni/ALEAPP/commit/6f0fc4d22d0b661bebcc7b1415c087a87607dc6c)
- iLEAPP reported the seeded build from a synthetic iOS `SystemVersion.plist`. OmniRecovery recorded 107 report files with hashes. [Tested iLEAPP revision](https://github.com/abrignoni/iLEAPP/commit/ff134661f602c2009a87f709b85970e608dcac73)
- Linux CI used the installed Sleuth Kit to find a seeded deleted FAT entry. [Linux package validation](https://github.com/sabbirimon/OmniRecovery/actions/runs/43215121835)

The LEAPP checks used Python 3.13 on Intel macOS with network access disabled during analysis. Coverage applies to the seeded Android and iOS filesystem exports. A real Finder backup and broader artifact coverage remain validation work.

## Upstream repository audit

Reviewed on 2026-11-12 using upstream repositories, documentation, and license files. Adapter status refers to OmniRecovery. A repository review establishes an integration candidate. Each proposed adapter has its own execution and ground-truth tests.

| Upstream | License and status | Integration and first validation |
| --- | --- | --- |
| [scrcpy](https://github.com/Genymobile/scrcpy) | Apache-2.0. Integrated as an installed helper | Keep display-only USB and paired Wi-Fi preview in Device Service. Qualify launch and teardown, transport selection, and opt-in camera/audio on separate supported devices |
| [ALEAPP](https://github.com/abrignoni/ALEAPP) | MIT. Optional CLI adapter | Qualify a pinned checkout against a representative Android export. Map tool results to the source path, parser and timestamp basis, and expose verified reports in Forensics |
| [iLEAPP](https://github.com/abrignoni/iLEAPP) | MIT. Optional CLI adapter | Qualify a known filesystem export and an unencrypted Finder or iTunes backup. Verify handling of missing or unsupported domains and explicit time-zone conversion |
| [androidqf](https://github.com/mvt-project/androidqf) | MVT License 1.1. Planned | Import previously collected reports and retain their original metadata. Check collection-side device effects and its usage conditions before any separate live adapter |
| [AFLogical OSE](https://github.com/nowsecure/android-forensics) | GPLv3 text. Archived. Planned legacy export import | Parse existing SMS, MMS, call and contact CSV fixtures. Test schema and encoding differences from legacy Android exports |
| [OpenMF](https://github.com/scorelab/OpenMF) | Apache-2.0. Planned | Import its previously collected database, JSON and TSV artifacts. Exercise timestamp and schema conversion. The upstream live collector expects a rooted Android phone |
| [OmniForensix-Mobile](https://github.com/MrKhan025/OmniForensix-Mobile) | Apache-2.0 text in the repository. Planned evaluation | Compare documented SQLite, plist and application artifacts with known fixtures. Review dependency licenses, format compatibility, and key handling before choosing an adapter |
| [RecoverKit](https://github.com/DataDave-Dev/recoverkit) | Apache-2.0. Planned | Import its manifests and correlation results. Test against a staged deletion timeline and non-deletion causes of record-ID gaps. Preserve original evidence and distinguish thumbnails from full files |
| [FQLite](https://github.com/pawlaszczyk/fqlite) | Apache-2.0. CASE report import available | Qualify a pinned version on copied SQLite databases and supplied sidecars. Validate page or row locations, deleted-record candidates, false positives, and recovered values against known ground truth |
| [The Sleuth Kit](https://github.com/sleuthkit/sleuthkit) | Mixed component licenses. `fls` and selected `icat` extraction available | Extend tests beyond the seeded FAT fixture. Test FAT, NTFS and ext images with intact, fragmented and overwritten examples |
| [Autopsy](https://github.com/sleuthkit/autopsy) | Apache-2.0 license text and component-specific notices. Planned | Exchange documented case exports or reports. Test timestamps, data-source identity and unsupported artifacts on representative exported cases |
| [Digler](https://github.com/ostafen/digler) | MIT. Early upstream release. Planned benchmark | Compare carving on verified images with known signatures, truncation, embedded objects and overlapping extents. Keep the upstream report and do not infer deletion from a signature match |
| [TestDisk](https://github.com/cgsecurity/testdisk) | GPL-2.0 repository. Partition listing available | Extend partition discovery tests against disposable copies and damaged partition tables. A partition write is a separate operation |
| [PhotoRec](https://github.com/cgsecurity/testdisk) | Same upstream repository as TestDisk. Whole-image carving available | Compare output hashes and coverage with seeded evidence and preserve tool-specific provenance |
| [RecoverPy](https://github.com/PabloLec/RecoverPy) | GPL-3.0 repository. Planned evaluation | Evaluate its interactive block search with an offline synthetic image. Preserve hit offsets and verify reconstructed bytes where a reference digest is available |
| [Volatility 3](https://github.com/volatilityfoundation/volatility3) | Volatility Software License. Planned report import | Start with already exported reports and verified RAM images. Pin symbol sets and record missing-symbol errors. Review its wrapper-specific license terms before distributing a dedicated wrapper |
| [Data Recovery Using Python](https://github.com/alok-kumar8765/Data_Recovery_Using_Python) | MIT. Planned benchmark | Review its imaging and carver implementation against the Rust engine. Test advertised formats and hash reporting with the same fixture corpus. The submitted link joined this repository with the topic index |
| [ForensicsTools](https://github.com/mesquidar/ForensicsTools) | CC0-1.0 curated index | Use as a discovery source. Evaluate each proposed tool’s own repository, license, release and tests before selecting it |
| [Awesome Forensics](https://github.com/cugu/awesome-forensics) | CC0-1.0 curated index | Use to locate comparison tools and test corpora. Check the dataset or tool owner’s distribution terms before incorporating any material |
| [GitHub file-recovery topic](https://github.com/topics/file-recovery) | Discovery index with independent projects | Use to identify specific candidates. Record the exact repository and review it before introducing a dependency |

## Implementation sequence

1. **Broaden Forensics validation.** Add imported report review and helper availability checks. Extend the pinned ALEAPP and iLEAPP fixtures to representative artifacts and an unencrypted Finder backup, and compare results to direct invocation. Keep the Sleuth Kit FAT fixture as a regression check.
2. **Standardize provenance.** Store the source digest, tool/version, argument profile, read scope, original report digest, and per-item evidence location. Display items as active files, deleted metadata, carved candidates, thumbnails, or fully verified exports as supported by their evidence. Treat gaps and unavailable regions as observations. Open HTML reports as external files or show parsed fields without executing upstream report content in the desktop app.
3. **Add mobile record and disk recovery coverage.** Qualify FQLite import against actual recovered records from copied SQLite databases and sidecars. Add mobile collectors’ exports and broaden selected `icat` extraction fixtures. Verify source integrity and output SHA-256. Test unavailable permissions, encrypted inputs, unsupported schemas, missing sidecars, and files whose extents have been reused.
4. **Compare independent methods.** Evaluate RecoverKit and OmniForensix-Mobile on named artifacts. Benchmark PhotoRec, Digler, and Data Recovery Using Python on the same images. Track recall, wrong matches, time, peak memory, and whether each result includes original path or offset. Keep TestDisk partition inspection, RecoverPy block search, and Volatility report import tied to their own evidence types.
5. **Qualify the release.** Pin helper revisions and dependency versions, document upstream notices and distribution choices, and make the offline install path explicit. Test macOS, Windows, and Linux on supported fixture formats and dedicate authorized hardware to USB flows. Include corrupt files, timeouts, duplicate results, source changes, and read-only input checks. Treat test results as applying to the exact helper, platform, and fixture combination exercised.

The desktop source tree stays MIT. The current adapters use installed upstream tools with their own terms. The remaining entries are planned work or research references.
