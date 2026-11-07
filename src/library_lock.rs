//! The library's password. Only a salted, slow hash is stored. This keeps
//! the library page out of reach in the app; files stay as they are on disk.

use std::hash::{BuildHasher, Hasher};

use sha2::{Digest, Sha256};

pub const SETTING: &str = "library_lock";
pub const MIN_LENGTH: usize = 4;
const ROUNDS: u32 = 100_000;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn stretch(password: &str, salt: &str) -> String {
    let mut digest = Sha256::digest(format!("{salt}:{password}").as_bytes()).to_vec();
    for _ in 0..ROUNDS {
        digest = Sha256::digest(&digest).to_vec();
    }
    hex(&digest)
}

fn new_salt() -> String {
    let mut seed = Sha256::new();
    for _ in 0..4 {
        // Each RandomState starts from the system's random source.
        let n = std::collections::hash_map::RandomState::new().build_hasher().finish();
        seed.update(n.to_le_bytes());
    }
    hex(&seed.finalize()[..16])
}

/// The text to store for `password`.
pub fn seal(password: &str) -> String {
    let salt = new_salt();
    let hash = stretch(password, &salt);
    format!("{salt}:{hash}")
}

pub fn verify(sealed: &str, password: &str) -> bool {
    let Some((salt, hash)) = sealed.split_once(':') else { return false };
    let wanted = stretch(password, salt);
    // Compares everything, so timing doesn't show how much matched.
    wanted.len() == hash.len() && wanted.bytes().zip(hash.bytes()).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_password_opens() {
        let sealed = seal("1234");
        assert!(verify(&sealed, "1234"));
        assert!(!verify(&sealed, "12345"));
        assert!(!verify(&sealed, ""));
        assert!(!verify("broken", "1234"));
    }

    #[test]
    fn the_password_is_not_stored_and_salts_differ() {
        let (a, b) = (seal("secret"), seal("secret"));
        assert_ne!(a, b);
        assert!(!a.contains("secret"));
    }
}
