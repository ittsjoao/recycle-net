//! Associate Security-log delete events (4663) with captured backups.
use crate::index::{DeleteEvent, Index};

/// Max distance between a backup's deleted_utc and its 4663 event.
pub const WINDOW_SECS: i64 = 120;
/// Rows without a user are retried for this long, then stay "desconhecido".
pub const PENDING_SECS: i64 = 600;

/// The event closest in time to `deleted_utc`, within WINDOW_SECS.
pub fn match_event(deleted_utc: i64, events: &[DeleteEvent]) -> Option<&DeleteEvent> {
    events
        .iter()
        .filter(|e| (e.utc - deleted_utc).abs() <= WINDOW_SECS)
        .min_by_key(|e| (e.utc - deleted_utc).abs())
}

/// Give recent rows without a user the user of their matching event.
/// Returns how many rows were filled.
pub fn fill_pending(idx: &Index, now: i64) -> rusqlite::Result<usize> {
    let mut filled = 0;
    for (id, path, utc) in idx.pending_attribution(now - PENDING_SECS)? {
        let events = idx.events_for(&path, utc - WINDOW_SECS, utc + WINDOW_SECS)?;
        if let Some(e) = match_event(utc, &events) {
            idx.set_user(id, &e.user, &e.sid)?;
            filled += 1;
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::DeletedFile;

    fn ev(user: &str, utc: i64) -> DeleteEvent {
        DeleteEvent { path: "D:\\A\\x.txt".into(), user: user.into(), sid: "S".into(), utc }
    }

    #[test]
    fn match_event_picks_nearest_within_window() {
        let evs = [ev("far", 1000 - 121), ev("near", 1000 + 5), ev("mid", 1000 - 60)];
        assert_eq!(match_event(1000, &evs).unwrap().user, "near");
        assert!(match_event(1000, &[ev("late", 1000 + 121)]).is_none());
        assert!(match_event(1000, &[]).is_none());
    }

    fn add(idx: &Index, path: &str, utc: i64) -> i64 {
        idx.insert(&DeletedFile {
            original_path: path.into(), backup_path: format!("E:\\L\\{utc}"), volume: "D:".into(),
            size: 1, deleted_utc: utc, user_sid: None, user_name: None,
        }).unwrap()
    }

    #[test]
    fn fill_pending_sets_user_from_event_case_insensitively() {
        let idx = Index::open_memory().unwrap();
        let id = add(&idx, "D:\\A\\x.txt", 1000);
        idx.add_delete_event(&DeleteEvent { path: "d:\\a\\X.TXT".into(), user: "AUSTER\\ana".into(), sid: "S-1".into(), utc: 1003 }).unwrap();
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 1);
        let v = idx.all_versions().unwrap();
        assert_eq!(v.iter().find(|x| x.id == id).unwrap().user_name.as_deref(), Some("AUSTER\\ana"));
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 0, "already attributed");
    }

    #[test]
    fn fill_pending_leaves_unknown_without_event_and_stops_after_pending_window() {
        let idx = Index::open_memory().unwrap();
        add(&idx, "D:\\A\\x.txt", 1000);
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 0);
        // An event arriving after PENDING_SECS is no longer looked up.
        idx.add_delete_event(&DeleteEvent { path: "D:\\A\\x.txt".into(), user: "u".into(), sid: "s".into(), utc: 1000 }).unwrap();
        assert_eq!(fill_pending(&idx, 1000 + PENDING_SECS + 1).unwrap(), 0);
        assert_eq!(fill_pending(&idx, 1000 + PENDING_SECS).unwrap(), 1);
    }
}
