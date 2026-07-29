use std::fs::File;
use std::io::Read;

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewOutcome {
    Submitted(String),
    Discarded,
}

pub(crate) fn generate_token() -> Result<String> {
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .context("failed to open operating-system random source")?
        .read_exact(&mut random)
        .context("failed to generate web session token")?;
    Ok(token_from_bytes(random))
}

fn token_from_bytes(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_256_bits_of_lowercase_hex() {
        let token = token_from_bytes([0xab; 32]);
        assert_eq!(token.len(), 64);
        assert_eq!(token, "ab".repeat(32));
    }

    #[test]
    fn generated_tokens_are_distinct_and_well_formed() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_ne!(first, second);
        for token in [first, second] {
            assert_eq!(token.len(), 64);
            assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
