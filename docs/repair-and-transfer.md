# Repair and phone transfer

## Inspect a damaged file

```sh
omnirecovery repair damaged.png
omnirecovery repair damaged.jpg --expected-sha256 ORIGINAL_SHA256
```

`repair` reads a standalone file, verifies its source hash and reports the proposed change, its byte offset and the derivative hash. The Rust `preview_repair` API returns validated derivative bytes in memory. The CLI emits the assessment as JSON. Case-bound file export uses the planned case service.

| Input damage | Validation |
| --- | --- |
| Missing, truncated or checksum-damaged PNG `IEND` chunk | Reconstruct the canonical ending. Check all chunk CRCs, require a complete zlib stream with valid checksum and exact filtered pixel length, and decode the complete image |
| One changed bit in a PNG `IDAT` chunk of up to 4 MiB, or its CRC field | Use the CRC-32 syndrome to locate a possible single-bit change. Require a SHA-256 match to the original before reporting restored bytes, then verify and decode |
| Missing or truncated JPEG end marker | Reconstruct the marker, require a SHA-256 match to the original, and decode the complete file |
| Incorrect RIFF size in supported PCM WAV | Derive the outer size from the standalone file length. Validate the format and sample alignment and require an exact original SHA-256 match |
| Incorrect terminal `data` size, byte rate or alignment in supported PCM WAV | Derive a candidate size from the remaining bytes, or derive byte rate and alignment from sample rate, channels and sample width. Require validation and an exact original SHA-256 match |
| Incorrect 32-bit size in a final `moov` or `mdat` box of supported MP4 or MOV | Derive the final box size from the remaining bytes. Check sample extents and require an exact original SHA-256 match |
| Incorrect directory offset or size in a supported DOCX, XLSX or PPTX ZIP footer | Reconcile one intact directory boundary with the footer. Check the indexed entries, local headers and expected Office part names, then require an exact original SHA-256 match |

The input is limited to 16 MiB and existing decoder limits apply. Use a trusted pre-damage hash or backup manifest for `--expected-sha256`. Original-byte identity requires a matching trusted hash. The WAV, MP4, MOV and Office validators check container metadata. They do not play codecs, open documents, repair missing payloads, or rewrite Office XML. [PNG specification](https://www.w3.org/TR/png-3/), [JPEG specification](https://www.itu.int/rec/T-REC-T.81), [Microsoft RIFF documentation](https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-), [MP4 box registry](https://mp4ra.org/registered-types/boxes), [QuickTime movie format](https://developer.apple.com/documentation/quicktime-file-format/quicktime_movie_files), [Microsoft Office package structure](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-offmacro2/33b837b7-4e59-41c4-aed2-7497674a4b0b)

## Transfer files from a PC or previous phone export

1. Put the readable files from the previous phone export in one folder on the PC. A file inventory can establish which paths are present. If the previous clone is a raw or encrypted phone image, extract accessible files through an authorized workflow before creating the transfer archive.
2. Run `omnirecovery backup-files previous-phone-export/ transfer.omnibackup`. Enter and confirm a 12–128 character printable ASCII passphrase in the terminal. Choose a new archive path outside the source folder.
3. Copy `transfer.omnibackup` to the receiving phone, for example over USB. Open the OmniRecovery Android companion, select the archive and a writable folder, and enter the same passphrase.
4. Select **Verify and restore files**. The app authenticates the complete archive before creating a new folder. It restores the relative paths inside that folder, reads each written file back and checks its length and SHA-256. An interrupted restore can leave a new folder marked incomplete. Previous destination files are outside the restore path.

This personal transfer workflow moves ordinary files such as photos, videos and documents. It preserves byte contents and relative paths. Empty directories, original timestamps, permissions, app-private databases, credentials, Android settings and system partitions are outside the transfer format. Android's system picker grants access only to supported user-selected locations. A raw sector clone cannot be restored onto another phone using this app. [Android Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files), [Android file-based encryption](https://source.android.com/docs/security/features/encryption/file-based)

## Archive format

`OMNIBK01` starts with an eight-byte ASCII magic and a 16-byte random salt. Derive a 256-bit key using PBKDF2-HMAC-SHA256 with 600,000 iterations. Each record is a big-endian 32-bit plaintext length followed by AES-256-GCM ciphertext and a 16-byte tag. The nonce is four zero bytes followed by the big-endian 64-bit zero-based record index. Associated data consists of the header, record index and length. A new random salt yields a separate archive key. Passphrases are entered through the trusted UI and excluded from arguments, report fields and logs.

The first byte of each decrypted record identifies its type.

| Type | Payload |
| --- | --- |
| `1` | Relative UTF-8 file path with `/` separators |
| `2` | One to 1,048,576 bytes of the current file |
| `3` | Eight-byte file length and 32-byte SHA-256 of the current file |
| `4` | Four-byte file count and eight-byte total payload length. The archive must end immediately after this authenticated record |

All integers are big-endian. The writer accepts up to 20,000 files and 64 GiB of payload. It verifies each file against the inventory before publishing the archive without overwriting an existing path. The reader bounds record lengths, rejects duplicate and case-colliding paths and verifies file hashes, record order and the authenticated end. Android temporarily stages the encrypted archive in app-private storage so a successful verification pass covers the exact bytes used for restore. The phone needs room for both the staged encrypted archive and restored files.

The personal archive path has no connection to a forensic case store. The high-assurance deployment requires its own authenticated case, retention and export controls.

## Build the Android companion

Use Android SDK Platform 36, JDK 21 and Gradle 9.4.1. In `android/`, run `gradle :app:assembleDebug`. The APK is written to `android/app/build/outputs/apk/debug/app-debug.apk` and declares Android 8.0 or later. It declares no Internet permission.
