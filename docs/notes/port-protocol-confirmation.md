# netrbin control port protocol confirmation

Status: PARTIALLY CONFIRMED (recovered from the driver); runtime confirmation PENDING.
No live reverse engineering or driver connection was performed as part of this task.

## Recovered protocol

- Port name: `\CaptureFileMonitorPort` (single-client; the `SNetRBin` service must be stopped and disabled first).
- Command codes (int32): 0 = AddIncludeMask, 1 = AddExcludeMask, 2 = ClearDirItems, 3 = SetupMonitor.
- Buffer placement: the driver `MessageCallback` reads the command int from InBuffer and the payload from OutBuffer:
  `FilterSendMessage(hPort, &cmd, 4, payload, payload_len, &returned)`.
- Mask payload: nul-terminated UTF-16LE string.
- SetupMonitor payload: `u8 bEnabled; u8 pad; u16 cbLogDir; WCHAR path[]`.
  `cbLogDir` is the BYTE length of the UTF-16 path, without the `\??\` prefix (the driver prepends it) and without the nul,
  but the path MUST be nul-terminated: the driver copies it with `RtlAppendUnicodeToString` (reads to the nul, ignores
  `cbLogDir`) into a buffer of exactly `cbLogDir + 8 + 2` bytes. Without the nul it reads past the payload, the append
  fails and the target stays `\??\` while the monitor is enabled: deletes pass and nothing is copied (seen on a
  Server 2019 install, 2026-10-15).
## What the driver does on a delete (read from netrbin.sys, 2026-10-15)

netrbin.sys is the WDK "Delete" sample (6.3.9600) plus a copy step. Static reading only; runtime pending.

- Seen deletes: `IRP_MJ_SET_INFORMATION` with class **13 only** (`FileDispositionInformation`) and
  `FILE_DELETE_ON_CLOSE` at create. Class 64 (`FileDispositionInformationEx`, used by `DeleteFileW` since
  Windows 10 1809 / Server 2019 for local deletes) is ignored: local `del`/Explorer deletes on the server are
  not captured. SMB deletes depend on the class srv2 passes down.
- The copy runs in PreCleanup when the monitor is enabled, the file is not a directory, and the path matches
  an include and no exclude mask.
- Matched path: `\` + DOS drive + parent + name, e.g. `\E:\Dados\TI\a.txt` (leading backslash; the drive
  comes from `IoVolumeDeviceToDosName` with the `:` stripped and `\` prepended, then `:` appended back).
  Masks are split on `;` and matched with `FsRtlIsNameInExpression(IgnoreCase)`: write them in uppercase
  with the leading backslash (`config::driver_masks`).
- Copy target: `<log dir>\<Letter>\<parent>\<name>` (e.g. `<root>\E\Dados\TI\a.txt`, original name, no
  suffix); the driver creates missing directories. Opened with `FILE_CREATE`: if the name exists the copy
  is dropped silently (`STATUS_OBJECT_NAME_COLLISION`).
- Then it creates an empty completion flag `<root>\E\Dados\TI\a.txt.nrbnew`, even when the copy was dropped.
  The service moves finalized copies to `<root>\.store\...` so the driver's names stay free.

## Pending (not done here)

1. Done (2026-10-15, REA + Ghidra 12.1.4): the port client is `drv.dll` (x86, loaded by `snetrbinsrv.exe`), exporting
   `connectToDriver`, `clearDirItems`, `addInclMask`/`addExclMask` (cmd 0/1, `WCHAR[1024]` OutBuffer of 0x800 bytes,
   nul-terminated), `setMonitorSetup` (cmd 3, 0x804-byte OutBuffer; it also runs `GetFullPathNameW` + `CreateDirectoryW`
   on the dir) and `loadDriver` (`FilterLoad("netrbin")`). Command in InBuffer, payload in OutBuffer: confirmed. The
   driver `MessageCallback` checks only `InputBufferLength > 3`, never the OutBuffer length.
2. Done: flag and layout read from the driver (section above).
3. Record how the original stores its config (registry/ini) and its SQLite path.

## Runtime confirmation procedure (fallback / cross-check)

On a VM only (never a production workstation): run the original service under Process Monitor or a
port trace, delete a file on a monitored volume, and capture the exact FilterSendMessage buffers and the
files produced in the recycle dir (copy plus `.nrbnew` flag). Record results here.

## Path model

The driver mirrors the path under the recycle root as `<recycle_root>\<DriveLetter>\<rest of path>`
(`D:\data\a.txt` -> `<root>\D\data\a.txt` + `a.txt.nrbnew`). `finalize::derive_original` maps it back to volume
`D:` and `D:\data\a.txt`. Finalized backups move to `<root>\.store\D\data\a.txt`, with a counter before the
extension on name collision (`a.1.txt`); the original path recorded never contains that suffix.
