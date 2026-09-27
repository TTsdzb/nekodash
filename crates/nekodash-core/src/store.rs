use crate::{Endpoint, Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

/// Call filesystem methods on a worker thread with a per-user app-data path.
/// Unix files use mode 0600; other platforms inherit the app-data directory's ACLs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EndpointStore {
    schema_version: u32,
    endpoints: Vec<Endpoint>,
    selected: Option<String>,
}

impl Default for EndpointStore {
    fn default() -> Self {
        Self {
            schema_version: 1,
            endpoints: Vec::new(),
            selected: None,
        }
    }
}

impl EndpointStore {
    pub fn endpoints(&self) -> &[Endpoint] {
        &self.endpoints
    }
    pub fn selected(&self) -> Option<&Endpoint> {
        self.selected
            .as_ref()
            .and_then(|id| self.endpoints.iter().find(|endpoint| endpoint.id() == id))
    }
    pub fn upsert(&mut self, endpoint: Endpoint) -> Result<()> {
        endpoint.validate()?;
        if let Some(existing) = self
            .endpoints
            .iter_mut()
            .find(|item| item.id() == endpoint.id())
        {
            *existing = endpoint;
        } else {
            self.endpoints.insert(0, endpoint);
        }
        Ok(())
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
        if !self.endpoints.iter().any(|endpoint| endpoint.id() == id) {
            return Err(Error::invalid("selected endpoint does not exist"));
        }
        self.selected = Some(id.to_owned());
        Ok(())
    }
    pub fn remove(&mut self, id: &str) {
        self.endpoints.retain(|endpoint| endpoint.id() != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(Error::new(
                ErrorKind::Storage,
                "load endpoints",
                "unsupported settings schema version",
            ));
        }
        let mut ids = HashSet::new();
        for endpoint in &self.endpoints {
            endpoint.validate()?;
            if !ids.insert(endpoint.id()) {
                return Err(Error::invalid("duplicate endpoint ID"));
            }
        }
        if self.selected.is_some() && self.selected().is_none() {
            return Err(Error::invalid("selected endpoint does not exist"));
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(Error::io("load endpoints", &error)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_STORE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("load endpoints", &e))?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::new(
                ErrorKind::ResponseTooLarge,
                "load endpoints",
                "settings file exceeds size limit",
            ));
        }
        let store: Self =
            serde_json::from_slice(&bytes).map_err(|e| Error::decode("load endpoints", e))?;
        store.validate()?;
        Ok(store)
    }
    /// Writes a complete temporary file beside the destination, syncs it, then atomically replaces it.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| Error::decode("save endpoints", e))?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::new(
                ErrorKind::ResponseTooLarge,
                "save endpoints",
                "settings exceed size limit",
            ));
        }
        fs::create_dir_all(parent).map_err(|e| Error::io("create settings directory", &e))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| Error::io("create settings file", &e))?;
        // tempfile creates mode 0600 on Unix; desktop callers choose their per-user app-data directory.
        temporary
            .write_all(&bytes)
            .map_err(|e| Error::io("write endpoints", &e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| Error::io("sync endpoints", &e))?;
        temporary
            .persist(path)
            .map_err(|e| Error::io("replace endpoints", &e.error))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| Error::io("sync settings directory", &e))?;
        Ok(())
    }
}
