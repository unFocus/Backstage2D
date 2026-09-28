//! Stable, typed IDs: random 64-bit values written as a kind prefix plus 13
//! characters of Crockford base32, e.g. `node_3kq9t0x2v7m1a`.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use std::str::FromStr;

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
const ENCODED_LEN: usize = 13;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id {input:?}")]
pub struct ParseIdError {
    kind: &'static str,
    input: String,
}

fn encode(value: u64) -> String {
    (0..ENCODED_LEN).rev().map(|i| ALPHABET[((value >> (i * 5)) & 31) as usize] as char).collect()
}

fn decode(s: &str) -> Option<u64> {
    if s.len() != ENCODED_LEN {
        return None;
    }
    let mut value: u64 = 0;
    for (i, c) in s.bytes().enumerate() {
        let digit = ALPHABET.iter().position(|&a| a == c.to_ascii_lowercase())? as u64;
        // The first character only holds the top 4 bits of 65.
        if i == 0 && digit > 15 {
            return None;
        }
        value = (value << 5) | digit;
    }
    Some(value)
}

macro_rules! define_ids {
    ($($(#[$doc:meta])* $name:ident => $prefix:literal;)*) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A new random ID.
            pub fn new() -> Self {
                Self(getrandom::u64().expect("OS random number generator unavailable"))
            }

            /// An ID with a fixed value, for tests and samples.
            pub const fn from_raw(value: u64) -> Self {
                Self(value)
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.strip_prefix(concat!($prefix, "_"))
                    .and_then(decode)
                    .map(Self)
                    .ok_or_else(|| ParseIdError { kind: $prefix, input: s.to_owned() })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
            }
        }
    )*};
}

define_ids! {
    /// A composition in the project library.
    CompId => "comp";
    /// A node within a composition.
    NodeId => "node";
    /// An animation within a composition.
    AnimId => "anim";
    /// A drawing within a flipbook node.
    DrawingId => "draw";
    /// A bitmap, font, or audio asset.
    AssetId => "asset";
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn display_has_prefix_and_fixed_length() {
        let s = NodeId::from_raw(0).to_string();
        assert_eq!(s, "node_0000000000000");
        assert_eq!(NodeId::from_raw(u64::MAX).to_string(), "node_fzzzzzzzzzzzz");
    }

    #[test]
    fn parse_checks_prefix_and_encoding() {
        let id = CompId::from_raw(12345);
        assert_eq!(id.to_string().parse(), Ok(id));
        assert_eq!(id.to_string().to_uppercase().replace("COMP", "comp").parse(), Ok(id));
        assert!("node_0000000000001".parse::<CompId>().is_err(), "wrong prefix");
        assert!("comp_000000000001".parse::<CompId>().is_err(), "too short");
        assert!("comp_g000000000000".parse::<CompId>().is_err(), "overflows 64 bits");
        assert!("comp_000000000000u".parse::<CompId>().is_err(), "'u' is not in the alphabet");
    }

    #[test]
    fn random_ids_are_distinct() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| AnimId::new()).collect();
        assert_eq!(ids.len(), 1000);
    }

    proptest! {
        #[test]
        fn round_trips(raw in any::<u64>()) {
            let id = AssetId::from_raw(raw);
            prop_assert_eq!(id.to_string().parse::<AssetId>(), Ok(id));
        }
    }
}
