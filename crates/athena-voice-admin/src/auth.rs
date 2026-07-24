//! First-run admin token + Bearer verification.
//!
//! The token is generated once, printed to the terminal by the caller, and
//! only its argon2 hash is stored. There is deliberately no recovery: to
//! reset, delete the `admin_auth` row and restart.

use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use athena_voice_storage::Store;
use uuid::Uuid;

/// On first run: generate a token, store its hash, and return the plaintext
/// (show it to the user immediately — it is never recoverable later).
/// Subsequent runs return `None`.
pub async fn ensure_token(store: &Arc<dyn Store>) -> anyhow::Result<Option<String>> {
    if store.admin_token_hash().await?.is_some() {
        return Ok(None);
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash admin token: {e}"))?
        .to_string();
    store.admin_token_hash_set(&hash).await?;
    Ok(Some(token))
}

pub fn verify(hash: &str, token: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(token.as_bytes(), &h)
            .is_ok()
    })
}
