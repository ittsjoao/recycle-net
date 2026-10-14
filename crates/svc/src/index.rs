use std::path::Path;
use rusqlite::{Connection, params};

pub struct DeletedFile {
    pub original_path: String, pub backup_path: String, pub volume: String,
    pub size: i64, pub deleted_utc: i64,
    pub user_sid: Option<String>, pub user_name: Option<String>,
}
pub struct Index { conn: Connection }

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS deleted_files(
  id INTEGER PRIMARY KEY, original_path TEXT NOT NULL, backup_path TEXT NOT NULL,
  volume TEXT NOT NULL, size INTEGER NOT NULL, deleted_utc INTEGER NOT NULL,
  user_sid TEXT, user_name TEXT, is_restored INTEGER NOT NULL DEFAULT 0, restored_utc INTEGER);
CREATE INDEX IF NOT EXISTS ix_deleted_utc ON deleted_files(deleted_utc);
CREATE INDEX IF NOT EXISTS ix_original_path ON deleted_files(original_path);
CREATE INDEX IF NOT EXISTS ix_volume ON deleted_files(volume);
"#;

impl Index {
    pub fn open(path: &Path) -> rusqlite::Result<Index> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Index { conn })
    }
    #[cfg(test)]
    pub fn open_memory() -> rusqlite::Result<Index> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Index { conn })
    }
    pub fn insert(&self, f: &DeletedFile) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO deleted_files(original_path,backup_path,volume,size,deleted_utc,user_sid,user_name)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![f.original_path,f.backup_path,f.volume,f.size,f.deleted_utc,f.user_sid,f.user_name])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn total_size(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(size),0) FROM deleted_files WHERE is_restored=0", [], |r| r.get(0))
    }
    pub fn oldest_unrestored(&self, limit: usize) -> rusqlite::Result<Vec<(i64,String,i64)>> {
        let mut st = self.conn.prepare(
            "SELECT id,backup_path,size FROM deleted_files WHERE is_restored=0 ORDER BY deleted_utc ASC LIMIT ?1")?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    pub fn mark_restored(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE deleted_files SET is_restored=1, restored_utc=strftime('%s','now') WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn delete_row(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM deleted_files WHERE id=?1", [id])?; Ok(())
    }
    pub fn conn_row(&self, id: i64) -> rusqlite::Result<(String, String)> {
        self.conn.query_row(
            "SELECT original_path, backup_path FROM deleted_files WHERE id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?)))
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
    fn insert_and_total() {
        let idx = Index::open_memory().unwrap();
        idx.insert(&sample(1, 100)).unwrap();
        idx.insert(&sample(2, 50)).unwrap();
        assert_eq!(idx.total_size().unwrap(), 150);
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
}
