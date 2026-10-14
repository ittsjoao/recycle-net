use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use rusqlite::{params, Connection, OptionalExtension};

pub struct DeletedFile {
    pub original_path: String, pub backup_path: String, pub volume: String,
    pub size: i64, pub deleted_utc: i64,
    pub user_sid: Option<String>, pub user_name: Option<String>,
}
/// A non-restored backup as listed in the GUI.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64, pub original_path: String, pub size: i64, pub deleted_utc: i64,
}
/// A 4663 Security event with DELETE access (who removed `path`, and when).
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteEvent {
    pub path: String,
    pub user: String,
    pub sid: String,
    pub utc: i64,
}

/// A non-restored backup with how many backups share its original path.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub id: i64,
    pub original_path: String,
    pub size: i64,
    pub deleted_utc: i64,
    pub user_name: Option<String>,
    pub copies: i64,
}
/// Thread-safe: the single SQLite connection is guarded by a Mutex.
pub struct Index { conn: Mutex<Connection> }

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS deleted_files(
  id INTEGER PRIMARY KEY, original_path TEXT NOT NULL, backup_path TEXT NOT NULL,
  volume TEXT NOT NULL, size INTEGER NOT NULL, deleted_utc INTEGER NOT NULL,
  user_sid TEXT, user_name TEXT, is_restored INTEGER NOT NULL DEFAULT 0, restored_utc INTEGER);
CREATE INDEX IF NOT EXISTS ix_deleted_utc ON deleted_files(deleted_utc);
CREATE INDEX IF NOT EXISTS ix_original_path ON deleted_files(original_path);
CREATE INDEX IF NOT EXISTS ix_volume ON deleted_files(volume);
CREATE INDEX IF NOT EXISTS ix_backup_path ON deleted_files(backup_path);
CREATE TABLE IF NOT EXISTS delete_events(
  path_upper TEXT NOT NULL, user TEXT NOT NULL, sid TEXT NOT NULL, utc INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS ix_de_path ON delete_events(path_upper, utc);
CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);
"#;

