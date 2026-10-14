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
        let c = Config { recycle_dir: "D:\\x".into(), db_path: "D:\\x\\i.db".into(),
            includes: vec![], excludes: vec![], max_bin_bytes: 1, max_age_days: None };
        assert!(c.validate().is_err());
    }
}
