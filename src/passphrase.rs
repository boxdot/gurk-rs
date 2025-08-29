use std::{env::var, fmt, process::Command, str::FromStr};

use anyhow::{Context, ensure};
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

    /// Executes an external command (simple shell script form "cmd [args...]") to get the passphrase.
    fn get_from_external_command(script: &str) -> anyhow::Result<Self> {
        let mut cli = script.split_whitespace();
        let cmd = cli
            .next()
            .ok_or_else(|| anyhow::anyhow!("command cannot be empty"))?;
        let args: Vec<&str> = cli.collect();

        let output = Command::new(cmd)
            .args(&args)
            .output()
            .with_context(|| format!("failed to execute command: {script}"))?;

        if !output.status.success() {
            anyhow::bail!("command failed with exit code {:?}", output.status.code());
        }

        let passphrase = String::from_utf8(output.stdout)?.trim_end().to_string();
        ensure!(!passphrase.is_empty(), "passphrase cannot be empty");
        Ok(Self(passphrase))
    }

    /// Gets the passphrase from difference sources in the following order:
    ///
    /// 1. CLI argument for passphrase
    /// 2. config file
    /// 3. CLI argument for external command
    /// 4. environment variable for external command
    /// 5. Keychain (macOS only)
    /// 6. prompt for passphrase
    pub fn get(
        passphrase_cli: Option<Passphrase>,
        passphrase_command_cli: Option<String>,
        config: &mut Config,
    ) -> anyhow::Result<Passphrase> {
        if let Some(passphrase) = passphrase_cli {
            return Ok(passphrase);
        }

        if let Some(passphrase) = config.passphrase.take() {
            return Ok(passphrase);
        }

        if let Some(script) = passphrase_command_cli {
            return Self::get_from_external_command(&script);
        }

        if let Ok(script) = var("GURK_PASSPHRASE_COMMAND") {
            return Self::get_from_external_command(&script);
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

    #[test]
    fn test_get_from_external_command_success() {
        let result = Passphrase::get_from_external_command("echo secret");
        assert!(result.is_ok());
        let passphrase = result.unwrap();
        assert_eq!(format!("{:?}", passphrase), "Passphrase(<redacted>)");
    }

    #[test]
    fn test_get_from_external_command_with_args() {
        let result = Passphrase::get_from_external_command("echo -n secret");
        assert!(result.is_ok());
        let passphrase = result.unwrap();
        assert_eq!(format!("{:?}", passphrase), "Passphrase(<redacted>)");
    }

    #[test]
    fn test_get_from_external_command_failure() {
        let result = Passphrase::get_from_external_command("false");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_from_external_command_empty() {
        let result = Passphrase::get_from_external_command("");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_from_external_command_nonexistent() {
        let result = Passphrase::get_from_external_command("gurk-test_nonexistent_command");
        assert!(result.is_err());
    }
}
