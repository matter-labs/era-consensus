//! Http Server configuration structs
use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
/// Debug Page credentials (user:password)
#[derive(Debug, PartialEq, Clone)]
pub struct DebugCredentials {
    /// User for debug page
    pub user: String,
    /// Password for debug page
    pub password: String,
}

impl TryFrom<String> for DebugCredentials {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self> {
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

impl From<DebugCredentials> for String {
    fn from(val: DebugCredentials) -> Self {
        format!("{}:{}", val.user, val.password)
    }
}

/// Http debug page configuration.
#[derive(Debug, Clone)]
pub struct DebugPageConfig {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
    /// Debug page credentials.
    pub credentials: Option<DebugCredentials>,
    /// Cert file path
    pub cert_path: PathBuf,
    /// Key file path
    pub key_path: PathBuf,
}
