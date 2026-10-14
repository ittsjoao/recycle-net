use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Fixed location read by the service and written by the GUI.
pub const CONFIG_PATH: &str = r"C:\ProgramData\rnetrbin\config.toml";

/// One protected folder: a deleted file under `path` (subfolders included) goes to
/// the bin when its name matches some `include` mask and no `exclude` mask.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FolderRule {
    pub path: PathBuf,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl FolderRule {
    /// Rules offered when a folder is added in the GUI.
    pub fn with_defaults(path: PathBuf) -> FolderRule {
        FolderRule { path, include: vec!["*.*".into()], exclude: vec!["*.tmp".into(), "~$*".into()] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    Rust,
    Branco,
    Preto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub recycle_dir: PathBuf,
    pub db_path: PathBuf,
    pub max_bin_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub theme: ThemeName,
    /// Last: TOML writes the `[[folders]]` tables after the plain keys.
    #[serde(default)]
    pub folders: Vec<FolderRule>,
}

impl Default for Config {
    /// Starting values offered by the GUI before the first install.
    fn default() -> Self {
        Config {
            recycle_dir: r"D:\RecycleBinNet".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            max_bin_bytes: 10 * 1024 * 1024 * 1024,
            max_age_days: None,
            theme: ThemeName::Rust,
            folders: vec![],
        }
    }
}

/// `child` is `parent` or lies under it (case-insensitive, `X:\` roots included).
pub fn nests(parent: &Path, child: &Path) -> bool {
    let p = parent.to_string_lossy().to_lowercase();
    let p = p.trim_end_matches('\\');
    let c = child.to_string_lossy().to_lowercase();
    let c = c.trim_end_matches('\\');
    c == p || c.starts_with(&format!("{p}\\"))
}

/// Driver masks for one rule mask, anchored on folder `f` (spec section 2).
fn anchored(f: &Path, m: &str) -> Vec<String> {
    let f = f.to_string_lossy().trim_end_matches('\\').to_uppercase();
    let m = m.to_uppercase();
    if m == "*" || m == "*.*" {
        vec![format!("{f}\\*")]
    } else if m.starts_with('*') {
        // The leading `*` already crosses `\`, so it covers subfolders.
        vec![format!("{f}\\{m}")]
    } else {
        vec![format!("{f}\\{m}"), format!("{f}\\*\\{m}")]
    }
}

/// The netrbin driver matches each mask with FsRtlIsNameInExpression(IgnoreCase)
/// against the deleted file's full DOS path (`D:\dir\file.ext`): masks must be
/// uppercase and anchored on the whole path. It keeps a file that matches any
/// include and no exclude. Returns (includes, excludes); the recycle dir and the
/// db are always excluded so the service never captures its own writes.
pub fn driver_masks(c: &Config) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for f in &c.folders {
        includes.extend(f.include.iter().flat_map(|m| anchored(&f.path, m)));
        excludes.extend(f.exclude.iter().flat_map(|m| anchored(&f.path, m)));
    }
    excludes.push(format!("{}\\*", c.recycle_dir.to_string_lossy()).to_uppercase());
    excludes.push(format!("{}*", c.db_path.to_string_lossy()).to_uppercase());
    (includes, excludes)
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("parse: {0}")] Parse(#[from] toml::de::Error),
    #[error("write: {0}")] Write(#[from] toml::ser::Error),
    #[error("invalid: {0}")] Invalid(String),
}

/// Marker file proving a directory is owned by rnetrbin. Quota eviction only
/// ever deletes indexed backups, but the sentinel also stops the service from
/// starting against a populated directory it did not create (a drive root, a
/// share root, or another tool's bin).
pub const SENTINEL: &str = ".rnetrbin-root";

/// Create `dir` if needed and mark it as rnetrbin's. A directory that already
/// exists is accepted only if it is marked, or empty (then it gets marked).
pub fn ensure_dedicated_dir(dir: &Path) -> Result<(), ConfigError> {
    std::fs::create_dir_all(dir)?;
    let sentinel = dir.join(SENTINEL);
    if sentinel.exists() {
        return Ok(());
    }
    if std::fs::read_dir(dir)?.next().is_some() {
        return Err(ConfigError::Invalid(format!(
            "recycle_dir {} is not empty and has no {SENTINEL} marker; refusing to manage it \
             (create the marker manually only if this directory is dedicated to rnetrbin)",
            dir.display()
        )));
    }
    std::fs::write(&sentinel, b"rnetrbin recycle root\n")?;
    Ok(())
}

/// Normalize a configured path to `X:\a\b` form: forward slashes become
/// backslashes, the drive letter is uppercased and trailing separators removed.
/// Only plain drive-absolute paths are accepted (no UNC, no `\\?\`, no
/// drive-relative `X:a`, no rootless `\a`). With `allow_root` false a bare
/// drive root (`X:\`) is rejected.
fn normalize_drive_path(p: &Path, what: &str, allow_root: bool) -> Result<PathBuf, ConfigError> {
    let s = p.to_string_lossy().replace('/', "\\");
    let bad = |why: &str| ConfigError::Invalid(format!("{what} {s:?}: {why}"));
    if s.starts_with("\\\\") {
        return Err(bad("UNC and \\\\?\\ paths are not supported; use a local X:\\ path"));
    }
    let b = s.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return Err(bad("must be a drive-absolute path like D:\\RecycleBinNet"));
    }
    let rest = s[3..].trim_end_matches('\\');
    if rest.is_empty() && !allow_root {
        return Err(bad("must not be a drive root"));
    }
    if !rest.is_empty() && rest.split('\\').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(bad("must not contain empty, '.' or '..' components"));
    }
    Ok(PathBuf::from(format!("{}:\\{rest}", b[0].to_ascii_uppercase() as char)))
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let mut c: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
        c.normalize()?;
        c.validate()?;
        Ok(c)
    }
    /// Normalize, validate and write; an invalid config is never written.
    pub fn save(&mut self, path: &Path) -> Result<(), ConfigError> {
        self.normalize()?;
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string(self)?)?;
        Ok(())
    }
    /// Rewrite `recycle_dir` and `db_path` into canonical `X:\...` form; this is
    /// the exact string later sent to the driver.
    pub fn normalize(&mut self) -> Result<(), ConfigError> {
        self.recycle_dir = normalize_drive_path(&self.recycle_dir, "recycle_dir", false)?;
        self.db_path = normalize_drive_path(&self.db_path, "db_path", false)?;
        // A drive root (`D:\`) protects the whole volume.
        for f in &mut self.folders {
            f.path = normalize_drive_path(&f.path, "folder", true)?;
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.folders.is_empty() {
            return Err(ConfigError::Invalid("at least one protected folder is required".into()));
        }
        for f in &self.folders {
            normalize_drive_path(&f.path, "folder", true)?;
            let shown = f.path.display();
            if f.include.is_empty() {
                return Err(ConfigError::Invalid(format!("folder {shown}: include must not be empty")));
            }
            for m in f.include.iter().chain(f.exclude.iter()) {
                if m.is_empty() || m.contains('\0') || m.contains('\\') {
                    return Err(ConfigError::Invalid(format!(
                        "folder {shown}: invalid mask {m:?} (empty, NUL or backslash; masks are file names)"
                    )));
                }
            }
        }
        // The driver adds every folder's masks into one list, so a nested pair
        // would leak one folder's exclusions into the other.
        for (i, a) in self.folders.iter().enumerate() {
            for b in &self.folders[i + 1..] {
                if nests(&a.path, &b.path) || nests(&b.path, &a.path) {
                    return Err(ConfigError::Invalid(format!(
                        "folders {} and {} overlap; protect only one of them",
                        a.path.display(),
                        b.path.display()
                    )));
                }
            }
        }
        // Re-check the canonical form (validate may be called without normalize).
        if normalize_drive_path(&self.recycle_dir, "recycle_dir", false)? != self.recycle_dir {
            return Err(ConfigError::Invalid("recycle_dir is not normalized".into()));
        }
        normalize_drive_path(&self.db_path, "db_path", false)?;
        if self.max_bin_bytes == 0 {
            return Err(ConfigError::Invalid("max_bin_bytes must be > 0".into()));
        }
        if self.max_age_days == Some(0) {
            return Err(ConfigError::Invalid("max_age_days must be > 0 when set".into()));
        }
        // The driver would capture the service's own db/deletes if the db lived in the recycle dir.
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        if lower(&self.db_path).starts_with(lower(&self.recycle_dir)) {
            return Err(ConfigError::Invalid("db_path must not be inside recycle_dir".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_toml_with_folder_tables_and_theme() {
        let t = r#"
recycle_dir = 'E:\Lixeira'
db_path = 'C:\ProgramData\rnetrbin\index.db'
max_bin_bytes = 10737418240
theme = "preto"

[[folders]]
path = 'D:\Dados\Contabil'
include = ["*.*"]
exclude = ["*.bak"]

[[folders]]
path = 'D:\Dados\Fiscal'
include = ["*.xml", "*.pdf"]
"#;
        let c: Config = toml::from_str(t).unwrap();
        assert_eq!(c.theme, ThemeName::Preto);
        assert_eq!(c.folders.len(), 2);
        assert_eq!(c.folders[0].exclude, ["*.bak"]);
        assert!(c.folders[1].exclude.is_empty());
    }
    #[test]
    fn theme_defaults_to_rust_and_unknown_theme_fails() {
        let base = "recycle_dir = 'E:\\L'\ndb_path = 'C:\\x\\i.db'\nmax_bin_bytes = 1\n";
        let c: Config = toml::from_str(base).unwrap();
        assert_eq!(c.theme, ThemeName::Rust);
        assert!(toml::from_str::<Config>(&format!("{base}theme = \"verde\"\n")).is_err());
    }
    fn rule(path: &str, include: &[&str], exclude: &[&str]) -> FolderRule {
        FolderRule {
            path: path.into(),
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn ok_cfg() -> Config {
        Config {
            recycle_dir: "D:\\x".into(), db_path: "D:\\db\\i.db".into(), max_bin_bytes: 1,
            max_age_days: None, theme: ThemeName::Rust,
            folders: vec![rule("D:\\Dados", &["*.*"], &[])],
        }
    }
    #[test]
    fn empty_folders_is_invalid() {
        let mut c = ok_cfg();
        c.folders = vec![];
        assert!(c.validate().is_err());
    }
    #[test]
    fn folder_rules_are_validated() {
        let mut c = ok_cfg(); c.folders = vec![rule("D:\\A", &[], &[])];
        assert!(c.validate().is_err(), "empty include");
        for bad in ["", "a\0b", "sub\\*.txt"] {
            let mut c = ok_cfg(); c.folders = vec![rule("D:\\A", &["*.*"], &[bad])];
            assert!(c.validate().is_err(), "accepted mask {bad:?}");
        }
    }
    #[test]
    fn nested_folders_are_rejected_in_any_order_and_case() {
        for (a, b) in [("D:\\Dados", "d:\\dados\\Fiscal"), ("D:\\Dados\\Fiscal", "D:\\Dados"), ("E:\\", "e:\\x"), ("D:\\A", "d:\\a")] {
            let mut c = ok_cfg();
            c.folders = vec![rule(a, &["*.*"], &[]), rule(b, &["*.*"], &[])];
            c.normalize().unwrap();
            assert!(c.validate().is_err(), "accepted {a} + {b}");
        }
        let mut c = ok_cfg();
        c.folders = vec![rule("D:\\Dados", &["*.*"], &[]), rule("D:\\DadosOld", &["*.*"], &[])];
        c.validate().unwrap();
    }
    #[test]
    fn driver_masks_follow_the_spec_table() {
        let mut c = Config {
            recycle_dir: r"E:\Lixeira".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            folders: vec![
                rule(r"d:/Dados/Contabil/", &["*.*"], &["*.bak", "~$*"]),
                rule(r"D:\Dados\Fiscal", &["*.xml", "Relatorio.pdf"], &[]),
                rule(r"F:\", &["*"], &[]),
            ],
            ..ok_cfg()
        };
        c.normalize().unwrap();
        c.validate().unwrap();
        let (inc, exc) = driver_masks(&c);
        assert_eq!(inc, [
            r"D:\DADOS\CONTABIL\*",
            r"D:\DADOS\FISCAL\*.XML",
            r"D:\DADOS\FISCAL\RELATORIO.PDF", r"D:\DADOS\FISCAL\*\RELATORIO.PDF",
            r"F:\*",
        ]);
        assert_eq!(exc, [
            r"D:\DADOS\CONTABIL\*.BAK",
            r"D:\DADOS\CONTABIL\~$*", r"D:\DADOS\CONTABIL\*\~$*",
            r"E:\LIXEIRA\*", r"C:\PROGRAMDATA\RNETRBIN\INDEX.DB*",
        ]);
    }
    #[test]
    fn relative_recycle_dir_invalid() {
        let mut c = ok_cfg(); c.recycle_dir = "rel\\x".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn zero_quota_invalid() {
        let mut c = ok_cfg(); c.max_bin_bytes = 0;
        assert!(c.validate().is_err());
    }
    #[test]
    fn db_inside_recycle_dir_invalid() {
        let mut c = ok_cfg(); c.db_path = "d:\\X\\sub\\i.db".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn recycle_dir_rejects_roots_unc_and_verbatim() {
        for bad in ["D:\\", "D:", "\\\\server\\share\\bin", "\\\\?\\D:\\bin", "\\bin", "D:bin"] {
            let mut c = ok_cfg(); c.recycle_dir = bad.into();
            assert!(c.normalize().and_then(|_| c.validate()).is_err(), "accepted {bad:?}");
        }
    }
    #[test]
    fn normalize_fixes_slashes_and_trailing_separator() {
        let mut c = ok_cfg();
        c.recycle_dir = "d:/RecycleBinNet/".into();
        c.db_path = "D:/rnetrbin/index.db".into();
        c.normalize().unwrap();
        c.validate().unwrap();
        assert_eq!(c.recycle_dir, PathBuf::from("D:\\RecycleBinNet"));
        assert_eq!(c.db_path, PathBuf::from("D:\\rnetrbin\\index.db"));
    }
    #[test]
    fn db_path_must_be_drive_absolute() {
        let mut c = ok_cfg(); c.db_path = "index.db".into();
        assert!(c.normalize().and_then(|_| c.validate()).is_err());
    }
    #[test]
    fn zero_max_age_is_invalid() {
        let mut c = ok_cfg(); c.max_age_days = Some(0);
        assert!(c.validate().is_err());
    }
    #[test]
    fn dedicated_dir_created_and_marked_when_new() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("bin");
        ensure_dedicated_dir(&d).unwrap();
        assert!(d.join(SENTINEL).exists());
        ensure_dedicated_dir(&d).unwrap(); // idempotent
    }
    #[test]
    fn dedicated_dir_marks_existing_empty_dir() {
        let t = tempfile::tempdir().unwrap();
        ensure_dedicated_dir(t.path()).unwrap();
        assert!(t.path().join(SENTINEL).exists());
    }
    #[test]
    fn dedicated_dir_refuses_populated_unmarked_dir() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("user-data.xlsx"), b"x").unwrap();
        assert!(ensure_dedicated_dir(t.path()).is_err());
        assert!(!t.path().join(SENTINEL).exists());
    }
}
