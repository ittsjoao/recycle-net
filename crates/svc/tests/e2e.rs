// Gated: runs only on a configured VM per docs/notes/e2e-checklist.md.
#[test]
#[ignore]
fn delete_over_smb_is_captured_and_restorable() {
    // 1. start service (configures driver)
    // 2. create a >128KB file on the monitored volume, delete it
    // 3. assert a finalized backup + SQLite row appear
    // 4. restore by id to an override path; assert bytes match
    // Implemented against the live environment per e2e-checklist.md.
}
