# 🩺 Disk Doctor

Forensic recovery toolkit for macOS — recover deleted projects, carve files from raw disk, and re-clone from git remotes.

## Quick Start

```zsh
# Build & run
cd ~/Projects/recovery-tool
cargo build --release
sudo ./target/release/disk-doctor
```

Or use the launcher:
```zsh
sudo ~/disk-doctor
```

> ⚠️ **Disk-level operations require `sudo`** (Imaging, Partition Scan, File Carver, Hex Viewer).
> **Git Recovery** tab works without `sudo`.

---

## Tabs

### 🔗 Git Recovery (start here)
Recovers project remotes from three sources:
1. **Surviving `.git/config`** — scans `~/Projects` for any remaining git configs
2. **Shell history** — parses `~/.zhistory`, `~/.bash_history` for `git clone` commands
3. **GitHub CLI** — runs `gh repo list` for repos you had access to

➡ Click **"Generate Re-clone Script"** → saves `~/recovery.sh` to re-clone everything from GitHub with full history.

### 🔧 File Carver
Scans a raw disk or disk image for file signatures (magic bytes) and extracts recoverable files:

| Type | Extensions | Magic |
|------|-----------|-------|
| ZIP / JAR / APK / AAR | .zip .jar .apk .aar | `PK\x03\x04` |
| Java Class | .class | `0xCAFEBABE` |
| DEX (Android) | .dex | `dex\n` |
| Mach-O (macOS/iOS) | .bin | `0xFEEDFACE` / `0xCFFAEDFE` |
| ELF (Linux) | .elf .so | `\x7FELF` |
| WebAssembly | .wasm | `\0asm` |
| JPEG / PNG / GIF | .jpg .png .gif | Image headers |
| PDF | .pdf | `%PDF` |
| SQLite | .sqlite .db | `SQLite format 3\0` |
| MP4 / MP3 | .mp4 .mp3 | Media headers |
| GZip / 7z / RAR | .gz .7z .rar | Compression headers |

1. Set **Source** to your Data volume: `/dev/disk3s5`
2. Set **Output** directory: e.g. `~/carved_files`
3. Check the file types you want to scan for
4. Click **Start Carving**

> ⚠️ Source code files (`.rs`, `.java`, `.kt`, `.swift`, `.dart`, `.py`) are **plain text** — they lack magic bytes.
> Use **Photorec** (see Git Recovery tab → "Photorec guide") for text-file recovery.

### 💾 Imaging
Creates a forensically-sound byte-for-byte copy of a disk/partition.

- SHA-256 computed during copy
- **Verify** button checks source vs. destination hash
- Use an **external drive** as destination — never write to the disk being recovered

### 🔍 Partition Scan
Scans the first ~50MB of a device for partition headers:
- GPT (`EFI PART`)
- APFS Container (`NXSB`) / Volume (`APSB`)
- HFS+, exFAT, FAT32, NTFS

Useful for finding lost partitions or verifying disk structure.

### 💻 Disk Info
Lists all disks/partitions from `diskutil list` with refresh support.

### 📝 Hex Viewer
Browse raw bytes at any offset on any device or file.
- Go to offset (decimal or `0x...`)
- Page through 64KB windows
- ASCII preview alongside hex

---

## Recovery Workflow (recommended order)

```
1. 🔗 Git Recovery  ──→  Re-clone from remotes (fastest, full history)
2. 🔧 File Carver   ──→  Scan raw disk for binary source artifacts
3. 📖 Photorec      ──→  Text source recovery (last resort)
```

### Step-by-step

1. Open **Git Recovery** → click **Scan for recoverable repos**
2. Review found repos — ones with ✅ still have local data
3. Click **Generate Re-clone Script** → **Save to ~/recovery.sh**
4. Run `zsh ~/recovery.sh` to re-clone all projects
5. For files not in git: **File Carver** tab, set source to `/dev/disk3s5`
6. Start carving — recovered files go to your output directory

---

## Technical Notes

- **APFS + SSD + TRIM**: Deleted blocks are physically erased quickly. Recovery odds drop sharply after deletion.
- **Raw devices**: Use `/dev/rdisk3s5` (raw) for faster reads vs `/dev/disk3s5` (buffered).
- **No filenames**: File carving recovers content only — filenames and folder structure are lost.
- **Image first**: For forensics, create a disk image (Imaging tab) and carve from the image, not the live device.

## Dependencies

- Rust 2021 edition
- [egui](https://github.com/emilk/egui) — immediate-mode GUI
- [sha2](https://crates.io/crates/sha2) — SHA-256 verification
