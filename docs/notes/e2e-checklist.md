# End-to-end checklist (VM only, manual)

Status: PENDING. Never run on a production workstation: the port is single-client and the steps stop services.

## VM setup
- Original product installed, `netrbin.sys` loaded, `SNetRBin` service present.
- A data volume plus an SMB share on it.
- `rnetrbin-svc` config (TOML) with `recycle_dir` on that volume, `includes`, `excludes`, `max_bin_bytes`.

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

## Run
`cargo test -p rnetrbin-svc --test e2e -- --ignored` (skeleton; implement against the VM) and
`cargo test -p rnetrbin-svc --test port_live -- --ignored`.

## Results
(to be recorded)
