//! What the list shows for the current folder: its subfolders (from the tree)
//! and the files directly in it; or, with a search, everything below it.
use std::collections::HashMap;

use super::tree::{key, parent, NodeInfo, Tree};
use crate::index::Version;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortCol {
    Name,
    Copies,
    User,
    Size,
    Deleted,
}

pub struct View<'a> {
    /// Current folder ("" = the bin's root).
    pub cur: &'a str,
    /// Non-empty: search `cur` and everything below it.
    pub query: &'a str,
    /// false: only the newest backup of each original path.
    pub all_copies: bool,
    /// Some: every backup of this one original path ("Ver cópias").
    pub copies_of: Option<&'a str>,
    /// (column, ascending)
    pub sort: (SortCol, bool),
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirRow {
    pub path: String,
    pub name: String,
    pub info: NodeInfo,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub dirs: Vec<DirRow>,
    pub files: Vec<Version>,
    /// Files before truncation to `limit`.
    pub total_files: usize,
}

pub fn build(tree: &Tree, rows: &[Version], v: &View) -> Listing {
    let q = v.query.trim().to_lowercase();
    let cur = key(v.cur);
    let below = |p: &str| cur.is_empty() || key(p).starts_with(&format!("{cur}\\"));
    let mut files: Vec<Version> = rows
        .iter()
        .filter(|r| match (v.copies_of, q.is_empty()) {
            (Some(p), _) => key(&r.original_path) == key(p),
            (None, false) => {
                below(&r.original_path)
                    && (r.original_path.to_lowercase().contains(&q)
                        || r.user_name.as_deref().unwrap_or("").to_lowercase().contains(&q))
            }
            (None, true) => key(parent(&r.original_path)) == cur,
        })
        .cloned()
        .collect();
    if !v.all_copies && v.copies_of.is_none() {
        let mut newest: HashMap<String, (i64, i64)> = HashMap::new();
        for f in &files {
            let e = newest.entry(key(&f.original_path)).or_insert((f.deleted_utc, f.id));
            *e = (*e).max((f.deleted_utc, f.id));
        }
        files.retain(|f| newest[&key(&f.original_path)] == (f.deleted_utc, f.id));
    }
    sort(&mut files, v.sort);
    let total_files = files.len();
    files.truncate(v.limit);
    let dirs = if q.is_empty() && v.copies_of.is_none() {
        tree.children(v.cur)
            .into_iter()
            .map(|p| DirRow {
                name: if v.cur.is_empty() { p.clone() } else { split_path(&p).0.to_string() },
                info: tree.info(&p),
                path: p,
            })
            .collect()
    } else {
        Vec::new()
    };
    Listing { dirs, files, total_files }
}

fn sort(files: &mut [Version], (col, asc): (SortCol, bool)) {
    files.sort_by(|a, b| {
        let o = match col {
            SortCol::Name => split_path(&a.original_path).0.to_lowercase().cmp(&split_path(&b.original_path).0.to_lowercase()),
            SortCol::Copies => a.copies.cmp(&b.copies),
            SortCol::User => a.user_name.as_deref().unwrap_or("~").to_lowercase().cmp(&b.user_name.as_deref().unwrap_or("~").to_lowercase()),
            SortCol::Size => a.size.cmp(&b.size),
            SortCol::Deleted => (a.deleted_utc, a.id).cmp(&(b.deleted_utc, b.id)),
        };
        if asc { o } else { o.reverse() }
    });
}

/// (file name, folder) of a Windows path.
pub fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('\\') {
        Some(i) => (&p[i + 1..], &p[..i]),
        None => (p, ""),
    }
}

pub fn fmt_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    match b {
        b if b >= GB => format!("{:.1} GB", b / GB),
        b if b >= 1024.0 * 1024.0 => format!("{:.1} MB", b / 1024.0 / 1024.0),
        b if b >= 1024.0 => format!("{:.0} KB", b / 1024.0),
        b => format!("{b} B"),
    }
}

pub fn fmt_time(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%d/%m/%Y %H:%M").to_string(),
        chrono::LocalResult::None => unix.to_string(),
    }
}

