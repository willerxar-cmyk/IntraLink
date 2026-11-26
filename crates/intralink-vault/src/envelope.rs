//! Versioned, authenticated encryption with fixed, bounded KDF parameters.
use anyhow::{Result, anyhow, ensure};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::Profile;

const MAGIC: &[u8; 8] = b"ILVAULT2";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN;
const TAG_LEN: usize = 16;
pub(crate) const MAX_FILE_SIZE: usize = 1024 * 1024;

fn key(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    // Version 2 uses exactly these parameters. Do not accept resource costs from disk.
    let params =
        Params::new(64 * 1024, 3, 1, Some(32)).map_err(|_| anyhow!("Invalid KDF parameters"))?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut *key)
        .map_err(|_| anyhow!("Could not derive vault key"))?;
    Ok(key)
}

pub(crate) fn seal(profile: &Profile, password: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        (12..=1024).contains(&password.len()),
        "Use a passphrase of 12-1024 bytes"
    );
    profile.validate()?;
    let mut salt = [0; SALT_LEN];
    let mut nonce = [0; NONCE_LEN];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut salt)
        .map_err(|_| anyhow!("OS random source failed"))?;
    rng.try_fill_bytes(&mut nonce)
        .map_err(|_| anyhow!("OS random source failed"))?;
    let key = key(password, &salt)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| anyhow!("Invalid vault key"))?;
    let mut output = Vec::from(MAGIC.as_slice());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce);
    let plaintext = Zeroizing::new(
        serde_json::to_vec(profile).map_err(|_| anyhow!("Could not serialize profile"))?,
    );
    let nonce = XNonce::from(nonce);
    let encrypted = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &output,
            },
        )
        .map_err(|_| anyhow!("Could not encrypt vault"))?;
    ensure!(
        output.len() + encrypted.len() <= MAX_FILE_SIZE,
        "Vault size limit exceeded"
    );
    output.extend_from_slice(&encrypted);
    Ok(output)
}

pub(crate) fn open(bytes: &[u8], password: &[u8]) -> Result<Profile> {
    ensure!(
        (12..=1024).contains(&password.len()),
        "Invalid passphrase length"
    );
    ensure!(
        (HEADER_LEN + TAG_LEN..=MAX_FILE_SIZE).contains(&bytes.len()),
        "Invalid vault length"
    );
    ensure!(&bytes[..MAGIC.len()] == MAGIC, "Unsupported vault format");
    let salt = &bytes[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce_bytes: [u8; NONCE_LEN] = bytes[MAGIC.len() + SALT_LEN..HEADER_LEN].try_into()?;
    let nonce = XNonce::from(nonce_bytes);
    let key = key(password, salt)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| anyhow!("Invalid vault key"))?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &bytes[HEADER_LEN..],
                    aad: &bytes[..HEADER_LEN],
                },
            )
            .map_err(|_| anyhow!("Incorrect passphrase or modified vault"))?,
    );
    // Do not expose parser errors containing any decrypted values.
    let profile: Profile =
        serde_json::from_slice(&plaintext).map_err(|_| anyhow!("Invalid encrypted profile"))?;
    profile.validate()?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use intralink_core::Role;

    #[test]
    fn roundtrip_and_reject_tampering() -> Result<()> {
        let profile = Profile::new(Role::Hybrid);
        let password = b"test passphrase for vault";
        let bytes = seal(&profile, password)?;
        assert_eq!(open(&bytes, password)?.public_id(), profile.public_id());
        assert!(open(&bytes, b"different passphrase").is_err());
        let mut modified = bytes.clone();
        let last = modified.len() - 1;
        modified[last] ^= 1;
        assert!(open(&modified, password).is_err());
        assert!(open(&bytes[..HEADER_LEN], password).is_err());
        assert_ne!(seal(&profile, password)?, bytes);
        Ok(())
    }
}
