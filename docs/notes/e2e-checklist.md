# End-to-end checklist (VM only, manual)

Status: PENDING. Never run on a production workstation: the port is single-client and the steps stop services.

## VM setup
Two topologies; run the cases on both when possible.
- **Standalone (target):** the original product is NOT installed. The driver is
  embedded in the exe at build time; run the setup (it writes the config and installs
  everything), or by hand from an elevated prompt: `rnetrbin-svc.exe init-config
  --recycle-dir <dir>`, `rnetrbin-svc.exe install` and `sc.exe start rnetrbin-svc`.
- **Coexisting:** original product installed (`SNetRBin` present); `install` reuses the
  driver and the service takes over SNetRBin at start.
- A data volume plus an SMB share on it.
- `C:\ProgramData\rnetrbin\config.toml` based on `config.example.toml`.

## Setup
N1. First install asks for "Pasta da lixeira" (default `C:\RecycleBinNet`, refuses a
    populated folder) and writes `config.toml` with zero folders; an upgrade keeps the
    config and does not show the page.
N2. O setup registra e inicia o serviço: after Concluir, `sc query rnetrbin-svc` is
    RUNNING, `fltmc` lists `netrbin`, and the window opens on Pastas protegidas.
N3. A failing `install` (e.g. driver stuck STOP_PENDING) shows the pt-BR message with the
    detail, stops on the Preparing page, copies no file and leaves no `rnetrbin-svc`;
    `C:\ProgramData\rnetrbin\setup.log` has the full output.
N4. Upgrade with the service running: it is stopped, `install` re-runs, and it is
    started again; a service the user had stopped stays stopped.

## Install / uninstall (standalone)
I1. `install` creates `netrbin` (Type=2, Start=0, Group=Filter, ErrorControl=0) with
    `Instances\netrbin Instance` Altitude=370150 Flags=0; `fltmc` lists `netrbin` at 370150.
I2. `install` twice is idempotent (no error, file not rewritten when identical).
I3. `install` with a missing/invalid config fails before touching the system.
I4. Reboot: driver loads at boot, `rnetrbin-svc` auto-starts and captures.
I5. Driver signature loads on the target OS. Check with HVCI/Memory Integrity ON
    (Windows 11 / Server 2022): the 2015 cross-signed driver may be blocked.
I6. `uninstall` without SNetRBin: service gone, `netrbin` unloaded, service key and
    `System32\drivers\netrbin.sys` removed (or "reboot required" and gone after reboot).
I7. `uninstall` with SNetRBin installed (or `--keep-driver`): only our service is removed;
    SNetRBin is back to its original start type and running.
I8. Coexisting `install`: SNetRBin's `netrbin.sys` is not rewritten ("usando o driver já
    instalado"); a `netrbin` left at Start=4 is set back to 0 ("reativado").
    Upgrading while `netrbin` is loaded keeps the installed driver (file not rewritten).

## Cases
1. Large file (multiple 64KB chunks, > 128KB) deleted over SMB: finalized backup plus SQLite row appear; restore by id matches bytes.
2. Excluded mask (e.g. `*.tmp`) is not captured.
3. Quota eviction: exceed `max_bin_bytes`; oldest non-restored finalized backups are evicted first.
4. Service stopped during a delete, then started: an orphan `.nrbnew` with a flag is reconciled; one without a flag is ignored.
5. A `.nrbnew` without a flag is never indexed.

## Must-confirm protocol facts (parked T1 — the code assumes these)
These are assumptions baked into the service; confirm each on the VM and correct
the code if wrong (see also docs/notes/port-protocol-confirmation.md):
6. **Buffer placement:** the driver reads the command from the InBuffer and the
   payload from the OutBuffer of `FilterSendMessage`. If reversed, `PortClient::send`
   swaps the two arguments.
7. **Flag file name/semantics:** confirm the completion flag is `<name>.nrbnew.nrbflag`
   and that its presence means "copy complete" (not "in progress"). The whole
   finalize/watch design keys on this.
8. **Recycle-dir feedback loop (highest risk):** with `rnetrbin-svc` running,
   confirm the driver does NOT capture the service's own writes inside
   `recycle_dir` — i.e. deleting a flag, evicting a backup, or the SQLite db does
   not produce new `.nrbnew` captures. If it does, backups loop. Mitigation in
   place: db_path is rejected inside recycle_dir, and the recycle dir is marked
   with `.rnetrbin-root`; still verify no self-capture.
9. **Same path deleted twice while the service is down:** both deletions must
   survive as two distinct backups (unique-name reservation); confirm the driver
   does not overwrite the first `.nrbnew`/flag with the second.
10. **Mirror layout:** confirm the on-disk path under the recycle root is
    `<root>\<DriveLetter>\<original relative path>` so `derive_original` restores
    to the right absolute path; adjust `derive_original` otherwise.

## "Never unprotected" invariant (SCM)
11. Takeover success then `sc stop rnetrbin-svc`: SNetRBin is re-enabled (original
    start type) and started; the `.rnetrbin-root`/`takeover.json` is cleared.
12. Takeover success then **kill** `rnetrbin-svc` (simulate crash): SCM failure
    actions restart it within ~60s; meanwhile confirm the exposure window.
13. Windows **shutdown** after takeover: SNetRBin stays disabled (we resume on boot).
14. Driver unload/reload while connected: heartbeat fails, the service drops the
    session and re-runs connect+configure; no capture gap beyond one tick.
15. Config made invalid after a persisted takeover, then restart: the service
    restores SNetRBin on the early-exit path.

## Rules and deleted-by
16. Rules: folder with Include `*.*` and Exclude `*.bak`; deleting `a.docx` and `a.bak`
    over SMB keeps only `a.docx`, also in a subfolder.
17. Deleted-by: the list shows `DOMAIN\user` of the SMB user within ~10 s of the delete;
    with File System auditing turned off by `auditpol`, the backup still appears and the
    user shows "desconhecido".
18. Large folder: protecting a folder with ~1M files is immediate (no per-file ACL
    rewrite; `auditpol /resourceSACL /type:File /view` shows the DELETE entry).
19. `uninstall` restores the previous File System audit setting and removes the global
    SACL entry (unless `sacl_existed` was true in `audit-state.toml`: then it stays);
    the Security log size stays.
20. Removing the last protected folder in the window: the list shows the empty text,
    saving works, and a delete over SMB captures nothing.

## Run
`cargo test -p rnetrbin-svc --test e2e -- --ignored` (skeleton; implement against the VM) and
`cargo test -p rnetrbin-svc --test port_live -- --ignored`.

## Results
(to be recorded)
