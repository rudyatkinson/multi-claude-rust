//! OAuth tokens of token accounts in the OS credential store (`src/main/tokenVault.ts`, now via
//! `keyring`: Credential Manager on Windows, Keychain on macOS, Secret Service on Linux).
//!
//! The Electron version kept the token in `config.json`, encrypted with `safeStorage`, as
//! `tokenEnc`. Here `tokenEnc` holds only [`TOKEN_MARKER`] once the token is in the credential
//! store, so the file keeps its shape and never holds the token. A `tokenEnc` from the Electron
//! version cannot be decrypted without Electron; such an account reads as having no token and
//! the user enters it again ("Replace token").

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use mc_core::error::{AppError, AppResult};

/// The service name of the entries in the credential store; the user name is the profile id.
pub const KEYRING_SERVICE: &str = "multi-claude";

/// What `Profile.token_enc` holds when the token is in the credential store.
pub const TOKEN_MARKER: &str = "keyring";

/// Where tokens are kept. The app uses [`TokenVault`]; tests use [`MemoryTokenStore`].
pub trait TokenStore: Send + Sync {
    /// Saves or replaces the token of an account.
    fn save(&self, profile_id: &str, token: &str) -> AppResult<()>;
    /// The token of an account; None when there is none or it cannot be read.
    fn load(&self, profile_id: &str) -> Option<String>;
    /// Removes the token of an account. Removing one that is not there is not an error.
    fn delete(&self, profile_id: &str) -> AppResult<()>;
}

/// The OS credential store.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenVault;

impl TokenVault {
    pub fn new() -> Self {
        TokenVault
    }

    fn entry(profile_id: &str) -> keyring::Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, profile_id)
    }
}

impl TokenStore for TokenVault {
    fn save(&self, profile_id: &str, token: &str) -> AppResult<()> {
        Self::entry(profile_id).and_then(|e| e.set_password(token)).map_err(|e| {
            AppError::io(format!("Secure storage is not available on this system, so the token cannot be saved. ({e})"))
        })
    }

    fn load(&self, profile_id: &str) -> Option<String> {
        match Self::entry(profile_id).and_then(|e| e.get_password()) {
            Ok(token) => Some(token),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => {
                log::warn!("token of {profile_id}: {e}");
                None
            }
        }
    }

    fn delete(&self, profile_id: &str) -> AppResult<()> {
        match Self::entry(profile_id).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::io(format!("Could not remove the token from secure storage. ({e})"))),
        }
    }
}

/// Tokens in memory, for tests.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    tokens: Mutex<HashMap<String, String>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many tokens are held.
    pub fn len(&self) -> usize {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TokenStore for MemoryTokenStore {
    fn save(&self, profile_id: &str, token: &str) -> AppResult<()> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner).insert(profile_id.to_owned(), token.to_owned());
        Ok(())
    }

    fn load(&self, profile_id: &str) -> Option<String> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner).get(profile_id).cloned()
    }

    fn delete(&self, profile_id: &str) -> AppResult<()> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner).remove(profile_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_store_saves_replaces_and_deletes() {
        let store = MemoryTokenStore::new();
        assert_eq!(store.load("a"), None);
        store.save("a", "one").unwrap();
        store.save("a", "two").unwrap();
        assert_eq!(store.load("a").as_deref(), Some("two"));
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert!(store.is_empty());
    }
}
