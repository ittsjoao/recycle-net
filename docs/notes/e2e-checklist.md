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

## Run
`cargo test -p rnetrbin-svc --test e2e -- --ignored` (skeleton; implement against the VM) and
`cargo test -p rnetrbin-svc --test port_live -- --ignored`.

## Results
(to be recorded)
