use sha2::{Digest, Sha256};

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex_encode(&digest)
}

pub fn token_matches(hash: &str, token: &str) -> bool {
    hash_token(token) == hash
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{hash_token, token_matches};

    #[test]
    fn hashes_are_stable() {
        let hash = hash_token("sample-token-value");
        assert!(token_matches(&hash, "sample-token-value"));
        assert!(!token_matches(&hash, "other"));
    }
}