/// Masks typed in the GUI are separated by `;` (names may contain spaces).
pub fn split_masks(s: &str) -> Vec<String> {
    s.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: i64, p: &str, utc: i64, user: Option<&str>, copies: i64) -> Version {
        Version { id, original_path: p.into(), size: id * 10, deleted_utc: utc, user_name: user.map(String::from), copies }
    }

    fn rows() -> Vec<Version> {
        vec![
            v(1, r"D:\Dados\a.txt", 5, Some("AUSTER\\ana"), 2),
            v(2, r"D:\Dados\A.TXT", 9, None, 2),
            v(3, r"D:\Dados\Sub\b.txt", 7, Some("AUSTER\\bia"), 1),
            v(4, r"D:\Dados\Sub\Deep\c.txt", 3, None, 1),
        ]
    }

    fn view<'a>(cur: &'a str, query: &'a str) -> View<'a> {
        View { cur, query, all_copies: true, copies_of: None, sort: (SortCol::Deleted, false), limit: 1000 }
    }

    fn tree(rows: &[Version]) -> Tree {
        let files: Vec<_> = rows.iter().map(|r| (r.original_path.clone(), r.size, r.deleted_utc)).collect();
        Tree::build(&[r"D:\Dados".into()], &files)
    }

    fn ids(l: &Listing) -> Vec<i64> {
        l.files.iter().map(|f| f.id).collect()
    }

    #[test]
    fn shows_direct_files_and_subfolders_only() {
        let r = rows();
        let l = build(&tree(&r), &r, &view(r"D:\Dados", ""));
        assert_eq!(ids(&l), [2, 1]);
        assert_eq!(l.dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Sub"]);
        assert_eq!(l.dirs[0].info.count, 2);
        let root = build(&tree(&r), &r, &view("", ""));
        assert!(root.files.is_empty());
        assert_eq!(root.dirs[0].name, r"D:\Dados");
    }

    #[test]
    fn search_looks_below_current_folder_by_path_or_user() {
        let r = rows();
        let t = tree(&r);
        assert_eq!(ids(&build(&t, &r, &view(r"D:\Dados", "c.txt"))), [4]);
        assert_eq!(ids(&build(&t, &r, &view(r"D:\Dados\Sub", "bia"))), [3]);
        assert!(build(&t, &r, &view(r"D:\Dados\Sub", "bia")).dirs.is_empty());
    }

    #[test]
    fn latest_copy_only_and_copies_of() {
        let r = rows();
        let t = tree(&r);
        let mut v1 = view(r"D:\Dados", "");
        v1.all_copies = false;
        assert_eq!(ids(&build(&t, &r, &v1)), [2]);
        let mut v2 = view(r"D:\Dados\Sub", "");
        v2.copies_of = Some(r"d:\dados\a.txt");
        let l = build(&t, &r, &v2);
        assert_eq!(ids(&l), [2, 1]);
        assert!(l.dirs.is_empty());
    }

    #[test]
    fn sorts_by_each_column() {
        let r = rows();
        let t = tree(&r);
        let mut q = view("", "txt");
        q.sort = (SortCol::Size, true);
        assert_eq!(ids(&build(&t, &r, &q)), [1, 2, 3, 4]);
        q.sort = (SortCol::User, true);
        assert_eq!(ids(&build(&t, &r, &q))[..2], [1, 3]);
        q.sort = (SortCol::Name, false);
        assert_eq!(ids(&build(&t, &r, &q))[0], 4);
    }

    #[test]
    fn listing_truncates_and_reports_total() {
        let many: Vec<Version> = (1..=1500).map(|i| v(i, &format!(r"D:\Dados\f{i}.txt"), i, None, 1)).collect();
        let mut q = view(r"D:\Dados", "");
        q.limit = 1000;
        let l = build(&tree(&many), &many, &q);
        assert_eq!(l.files.len(), 1000);
        assert_eq!(l.total_files, 1500);
        assert_eq!(l.files[0].id, 1500, "newest first");
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(split_path(r"D:\a\b\c.txt"), ("c.txt", r"D:\a\b"));
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(split_masks(" *.docx ;; Relatório *.xlsx "), ["*.docx", "Relatório *.xlsx"]);
    }
}
