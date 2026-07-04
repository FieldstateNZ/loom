//! Integration tests for provider credential upsert and scoping, against a
//! real database.

mod common;

use loom_store::{CredentialStore, NewProviderCredential, NewTenant, TenantStore};

#[tokio::test]
async fn credential_upsert_and_global() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("creds", "Creds Tenant"))
        .await
        .unwrap();

    // Tenant-scoped credential.
    let cred = store
        .upsert_credential(NewProviderCredential {
            tenant_id: Some(tenant.id),
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![1, 2, 3, 4],
            nonce: Some(vec![9, 9]),
            aad: None,
            base_url: Some("https://api.anthropic.com".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(cred.tenant_id, Some(tenant.id));
    assert_eq!(cred.encrypted_secret, vec![1, 2, 3, 4]);

    // Upsert replaces the secret for the same (tenant, provider) pair.
    let updated = store
        .upsert_credential(NewProviderCredential {
            tenant_id: Some(tenant.id),
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![5, 6, 7, 8],
            nonce: Some(vec![1]),
            aad: Some(vec![2]),
            base_url: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.id, cred.id, "upsert keeps the same row");
    assert_eq!(updated.encrypted_secret, vec![5, 6, 7, 8]);
    assert_eq!(updated.base_url, None);

    // Gateway-global credential (NULL tenant) coexists with the tenant one.
    let global = store
        .upsert_credential(NewProviderCredential {
            tenant_id: None,
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![0],
            nonce: None,
            aad: None,
            base_url: None,
        })
        .await
        .unwrap();
    assert_eq!(global.tenant_id, None);

    let got = store
        .get_credential(Some(tenant.id), "anthropic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, updated);

    let got_global = store
        .get_credential(None, "anthropic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_global, global);

    let tenant_list = store.list_credentials(Some(tenant.id)).await.unwrap();
    assert_eq!(tenant_list.len(), 1);
}
