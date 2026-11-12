# Visual guide

## Toolbox navigation

```mermaid
flowchart LR
    T[Toolbox] --> S[Scan media]
    T --> D[Devices and storage]
    T --> C[Capture and clone]
    T --> P[Phone clone]
    T --> F[Forensics]
    T --> Q[Security scan]
    T --> L[Scan logs]
    D -->|Select mounted volume| S
    S -->|Inspect a listed file| V[Verified file preview or copy]
    C -->|Saved pair| K[Compare captures]
    F -->|Existing image| E[Range scan or verified image clone]
    F -->|Readable Android or iOS export| S
    F --> L
    P -->|Encrypted archive| A[Android companion]
    S --> L
    Q --> L
```

The light sidebar is always available. The Toolbox cards open the same workflows. **Sanitize** and **Automation** are separate entries in the sidebar.

## Before and after capture

```mermaid
flowchart LR
    A[Choose a folder or acquired image] --> B[Capture before]
    B --> C[Preserve the later source state]
    C --> D[Capture after]
    D --> E[Compare saved captures]
    E --> F[Review observations and export JSON]
```

Take **Before** while the earlier state is still available. Take **After** after the change you want to study. Captures save hashes and observations. Use **Clone image file** or an encrypted file archive when you need to preserve a copy of the bytes.

## Which source should I choose?

| What you have | Where to go |
| --- | --- |
| Raw or Android sparse image | **Scan media → Disk image** |
| External disk mounted by macOS | **Devices & storage → Scan files** |
| Existing export from a phone | **Scan media → Android export folder** or **iOS export folder** |
| A file you want to hash | **Forensics → Verify SHA-256** |
| Acquired image and a known byte range | **Forensics → Run forensic scan** |
| SMS, calls, contacts, mail, WhatsApp, Messenger, or device-info exports | **Forensics → Review phone exports** |
| A suspicious sample or exported text log | **Security scan** |
| Files to copy to another Android phone | **Phone clone → Review files** |

See the [user manual](user-manual.md) for interpretation and export details.
