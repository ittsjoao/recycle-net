use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Fixed location read by the service and written by the GUI.
pub const CONFIG_PATH: &str = r"C:\ProgramData\rnetrbin\config.toml";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub recycle_dir: PathBuf,
    pub db_path: PathBuf,
    /// Protected folders (subfolders included); deletes elsewhere are ignored.
    pub folders: Vec<PathBuf>,
    /// File-name masks (`*.tmp`, `~$*`) or full-path masks (with `\`) never captured.
    pub excludes: Vec<String>,
    pub max_bin_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

impl Default for Config {
    /// Starting values offered by the GUI before the first install.
    fn default() -> Self {
        Config {
            recycle_dir: r"D:\RecycleBinNet".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            folders: vec![],
            excludes: vec!["*.tmp".into(), "~$*".into()],
            max_bin_bytes: 10 * 1024 * 1024 * 1024,
            max_age_days: None,
        }
    }
}

/// The netrbin driver matches each mask with FsRtlIsNameInExpression(IgnoreCase)
/// against the deleted file's full DOS path (`D:\dir\file.ext`): masks must be
/// uppercase and anchored on the whole path. Returns (includes, excludes).
/// The recycle dir and the db are always excluded so the service never
/// captures its own writes.
pub fn driver_masks(c: &Config) -> (Vec<String>, Vec<String>) {
    let up = |s: String| s.to_uppercase();
    let includes = c
        .folders
        .iter()
        .map(|f| up(format!("{}\\*", f.to_string_lossy().trim_end_matches('\\'))))
        .collect();
    let mut excludes: Vec<String> = c
        .excludes
        .iter()
        .map(|m| up(if m.contains('\\') { m.clone() } else { format!("*\\{m}") }))
        .collect();
    excludes.push(up(format!("{}\\*", c.recycle_dir.to_string_lossy())));
    excludes.push(up(format!("{}*", c.db_path.to_string_lossy())));
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
            *f = normalize_drive_path(f, "folder", true)?;
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.folders.is_empty() {
            return Err(ConfigError::Invalid("at least one protected folder is required".into()));
        }
        for f in &self.folders {
            normalize_drive_path(f, "folder", true)?;
        }
        for m in self.excludes.iter() {
            if m.is_empty() || m.contains('\0') {
                return Err(ConfigError::Invalid(format!("invalid mask {m:?}: empty or contains NUL")));
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
    fn parses_toml() {
        let t = r#"
recycle_dir = 'D:\RecycleBinNet'
db_path = 'D:\RecycleBinNet\index.db'
folders = ['D:\Dados']
excludes = ['*.tmp']
max_bin_bytes = 10737418240
"#;
        let c: Config = toml::from_str(t).unwrap();
        assert_eq!(c.folders, vec![PathBuf::from("D:\\Dados")]);
        assert_eq!(c.max_bin_bytes, 10_737_418_240);
        assert!(c.max_age_days.is_none());
    }
    #[test]
    fn empty_folders_is_invalid() {
        let mut c = ok_cfg();
        c.folders = vec![];
        assert!(c.validate().is_err());
    }
    fn ok_cfg() -> Config {
        Config { recycle_dir: "D:\\x".into(), db_path: "D:\\db\\i.db".into(),
            folders: vec!["D:\\Dados".into()], excludes: vec![], max_bin_bytes: 1, max_age_days: None }
    }
    #[test]
    fn driver_masks_are_uppercase_full_path_and_exclude_own_files() {
        let mut c = Config {
            recycle_dir: r"E:\Lixeira".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            folders: vec![r"d:/Dados/".into(), r"F:\".into()],
            excludes: vec!["*.tmp".into(), "~$*".into(), r"D:\Dados\Temp\*".into()],
            ..Config::default()
        };
        c.normalize().unwrap();
        c.validate().unwrap();
        let (inc, exc) = driver_masks(&c);
        assert_eq!(inc, [r"D:\DADOS\*", r"F:\*"]);
        assert_eq!(
            exc,
            [r"*\*.TMP", r"*\~$*", r"D:\DADOS\TEMP\*", r"E:\LIXEIRA\*", r"C:\PROGRAMDATA\RNETRBIN\INDEX.DB*"]
        );
    }
    #[test]
    fn valid_config_passes() { assert!(ok_cfg().validate().is_ok()); }
    #[test]
    fn save_then_load_round_trips_and_invalid_is_not_written() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("sub").join("config.toml");
        let mut c = Config { recycle_dir: "d:/bin/".into(), folders: vec!["d:/Dados".into()], max_age_days: Some(30), ..Config::default() };
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
        let mut bad = Config { max_bin_bytes: 0, ..Config::default() };
        let q = t.path().join("bad.toml");
        assert!(bad.save(&q).is_err());
        assert!(!q.exists());
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
    fn masks_with_nul_or_empty_are_invalid() {
        let mut c = ok_cfg(); c.excludes = vec!["*.\0".into()];
        assert!(c.validate().is_err());
        let mut c = ok_cfg(); c.excludes = vec!["".into()];
        assert!(c.validate().is_err());
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
