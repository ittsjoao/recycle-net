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
- Backup temp suffix: `.nrbnew`. Completion flag file: `<name>.nrbnew.nrbflag` (as implemented in `finalize::flag_path`).

## Pending (not done here)

1. Done (2026-10-15, REA + Ghidra 12.1.4): the port client is `drv.dll` (x86, loaded by `snetrbinsrv.exe`), exporting
   `connectToDriver`, `clearDirItems`, `addInclMask`/`addExclMask` (cmd 0/1, `WCHAR[1024]` OutBuffer of 0x800 bytes,
   nul-terminated), `setMonitorSetup` (cmd 3, 0x804-byte OutBuffer; it also runs `GetFullPathNameW` + `CreateDirectoryW`
   on the dir) and `loadDriver` (`FilterLoad("netrbin")`). Command in InBuffer, payload in OutBuffer: confirmed. The
   driver `MessageCallback` checks only `InputBufferLength > 3`, never the OutBuffer length.
2. Confirm the exact flag file name/extension by finding the string the service searches for in the recycle dir.
3. Record how the original stores its config (registry/ini) and its SQLite path.

## Runtime confirmation procedure (fallback / cross-check)

On a VM only (never a production workstation): run the original service under Process Monitor or a
port trace, delete a file on a monitored volume, and capture the exact FilterSendMessage buffers and the
files produced in the recycle dir (`.nrbnew` plus flag). Record results here. If the payload turns out
to be in InBuffer, swap the buffer arguments in `PortClient::send` (public API unchanged).

## Path model assumption (PENDING VM confirmation)

The service assumes the driver mirrors the volume-relative path under the recycle root as
`<recycle_root>\<DriveLetter>\<rest of path>` (e.g. `D:\data\a.txt` -> `<root>\D\data\a.txt.nrbnew`).
`finalize::derive_original` maps this to volume `D:` and original path `D:\data\a.txt`. If the real layout differs,
only `derive_original` needs to change. Final backups get a counter before the extension on name collision
(`a.1.txt`); the original path recorded never contains that suffix.
