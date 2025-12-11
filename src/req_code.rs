use std::fmt::Formatter;
use std::str::FromStr;
use rand::Rng;
use serde::{Deserialize, Deserializer};
use serde::de::{Error, Unexpected, Visitor};

#[derive(Debug, Copy, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub struct RequestCode([u8; 9]);

impl RequestCode {
    pub fn new_rand() -> Self {
        let mut rng = rand::rng();
        let key = core::array::from_fn(|_| {
            rng.random_range(b'A'..=b'Z')
        });
        Self(key)
    }

    pub const fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.0) }
    }
}

impl FromStr for RequestCode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let key = <[u8; 9]>::try_from(s.as_bytes())
            .map_err(|_| "mismatched keycode length")?;

        if key.iter().any(|char| !(b'A'..=b'Z').contains(char)) {
            return Err("invalid character in key")
        }

        Ok(Self(key))
    }
}

impl<'de> Deserialize<'de> for RequestCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CodeVisitor;
        impl<'de> Visitor<'de> for CodeVisitor {
            type Value = RequestCode;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                formatter.write_str("an absher zt request code")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error
            {
                RequestCode::from_str(v).map_err(E::custom)
            }

            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: Error
            {
                self.visit_str(v)
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: Error
            {
                self.visit_str(&v)
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: Error
            {
                str::from_utf8(v)
                    .map_err(|_| E::invalid_value(Unexpected::Bytes(v), &self))
                    .and_then(|s| self.visit_str(s))
            }

            fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
            where
                E: Error
            {
                self.visit_bytes(v)
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
            where
                E: Error
            {
                self.visit_bytes(&v)
            }
        }


        deserializer.deserialize_str(CodeVisitor)
    }
}