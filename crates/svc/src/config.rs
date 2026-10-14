use std::path::{Path, PathBuf};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub recycle_dir: PathBuf,
    pub db_path: PathBuf,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub max_bin_bytes: u64,
    #[serde(default)]
    pub max_age_days: Option<u32>,
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("parse: {0}")] Parse(#[from] toml::de::Error),
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
    if rest.split('\\').any(|c| c.is_empty() || c == "." || c == "..") {
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
    /// Rewrite `recycle_dir` and `db_path` into canonical `X:\...` form; this is
    /// the exact string later sent to the driver.
    pub fn normalize(&mut self) -> Result<(), ConfigError> {
        self.recycle_dir = normalize_drive_path(&self.recycle_dir, "recycle_dir", false)?;
        self.db_path = normalize_drive_path(&self.db_path, "db_path", false)?;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.includes.is_empty() {
            return Err(ConfigError::Invalid("includes must not be empty".into()));
        }
        for m in self.includes.iter().chain(self.excludes.iter()) {
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
includes = ['*.*']
excludes = ['*.tmp']
max_bin_bytes = 10737418240
"#;
        let c: Config = toml::from_str(t).unwrap();
        assert_eq!(c.includes, vec!["*.*".to_string()]);
        assert_eq!(c.max_bin_bytes, 10_737_418_240);
        assert!(c.max_age_days.is_none());
    }
    #[test]
    fn empty_includes_is_invalid() {
        let mut c = ok_cfg();
        c.includes = vec![];
        assert!(c.validate().is_err());
    }
    fn ok_cfg() -> Config {
        Config { recycle_dir: "D:\\x".into(), db_path: "D:\\db\\i.db".into(),
            includes: vec!["*.*".into()], excludes: vec![], max_bin_bytes: 1, max_age_days: None }
    }
    #[test]
    fn valid_config_passes() { assert!(ok_cfg().validate().is_ok()); }
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
        let mut c = ok_cfg(); c.includes = vec!["*.\0".into()];
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