impl Index {
    fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn open(path: &Path) -> rusqlite::Result<Index> {
        let conn = Connection::open(path)?;
        // The service and the GUI open the same db; wait out the other's writes.
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Index { conn: Mutex::new(conn) })
    }
    #[cfg(test)]
    pub fn open_memory() -> rusqlite::Result<Index> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Index { conn: Mutex::new(conn) })
    }
    pub fn insert(&self, f: &DeletedFile) -> rusqlite::Result<i64> {
        let conn = self.c();
        conn.execute(
            "INSERT INTO deleted_files(original_path,backup_path,volume,size,deleted_utc,user_sid,user_name)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![f.original_path,f.backup_path,f.volume,f.size,f.deleted_utc,f.user_sid,f.user_name])?;
        Ok(conn.last_insert_rowid())
    }
    /// Insert a row and run `post` inside one transaction; if `post` fails the
    /// insert is rolled back. Used by finalize so the row exists iff the rename did.
    pub fn insert_with<E: From<rusqlite::Error>>(
        &self, f: &DeletedFile, post: impl FnOnce() -> Result<(), E>,
    ) -> Result<i64, E> {
        self.insert_with_commit(f, "COMMIT", post)
    }
    /// Like insert_with but with an injectable COMMIT statement (tests force a commit failure).
    pub(crate) fn insert_with_commit<E: From<rusqlite::Error>>(
        &self, f: &DeletedFile, commit_sql: &str, post: impl FnOnce() -> Result<(), E>,
    ) -> Result<i64, E> {
        let conn = self.c();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let res = (|| -> Result<i64, E> {
            conn.execute(
                "INSERT INTO deleted_files(original_path,backup_path,volume,size,deleted_utc,user_sid,user_name)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![f.original_path,f.backup_path,f.volume,f.size,f.deleted_utc,f.user_sid,f.user_name])?;
            let id = conn.last_insert_rowid();
            post()?;
            Ok(id)
        })();
        match res {
            Ok(id) => match conn.execute_batch(commit_sql) {
                Ok(()) => Ok(id),
                Err(e) => { let _ = conn.execute_batch("ROLLBACK"); Err(e.into()) }
            },
            Err(e) => { let _ = conn.execute_batch("ROLLBACK"); Err(e) }
        }
    }
    pub fn total_size(&self) -> rusqlite::Result<i64> {
        self.c().query_row(
            "SELECT COALESCE(SUM(size),0) FROM deleted_files WHERE is_restored=0", [], |r| r.get(0))
    }
    /// Non-restored rows with deleted_utc strictly before `cutoff` (id, backup_path, size).
    pub fn older_than_unrestored(&self, cutoff: i64) -> rusqlite::Result<Vec<(i64,String,i64)>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,backup_path,size FROM deleted_files WHERE is_restored=0 AND deleted_utc < ?1 ORDER BY deleted_utc ASC, id ASC")?;
        let rows = st.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    pub fn oldest_unrestored(&self, limit: usize) -> rusqlite::Result<Vec<(i64,String,i64)>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,backup_path,size FROM deleted_files WHERE is_restored=0 ORDER BY deleted_utc ASC, id ASC LIMIT ?1")?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    pub fn mark_restored(&self, id: i64) -> rusqlite::Result<()> {
        self.c().execute("UPDATE deleted_files SET is_restored=1, restored_utc=strftime('%s','now') WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn delete_row(&self, id: i64) -> rusqlite::Result<()> {
        self.c().execute("DELETE FROM deleted_files WHERE id=?1", [id])?; Ok(())
    }
    pub fn conn_row(&self, id: i64) -> rusqlite::Result<(String, String)> {
        self.c().query_row(
            "SELECT original_path, backup_path FROM deleted_files WHERE id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?)))
    }
    /// (original_path, backup_path, is_restored)
    pub fn row_info(&self, id: i64) -> rusqlite::Result<(String, String, bool)> {
        self.c().query_row(
            "SELECT original_path, backup_path, is_restored FROM deleted_files WHERE id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))
    }
    #[cfg(test)]
    pub fn deleted_utc(&self, id: i64) -> rusqlite::Result<i64> {
        self.c().query_row("SELECT deleted_utc FROM deleted_files WHERE id=?1", [id], |r| r.get(0))
    }
    /// Non-restored rows whose original path contains `search` (case-insensitive),
    /// newest first, at most `limit`; plus the total number of matches.
    pub fn search_unrestored(&self, search: &str, limit: usize) -> rusqlite::Result<(Vec<Entry>, i64)> {
        let pat = format!("%{}%", search.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let conn = self.c();
        let total = conn.query_row(
            "SELECT COUNT(*) FROM deleted_files WHERE is_restored=0 AND original_path LIKE ?1 ESCAPE '\\'",
            [&pat], |r| r.get(0))?;
        let mut st = conn.prepare(
            "SELECT id,original_path,size,deleted_utc FROM deleted_files
             WHERE is_restored=0 AND original_path LIKE ?1 ESCAPE '\\'
             ORDER BY deleted_utc DESC, id DESC LIMIT ?2")?;
        let rows = st.query_map(params![pat, limit as i64], |r| Ok(Entry {
            id: r.get(0)?, original_path: r.get(1)?, size: r.get(2)?, deleted_utc: r.get(3)?,
        }))?;
        Ok((rows.collect::<rusqlite::Result<_>>()?, total))
    }
    /// Paths are stored uppercased (Rust `to_uppercase`) so lookups ignore case.
    pub fn add_delete_event(&self, e: &DeleteEvent) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO delete_events(path_upper,user,sid,utc) VALUES(?1,?2,?3,?4)",
            params![e.path.to_uppercase(), e.user, e.sid, e.utc],
        )?;
        Ok(())
    }
    pub fn events_for(&self, path: &str, from: i64, to: i64) -> rusqlite::Result<Vec<DeleteEvent>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT path_upper,user,sid,utc FROM delete_events WHERE path_upper=?1 AND utc BETWEEN ?2 AND ?3",
        )?;
        let rows = st.query_map(params![path.to_uppercase(), from, to], |r| {
            Ok(DeleteEvent { path: r.get(0)?, user: r.get(1)?, sid: r.get(2)?, utc: r.get(3)? })
        })?;
        rows.collect()
    }
    pub fn prune_delete_events(&self, before: i64) -> rusqlite::Result<usize> {
        self.c().execute("DELETE FROM delete_events WHERE utc < ?1", [before])
    }
    /// (id, original_path, deleted_utc) of non-restored rows still without a user.
    pub fn pending_attribution(&self, since: i64) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,original_path,deleted_utc FROM deleted_files
             WHERE user_name IS NULL AND is_restored=0 AND deleted_utc >= ?1",
        )?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    pub fn set_user(&self, id: i64, user: &str, sid: &str) -> rusqlite::Result<()> {
        self.c().execute("UPDATE deleted_files SET user_name=?2, user_sid=?3 WHERE id=?1", params![id, user, sid])?;
        Ok(())
    }
    pub fn get_kv(&self, k: &str) -> rusqlite::Result<Option<String>> {
        self.c().query_row("SELECT v FROM kv WHERE k=?1", [k], |r| r.get(0)).optional()
    }
    pub fn set_kv(&self, k: &str, v: &str) -> rusqlite::Result<()> {
        self.c().execute("INSERT OR REPLACE INTO kv(k,v) VALUES(?1,?2)", [k, v])?;
        Ok(())
    }
    /// Every non-restored backup, with its copy count (case-insensitive path).
    pub fn all_versions(&self) -> rusqlite::Result<Vec<Version>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,original_path,size,deleted_utc,user_name,
                    COUNT(*) OVER (PARTITION BY lower(original_path))
             FROM deleted_files WHERE is_restored=0",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Version {
                id: r.get(0)?, original_path: r.get(1)?, size: r.get(2)?, deleted_utc: r.get(3)?,
                user_name: r.get(4)?, copies: r.get(5)?,
            })
        })?;
        rows.collect()
    }
    pub fn count_by_backup_path(&self, backup_path: &str) -> rusqlite::Result<i64> {
        self.c().query_row("SELECT COUNT(*) FROM deleted_files WHERE backup_path=?1",
            [backup_path], |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(ts: i64, size: i64) -> DeletedFile {
        DeletedFile { original_path: format!("D:\\a\\{ts}.txt"),
            backup_path: format!("D:\\bin\\{ts}.txt"), volume: "D:".into(),
            size, deleted_utc: ts, user_sid: None, user_name: None }
    }
    #[test]
    fn backup_path_lookups_are_indexed() {
        // reconcile/adoption query by backup_path for every candidate; without
        // an index each lookup is a full table scan (O(N^2) per reconcile).
        let idx = Index::open_memory().unwrap();
        let conn = idx.c();
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM deleted_files WHERE backup_path = 'x'",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("ix_backup_path"), "query plan: {plan}");
    }
    fn assert_send_sync<T: Send + Sync>() {}
    #[test]
    fn index_is_send_and_sync() {
        assert_send_sync::<Index>();
        assert_send_sync::<std::sync::Arc<Index>>();
    }
    #[test]
    fn insert_with_rolls_back_on_post_failure() {
        let idx = Index::open_memory().unwrap();
        let r: Result<i64, rusqlite::Error> =
            idx.insert_with(&sample(1, 10), || Err(rusqlite::Error::InvalidQuery));
        assert!(r.is_err());
        assert_eq!(idx.total_size().unwrap(), 0);
    }
    #[test]
    fn insert_and_total() {
        let idx = Index::open_memory().unwrap();
        idx.insert(&sample(1, 100)).unwrap();
        idx.insert(&sample(2, 50)).unwrap();
        assert_eq!(idx.total_size().unwrap(), 150);
    }
    #[test]
    fn search_is_substring_newest_first_and_literal() {
        let idx = Index::open_memory().unwrap();
        idx.insert(&sample(1, 1)).unwrap();
        let id2 = idx.insert(&sample(2, 1)).unwrap();
        let id3 = idx.insert(&sample(3, 1)).unwrap();
        let mut f = sample(4, 1);
        f.original_path = "D:\\x\\100%_done.txt".into();
        let id4 = idx.insert(&f).unwrap();
        idx.mark_restored(id3).unwrap();
        let (rows, total) = idx.search_unrestored("", 2).unwrap();
        assert_eq!(total, 3);
        assert_eq!(rows.iter().map(|e| e.id).collect::<Vec<_>>(), [id4, id2]);
        let (rows, total) = idx.search_unrestored("d:\\A\\2", 10).unwrap();
        assert_eq!((rows.len(), total), (1, 1));
        assert_eq!(idx.search_unrestored("%_", 10).unwrap().1, 1);
    }
    #[test]
    fn oldest_first_and_excludes_restored() {
        let idx = Index::open_memory().unwrap();
        let id1 = idx.insert(&sample(1, 100)).unwrap();
        idx.insert(&sample(2, 50)).unwrap();
        idx.mark_restored(id1).unwrap();
        assert_eq!(idx.total_size().unwrap(), 50);
        let old = idx.oldest_unrestored(10).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].0, id1 + 1);
    }
    fn ev(path: &str, user: &str, utc: i64) -> DeleteEvent {
        DeleteEvent { path: path.into(), user: user.into(), sid: "S-1-5-21-1".into(), utc }
    }
    #[test]
    fn events_match_case_insensitively_and_by_time_range() {
        let idx = Index::open_memory().unwrap();
        idx.add_delete_event(&ev("d:\\dados\\A.txt", "AUSTER\\ana", 100)).unwrap();
        idx.add_delete_event(&ev("D:\\Dados\\a.TXT", "AUSTER\\bia", 500)).unwrap();
        let got = idx.events_for("D:\\DADOS\\A.TXT", 0, 200).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].user, "AUSTER\\ana");
        assert_eq!(idx.prune_delete_events(200).unwrap(), 1);
        assert_eq!(idx.events_for("D:\\Dados\\a.txt", 0, 1000).unwrap().len(), 1);
    }
    #[test]
    fn pending_attribution_and_set_user() {
        let idx = Index::open_memory().unwrap();
        let old = idx.insert(&sample(10, 1)).unwrap();
        let new = idx.insert(&sample(1000, 1)).unwrap();
        assert_eq!(idx.pending_attribution(500).unwrap().iter().map(|r| r.0).collect::<Vec<_>>(), [new]);
        idx.set_user(new, "AUSTER\\ana", "S-1").unwrap();
        assert!(idx.pending_attribution(0).unwrap().iter().all(|r| r.0 == old));
    }
    #[test]
    fn kv_round_trips() {
        let idx = Index::open_memory().unwrap();
        assert_eq!(idx.get_kv("a").unwrap(), None);
        idx.set_kv("a", "1").unwrap();
        idx.set_kv("a", "2").unwrap();
        assert_eq!(idx.get_kv("a").unwrap().as_deref(), Some("2"));
    }
    #[test]
    fn all_versions_counts_copies_per_path_and_skips_restored() {
        let idx = Index::open_memory().unwrap();
        let mut a1 = sample(1, 10); a1.original_path = "D:\\x\\a.txt".into();
        let mut a2 = sample(2, 20); a2.original_path = "d:\\X\\A.txt".into();
        let b = sample(3, 30);
        let r = sample(4, 40);
        idx.insert(&a1).unwrap();
        idx.insert(&a2).unwrap();
        idx.insert(&b).unwrap();
        let rid = idx.insert(&r).unwrap();
        idx.mark_restored(rid).unwrap();
        let mut v = idx.all_versions().unwrap();
        v.sort_by_key(|x| x.deleted_utc);
        assert_eq!(v.iter().map(|x| x.copies).collect::<Vec<_>>(), [2, 2, 1]);
        assert_eq!(v[0].user_name, None);
    }
}
