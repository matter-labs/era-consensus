//! Http Server configuration structs
use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
/// Debug Page credentials (user:password)
#[derive(PartialEq, Clone)]
pub struct DebugPageCredentials {
    /// User for debug page
    pub user: String,
    /// Password for debug page
    pub password: String,
}

impl TryFrom<String> for DebugPageCredentials {
    type Error = anyhow::Error;
    fn try_from(value: String) -> anyhow::Result<Self> {
        let mut credentials = value.split(':');
        let user = credentials.next().context("Empty debug page credentials")?;
        let password = credentials
            .next()
            .context("Invalid debug page credentials: expected '{user:password}'")?;
        Ok(Self {
            user: user.to_string(),
            password: password.to_string(),
        })
    }
}

impl From<DebugPageCredentials> for String {
    fn from(val: DebugPageCredentials) -> Self {
        format!("{}:{}", val.user, val.password)
    }
}

impl std::fmt::Debug for DebugPageCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugCredentials")
            .field("user", &"****")
            .field("password", &"****")
            .finish()
    }
}

/// Http debug page configuration.
#[derive(Debug, Clone)]
pub struct DebugPageConfig {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
    /// Debug page credentials.
    pub credentials: Option<DebugPageCredentials>,
    /// Cert file path
    pub cert_path: PathBuf,
    /// Key file path
    pub key_path: PathBuf,
}
