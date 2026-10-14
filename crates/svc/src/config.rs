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

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let c: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.includes.is_empty() {
            return Err(ConfigError::Invalid("includes must not be empty".into()));
        }
        if !self.recycle_dir.is_absolute() {
            return Err(ConfigError::Invalid("recycle_dir must be an absolute path".into()));
        }
        if self.max_bin_bytes == 0 {
            return Err(ConfigError::Invalid("max_bin_bytes must be > 0".into()));
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
}
