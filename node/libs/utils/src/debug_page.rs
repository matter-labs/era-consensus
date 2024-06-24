//! Http Server configuration structs
use anyhow::Context as _;
/// Debug Page credentials (user:password)
#[derive(PartialEq, Clone)]
pub struct Credentials {
    /// User for debug page
    pub user: String,
    /// Password for debug page
    pub password: String,
}

impl TryFrom<String> for Credentials {
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

impl From<Credentials> for String {
    fn from(val: Credentials) -> Self {
        format!("{}:{}", val.user, val.password)
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugCredentials")
            .field("user", &"****")
            .field("password", &"****")
            .finish()
    }
}
