# Android recovery workflow

The initial Rust implementation reads existing, immutable raw or Android sparse v1.0 images and logical export directories. It opens input files read-only. Use synthetic or independently prepared authorized inputs on a trusted host. Use a qualified read-only mount or an immutable snapshot when preservation includes host filesystem metadata. Live Android devices, block devices, encrypted backup containers and raw acquisition paths are outside this build's qualified input set.

## Pre-event and post-event data

| Event | Where to look | What OmniRecovery can establish |
| --- | --- | --- |
| File deletion | Older capture, export, surviving media bytes, accessible trash or cache, unallocated space on an authorized readable image | The current scanner identifies contiguous JPEG or PNG bytes that pass structure and decode checks. It also compares exact hashes and paths already present in logical exports. Surviving directory entries or Android database records require future parsers |
| Quick format or repartition | Pre-format images or backups, the formatted volume image and separately accessible removable media | Compare captures and look for complete media signatures in observed post-format bytes. Overwritten metadata, discard, encryption and inaccessible flash blocks can remove evidence needed to reconstruct names, files or prior contents |
| OS flash or update | Separate pre-event and post-event captures of the same partition or comparable logical exports | Some system flashes preserve userdata. A flash procedure can also wipe or replace userdata. An image of a flashed system partition only describes that partition. Record the exact command/method and compare like-for-like captures |
| Factory reset or bootloader unlock | Prior backups and captures, separately accessible removable media and artifacts outside the affected storage | On a modern encrypted phone, destruction or loss of the required keys makes old ciphertext inaccessible even when blocks survive. Bootloader unlock can wipe user data. The workflow does not perform flashing, unlocking or credential attacks |

Keep chronology and acquisition scope with each capture. A before-only finding means matching validated bytes were observed in the before capture only. It can reflect a deletion, a scope difference, unreadable regions or a changed encoding. When only a post-event capture exists, the scanner can report surviving data but cannot establish an earlier state by itself. The current comparison commands take capture order from the operator. They do not authenticate timestamps or infer creation or deletion dates. [Android flashing documentation](https://source.android.com/docs/setup/test/running), [Android bootloader locking and unlocking](https://source.android.com/docs/core/architecture/bootloader/locking_unlocking), [Android file-based encryption](https://source.android.com/docs/security/features/encryption/file-based), [Android metadata encryption](https://source.android.com/docs/security/features/encryption/metadata)

## Current commands

| Command | Accepted input | Output |
| --- | --- | --- |
| `scan` | One ordinary raw image or Android sparse v1.0 image | Deterministic JSON with container SHA-256, logical coverage, skipped bytes, an optional ext-family/F2FS magic hint, JPEG, PNG, MP4, MOV and PCM WAV findings and rejected candidates |
| `compare` | Two supported images in operator-supplied order | The two scan reports and exact media hashes observed in either or both. Duplicate occurrences keep their logical offsets |
| `inventory` | One directory of files already exported from Android | Relative paths, lengths and SHA-256 for accessible regular files. Links and special files are recorded as skipped |
| `compare-exports` | Two export directories in operator-supplied order | Exact path and hash differences with explicit skipped-path observations. The inventory can compare databases, videos and other files already present in the export without interpreting their internal formats |

JSON is suitable for local automation and may contain sensitive paths and hashes. Run only through a trusted caller on an approved input. Version 1 is a development schema. Agent grants, remote sessions, human approvals, durable case state and export enforcement are planned separately in the [automation design](automation.md).

`recover_candidate` in the Rust library returns exact validated candidate bytes in memory. It checks the input SHA-256 and revalidates the reported extent, byte digest and media properties. Saving or releasing these bytes requires an authorized host workflow. The current CLI has no file extraction or export command.

## Coverage and interpretation

- A raw image's offsets are byte positions within that file. For a sparse image, offsets are logical positions after expansion. The scanner verifies declared CRCs, scans raw extents and reports the exact offsets and lengths of FILL and DONT_CARE ranges. CRC calculation follows the format's zero convention for DONT_CARE without treating those zeros as evidence. Adjacent raw chunks can form one observed range. A candidate must fit inside one contiguous observed range. [Android sparse format definition](https://android.googlesource.com/platform/system/core/+/refs/heads/main/libsparse/sparse_format.h), [AOSP sparse reader](https://android.googlesource.com/platform/system/core/+/refs/heads/main/libsparse/sparse_read.cpp)
- Decode-verified JPEG and PNG findings use confidence level 3 from the [validation rubric](validation.md). Container-verified MP4, MOV and PCM WAV findings use level 2. Exact trusted ground truth is needed for level 4. A filesystem magic hint is only a signature observation. Filesystem allocation, deleted-entry parsing, fragmented-file assembly, APNG, contacts inside raw images, app-private data and SQLite/WAL row recovery remain future work.
- Limits include 16 MiB per photo candidate, 128 MiB per supported video or WAV candidate, 24 megapixels per photo, 4,096 attempted signatures and a 2 GiB candidate validation read budget. A stopped scan reports incomplete coverage. Directory inventories reject inputs above 100,000 entries or their path/depth limits. Scan and inventory require immutable source snapshots and a trusted directory tree. Hash checks detect ordinary changes and do not establish a secure boundary against a concurrent privileged writer. Decoders currently run in the CLI process.
- No device, certified blocker, smartcard or HSM was used for this slice. The tests measure known-answer behavior in generated fixtures. They do not establish recovery rates on actual post-reset or post-flash devices.

## Live Android qualification

After the vault, custody, consent and dual-approval gates pass, build a controlled test matrix of device models, OS versions, storage types and acquisition permissions. Seed known allocated and deleted files, media database records and app exports. Compare same-scope captures for ordinary deletion, explicit trash removal, a format with and without discard where the test hardware permits, an OS update that preserves userdata, and a destructive reset on sacrificial hardware. Include a pre-event backup as a distinct source. Test accessible and inaccessible credentials, recording precisely what the approved method can read. Keep controlled tests that involve writes separate from acquisition and examination workflows. Publish per-scenario findings and limits, including zero recoveries.
