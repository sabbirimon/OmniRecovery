# Open-source forensic integrations

OmniRecovery can take authorized exports or raw images, run a selected upstream analysis tool, and save the reports with hashes. The core repository stays under its MIT license. Install analysis tools separately under their upstream licenses.

## Workflow and validation

1. Capture or import the source through OmniRecovery. Preserve the original, confirm its integrity, and record any missing or inaccessible data.
2. Choose the analysis tool according to the source and evidence question. Install a reviewed release or pinned checkout. For ALEAPP and iLEAPP, use a dedicated Python environment and retain the entrypoint digest with the dependency inventory.
3. The adapter creates a private staging copy and checks it against the source. Reports and their provenance are saved in a separate acquisition folder with a SHA-256 manifest.
4. Review findings alongside their source. Preserve the tool’s original report and record which files, filesystem structures, timestamps or parser assumptions support each conclusion.
5. Verify the report folder before export. Use the custody commands to sign the manifest and retain a separate checkpoint when required.

## Available adapters

| Tool | Source | Result |
| --- | --- | --- |
| ALEAPP | Saved Android filesystem export | Upstream artifact reports with the entrypoint digest and source manifest digest |
| iLEAPP | Saved iOS filesystem export or compatible unencrypted iTunes/Finder backup directory | Upstream artifact reports with the entrypoint digest and source manifest digest |
| The Sleuth Kit `fls` | Regular raw disk or partition image | Deleted entries reported by the selected filesystem parser, with its version, source SHA-256 and sector geometry |

```sh
omnirecovery analyze-leapp aleapp /cases/android-export /cases/reports \
  --checkout /opt/aleapp --python /opt/aleapp-venv/bin/python \
  --script-sha256 ENTRYPOINT_SHA256

omnirecovery analyze-leapp ileapp /cases/iphone-backup /cases/reports \
  --format itunes --checkout /opt/ileapp --python /opt/ileapp-venv/bin/python \
  --script-sha256 ENTRYPOINT_SHA256

omnirecovery tsk-deleted /cases/card.dd /cases/reports \
  --sector-size 512 --sector-offset 0

omnirecovery verify-acquisition /cases/reports/OmniRecovery-acquisition-EXAMPLE
```

The destination parent must already exist and have room for the input copy and results. The LEAPP adapter expects trusted upstream code and its installed dependencies. The recorded digest covers the entrypoint. Dependency verification and operating-system isolation belong to the analysis environment. In air-gapped work, provision dependencies offline and enforce the network policy at the operating-system level. Use a local backup copy when handling iTunes encryption. The adapter has no passphrase forwarding.

[Android ALEAPP](https://github.com/abrignoni/ALEAPP) · [iOS iLEAPP](https://github.com/abrignoni/iLEAPP) · [Sleuth Kit fls](https://www.sleuthkit.org/sleuthkit/man/fls.html)

## Additional tool evaluation

| Project | Planned scope and validation |
| --- | --- |
| androidqf | Import verified acquisition archives first. Review its use-restricted license and on-device collector before considering a live adapter. [androidqf](https://github.com/mvt-project/androidqf) |
| AFLogical OSE and OpenMF | Evaluate legacy Android provider access and rooted-device assumptions on dedicated fixtures. Require current repository provenance before selecting either for distribution |
| OmniForensix-Mobile and recoverkit | Review source formats, license terms and corroborated recovery claims before selecting adapters. The recoverkit reference requires an exact upstream repository. [OmniForensix-Mobile](https://github.com/MrKhan025/OmniForensix-Mobile) |
| FQLite | Qualify extraction of candidate records from SQLite unallocated pages and sidecars, including page offsets, schema ambiguity and known deleted records. [FQLite](https://github.com/pawlaszczyk/fqlite) |
| Autopsy and The Sleuth Kit `icat` | Test filesystem-aware extraction by metadata address and case export interoperability. Validate original bytes against known ground truth before labeling a file complete. [Autopsy](https://github.com/sleuthkit/autopsy) · [icat manual](https://www.sleuthkit.org/sleuthkit/man/icat.html) |
| Digler, PhotoRec and TestDisk | Benchmark carving and partition discovery against verified raw images. Compare recovered extents and false positives. Any partition write operation requires its own qualified workflow. [Digler](https://github.com/ostafen/digler) · [TestDisk and PhotoRec](https://github.com/cgsecurity/testdisk) |
| RecoverPy and Volatility 3 | Evaluate raw block search and captured memory as distinct input types with separate provenance, symbols and fixture sets. [RecoverPy](https://github.com/PabloLec/RecoverPy) · [Volatility 3](https://github.com/volatilityfoundation/volatility3) |
| ForensicsTools and Awesome Forensics | Use curated lists for discovery. Review original licenses, releases and tests before integrating individual projects. [ForensicsTools](https://github.com/mesquidar/ForensicsTools) |
