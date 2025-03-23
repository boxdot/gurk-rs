use std::{fmt, str::FromStr};

use anyhow::ensure;
use serde::{Deserialize, Deserializer, Serialize, de};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop, Serialize)]
pub struct Passphrase(String);

impl Passphrase {
    pub fn new(passphrase: impl Into<String>) -> anyhow::Result<Self> {
        let passphrase = passphrase.into();
        ensure!(!passphrase.is_empty(), "passphrase cannot be empty");
        Ok(Self(passphrase))
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
