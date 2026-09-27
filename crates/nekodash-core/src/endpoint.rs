use crate::{Error, Result};
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use std::fmt;
use url::Url;

#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    id: String,
    label: String,
    url: Url,
    secret: String,
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An endpoint prefix can itself be private; debug output uses only its public label/id.
        f.debug_struct("Endpoint")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl Endpoint {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        url: &str,
        secret: impl Into<String>,
    ) -> Result<Self> {
        let mut value = Self {
            id: id.into(),
            label: label.into(),
            url: Url::parse(url).map_err(|_| Error::invalid("invalid endpoint URL"))?,
            secret: secret.into(),
        };
        value.validate()?;
        if !value.url.path().ends_with('/') {
            let path = format!("{}/", value.url.path());
            value.url.set_path(&path);
        }
        Ok(value)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn url(&self) -> &Url {
        &self.url
    }
    /// Used to populate the password field when editing a saved endpoint.
    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(Error::invalid("endpoint ID is empty"));
        }
        if !matches!(self.url.scheme(), "http" | "https") || self.url.host_str().is_none() {
            return Err(Error::invalid("endpoint requires an HTTP or HTTPS host"));
        }
        if !self.url.username().is_empty()
            || self.url.password().is_some()
            || self.url.query().is_some()
            || self.url.fragment().is_some()
        {
            return Err(Error::invalid(
                "endpoint URL must contain only scheme, host, port and path",
            ));
        }
        self.authorization()?;
        Ok(())
    }

    pub(crate) fn authorization(&self) -> Result<Option<HeaderValue>> {
        if self.secret.is_empty() {
            return Ok(None);
        }
        let mut header = HeaderValue::from_str(&format!("Bearer {}", self.secret))
            .map_err(|_| Error::invalid("secret cannot be represented as a Bearer header"))?;
        header.set_sensitive(true);
        Ok(Some(header))
    }

    pub(crate) fn resource(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Error::invalid("endpoint cannot hold path segments"))?;
            path.pop_if_empty();
            for segment in segments {
                // The URL standard normalizes dot segments; reject them before URL construction.
                if segment.is_empty() || matches!(*segment, "." | "..") {
                    return Err(Error::invalid("resource name is empty or a dot segment"));
                }
                path.push(segment);
            }
        }
        Ok(url)
    }

    pub(crate) fn websocket_url(&self, path: &str, level: Option<&str>) -> Result<Url> {
        let mut url = self.resource(&[path])?;
        let scheme = if self.url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        url.set_scheme(scheme)
            .map_err(|_| Error::invalid("invalid WebSocket scheme"))?;
        if !self.secret.is_empty() {
            url.query_pairs_mut().append_pair("token", &self.secret);
        }
        if let Some(level) = level {
            url.query_pairs_mut().append_pair("level", level);
        }
        Ok(url)
    }

    pub(crate) fn redact(&self, message: &str) -> String {
        if self.secret.is_empty() {
            return message.to_owned();
        }
        let encoded: String =
            url::form_urlencoded::byte_serialize(self.secret.as_bytes()).collect();
        message
            .replace(&self.secret, "[redacted]")
            .replace(&encoded, "[redacted]")
    }
}
