//! The bin as a folder tree: protected folders as roots, below them only the
//! subfolders that contain deleted files, with count/size/last-deleted totals.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeInfo {
    pub count: usize,
    pub size: i64,
    pub last: i64,
}

/// Lookup key: case-insensitive, no trailing separator ("" is the whole bin).
pub fn key(p: &str) -> String {
    p.trim_end_matches('\\').to_lowercase()
}

/// Folder of a path ("" when there is none).
pub fn parent(p: &str) -> &str {
    p.rfind('\\').map(|i| &p[..i]).unwrap_or("")
}

#[derive(Debug, Default, Clone)]
pub struct Tree {
    roots: Vec<String>,
    display: BTreeMap<String, String>,
    info: BTreeMap<String, NodeInfo>,
    children: BTreeMap<String, BTreeSet<String>>,
}

impl Tree {
    /// `roots`: protected folders. `files`: (original_path, size, deleted_utc).
    /// A file under no root goes under its drive (`D:`), so backups of a folder
    /// removed from the config stay reachable.
    pub fn build(roots: &[String], files: &[(String, i64, i64)]) -> Tree {
        let mut t = Tree::default();
        for r in roots {
            t.add_root(r.trim_end_matches('\\'));
        }
        for (path, size, utc) in files {
            let root = t.root_of(path).unwrap_or_else(|| {
                let drive = path.split('\\').next().unwrap_or("").to_string();
                t.add_root(&drive);
                drive
            });
            let root_key = key(&root);
            let mut dir = parent(path).to_string();
            loop {
                let k = key(&dir);
                t.display.entry(k.clone()).or_insert_with(|| dir.clone());
                let e = t.info.entry(k.clone()).or_default();
                e.count += 1;
                e.size += size;
                e.last = e.last.max(*utc);
                if k == root_key || dir.is_empty() {
                    break;
                }
                let up = parent(&dir).to_string();
                t.children.entry(key(&up)).or_default().insert(k);
                dir = up;
            }
            let all = t.info.entry(String::new()).or_default();
            all.count += 1;
            all.size += size;
            all.last = all.last.max(*utc);
        }
        t
    }

    fn add_root(&mut self, r: &str) {
        let k = key(r);
        if self.roots.iter().any(|x| key(x) == k) {
            return;
        }
        self.roots.push(r.to_string());
        self.display.insert(k, r.to_string());
    }

    /// Subfolders with deleted files ("" lists the roots, in config order).
    pub fn children(&self, dir: &str) -> Vec<String> {
        if dir.is_empty() {
            return self.roots.clone();
        }
        let mut v: Vec<String> = self
            .children
            .get(&key(dir))
            .into_iter()
            .flatten()
            .filter(|k| !self.roots.iter().any(|r| &key(r) == *k))
            .map(|k| self.display[k].clone())
            .collect();
        v.sort_by_key(|d| d.to_lowercase());
        v
    }

    pub fn info(&self, dir: &str) -> NodeInfo {
        self.info.get(&key(dir)).copied().unwrap_or_default()
    }

    pub fn is_root(&self, dir: &str) -> bool {
        self.roots.iter().any(|r| key(r) == key(dir))
    }

    /// The deepest root containing `path` (a configured folder beats its drive).
    pub fn root_of(&self, path: &str) -> Option<String> {
        let k = key(path);
        self.roots
            .iter()
            .filter(|r| {
                let rk = key(r);
                k == rk || k.starts_with(&format!("{rk}\\"))
            })
            .max_by_key(|r| r.len())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, size: i64, t: i64) -> (String, i64, i64) {
        (p.to_string(), size, t)
    }

    fn sample() -> Tree {
        Tree::build(
            &[r"D:\Dados\Contabil".into(), r"D:\Dados\Fiscal".into(), r"E:\".into()],
            &[
                f(r"D:\Dados\Contabil\Clientes\Acme\a.xlsx", 10, 5),
                f(r"D:\Dados\Contabil\Clientes\Beta\b.xlsx", 20, 7),
                f(r"D:\Dados\Contabil\c.xlsx", 1, 9),
                f(r"E:\x\y.txt", 3, 2),
            ],
        )
    }

    #[test]
    fn roots_keep_config_order_and_children_only_with_deleted_files() {
        let t = sample();
        assert_eq!(t.children(""), [r"D:\Dados\Contabil", r"D:\Dados\Fiscal", r"E:"]);
        assert_eq!(t.children(r"D:\Dados\Contabil"), [r"D:\Dados\Contabil\Clientes"]);
        assert_eq!(t.children(r"D:\Dados\Contabil\Clientes"), [r"D:\Dados\Contabil\Clientes\Acme", r"D:\Dados\Contabil\Clientes\Beta"]);
        assert!(t.children(r"D:\Dados\Fiscal").is_empty());
    }

    #[test]
    fn info_aggregates_count_size_and_last() {
        let t = sample();
        assert_eq!(t.info(""), NodeInfo { count: 4, size: 34, last: 9 });
        assert_eq!(t.info(r"D:\Dados\Contabil"), NodeInfo { count: 3, size: 31, last: 9 });
        assert_eq!(t.info(r"D:\Dados\Contabil\Clientes"), NodeInfo { count: 2, size: 30, last: 7 });
        assert_eq!(t.info(r"D:\Dados\Fiscal"), NodeInfo::default());
    }

    #[test]
    fn tree_is_case_insensitive() {
        let t = Tree::build(&[r"D:\Dados".into()], &[f(r"d:\dados\SUB\a.txt", 1, 1), f(r"D:\Dados\sub\b.txt", 1, 2)]);
        assert_eq!(t.children(r"D:\DADOS").len(), 1);
        assert_eq!(t.info(r"d:\dados\sub").count, 2);
    }

    #[test]
    fn orphans_go_under_drive_root() {
        let t = Tree::build(&[r"D:\Dados".into()], &[f(r"D:\Antiga\a.txt", 1, 1), f(r"D:\Dados\b.txt", 1, 2)]);
        assert_eq!(t.children(""), [r"D:\Dados", "D:"]);
        assert_eq!(t.children("D:"), [r"D:\Antiga"]);
        assert_eq!(t.info("D:").count, 1, "configured folder is not counted twice");
        assert_eq!(t.root_of(r"D:\Antiga\a.txt").as_deref(), Some("D:"));
        assert_eq!(t.root_of(r"D:\Dados\b.txt").as_deref(), Some(r"D:\Dados"));
        assert!(t.is_root("d:"));
    }
}
