use std::{fmt, str::FromStr};

use anyhow::ensure;
use dialoguer::{Password, theme::ColorfulTheme};
use serde::{Deserialize, Deserializer, Serialize, de};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::config::Config;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE_NAME: &str = "gurk";

#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop, Serialize)]
pub struct Passphrase(String);

impl Passphrase {
    /// Creates a new passphrase from a string.
    ///
    /// Checks that the passphrase is not empty.
    pub fn new(passphrase: impl Into<String>) -> anyhow::Result<Self> {
        let passphrase = passphrase.into();
        ensure!(!passphrase.is_empty(), "passphrase cannot be empty");
        Ok(Self(passphrase))
    }

    /// Gets the passphrase from difference sources in the following order:
    ///
    /// 1. CLI argument
    /// 2. config file
    /// 3. Keychain (macOS only)
    /// 3. prompt for passphrase
    pub fn get(
        passphrase_cli: Option<Passphrase>,
        config: &mut Config,
    ) -> anyhow::Result<Passphrase> {
        if let Some(passphrase) = passphrase_cli {
            return Ok(passphrase);
        }

        if let Some(passphrase) = config.passphrase.take() {
            return Ok(passphrase);
        }

        #[cfg(target_os = "macos")]
        if let Ok(value) = security_framework::passwords::get_generic_password(
            KEYCHAIN_SERVICE_NAME,
            &config.user.display_name,
        ) {
            return Passphrase::new(String::from_utf8(value)?);
        }

        let value = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter passphrase")
            .interact()?;
        Passphrase::new(value)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn store_in_keychain(&self, user: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        security_framework::passwords::set_generic_password(
            KEYCHAIN_SERVICE_NAME,
            user,
            self.0.as_bytes(),
        )
        .context("Failed to store passphrase to keychain")
    }

    pub(crate) fn sqlite_string(&self) -> Zeroizing<String> {
        self.0.replace("'", "''").into()
    }

    pub(crate) fn sqlite_pragma_key(&self) -> Zeroizing<String> {
        format!("'{}'", self.sqlite_string().as_str()).into()
    }
}

impl AsRef<str> for Passphrase {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

impl FromStr for Passphrase {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<'de> Deserialize<'de> for Passphrase {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passphrase_debug() {
        let passphrase = Passphrase::new("secret").unwrap();
        assert_eq!(format!("{:?}", passphrase), "Passphrase(<redacted>)");
    }
}
