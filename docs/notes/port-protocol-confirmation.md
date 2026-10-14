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
  `cbLogDir` is the BYTE length of the UTF-16 path, without the `\??\` prefix (the driver prepends it) and without a nul.
- Backup temp suffix: `.nrbnew`. Completion flag file: `<name>.nrbnew.nrbflag` (as implemented in `finalize::flag_path`).

## Pending (not done here)

1. Open `C:\Program Files (x86)\Server Network Recycle Bin Tool\snetrbinsrv.exe` in REA, find the
   `FilterSendMessage` import and decompile its callers; confirm the InBuffer/OutBuffer mapping per call.
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
