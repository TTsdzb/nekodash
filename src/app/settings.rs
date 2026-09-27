use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub language: String,
    pub table_density: usize,
    pub dark: bool,
    pub default_page: usize,
    pub test_url: String,
    pub use_core_test_url: bool,
    pub test_timeout_ms: u32,
    pub test_concurrency: usize,
    pub proxy_sort: usize,
    pub display_mode: usize,
    pub close_after_select: bool,
    pub log_limit: usize,
    pub connection_limit: usize,
    pub log_level: usize,
    pub track_traffic: bool,
    pub retention_days: u32,
    pub source_tags: BTreeMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh".into(),
            table_density: 1,
            dark: true,
            default_page: 0,
            test_url: "https://www.gstatic.com/generate_204".into(),
            use_core_test_url: true,
            test_timeout_ms: 5000,
            test_concurrency: 8,
            proxy_sort: 0,
            display_mode: 0,
            close_after_select: true,
            log_limit: 1000,
            connection_limit: 1000,
            log_level: 1,
            track_traffic: true,
            retention_days: 30,
            source_tags: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> AppResult<()> {
        if !["zh", "en", "ru", "ko", "fr", "ja", "fa"].contains(&self.language.as_str())
            || self.table_density > 2
            || self.default_page > 6
            || self.proxy_sort > 4
            || self.display_mode > 1
            || self.log_level > 4
            || !(100..=100_000).contains(&self.log_limit)
            || !(100..=100_000).contains(&self.connection_limit)
            || !(1..=32).contains(&self.test_concurrency)
            || !(100..=60_000).contains(&self.test_timeout_ms)
            || self.retention_days > 3650
            || !(self.test_url.starts_with("http://") || self.test_url.starts_with("https://"))
        {
            return Err("Invalid settings value".into());
        }
        Ok(())
    }
    pub fn resolve_test_url(&self, core: Option<&str>) -> String {
        if self.use_core_test_url {
            core.filter(|url| !url.trim().is_empty())
                .unwrap_or(&self.test_url)
                .to_owned()
        } else {
            self.test_url.clone()
        }
    }
    pub fn load(path: &Path) -> AppResult<Self> {
        let value = match read_bounded(path, 4 * 1024 * 1024) {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes)?,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Self::default()
            }
            Err(e) => return Err(e),
        };
        value.validate()?;
        Ok(value)
    }
}
pub fn read_bounded(path: &Path, limit: u64) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("File exceeds size limit".into());
    }
    Ok(bytes)
}
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn data_dir() -> AppResult<PathBuf> {
    if let Some(path) = std::env::var_os("NEKODASH_DATA_DIR") {
        return Ok(path.into());
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    #[cfg(target_os = "android")]
    let base: Option<PathBuf> = None;
    base.map(|p| p.join("nekodash"))
        .ok_or_else(|| "Application data directory is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_reject_invalid_limits() -> AppResult<()> {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("settings.json");
        let mut settings = Settings {
            language: "ja".into(),
            ..Settings::default()
        };
        write_atomic(&path, &serde_json::to_vec(&settings)?)?;
        assert_eq!(Settings::load(&path)?.language, "ja");
        settings.test_concurrency = 33;
        assert!(settings.validate().is_err());
        settings.test_concurrency = 4;
        settings.test_timeout_ms = 0;
        assert!(settings.validate().is_err());
        write_atomic(&path, b"{")?;
        assert!(Settings::load(&path).is_err());
        Ok(())
    }
    #[test]
    fn latency_url_respects_source_and_empty_core_fallback() {
        let mut settings = Settings::default();
        assert_eq!(
            settings.resolve_test_url(Some("https://core.test/")),
            "https://core.test/"
        );
        assert_eq!(settings.resolve_test_url(Some("")), settings.test_url);
        settings.use_core_test_url = false;
        assert_eq!(
            settings.resolve_test_url(Some("https://core.test/")),
            settings.test_url
        );
    }
    #[test]
    fn read_limit_is_enforced() -> AppResult<()> {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("data");
        write_atomic(&path, &[0; 32])?;
        assert!(read_bounded(&path, 16).is_err());
        Ok(())
    }
}
