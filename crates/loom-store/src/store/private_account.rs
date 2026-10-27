//! Atomic initial private account persistence.
use crate::{NewPrivateAccount, PrivateAccount, Result};
use async_trait::async_trait;

/// Creates or recovers the exact initial tenant and key. A conflict, revoked
/// key, inactive tenant or altered request returns `None` without replacement.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait PrivateAccountStore {
    /// Commits tenant, key hash and receipt in one bounded transaction.
    async fn provision_private_account(
        &self,
        new: NewPrivateAccount,
    ) -> Result<Option<PrivateAccount>>;
}
