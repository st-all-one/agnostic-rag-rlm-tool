//! Behavioral tests for refresh/session token persistence (plan 018).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_storage::Storage;
use arags_storage::sqlite::tokens::{
    NewToken, Role, create_session, create_token, hash_refresh, list_tokens, revoke_all_tokens,
    revoke_token_by_id, validate_session,
};
use rusqlite::params;

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().unwrap();
    Storage::open(dir.path()).unwrap()
}

fn new_token(username: &str, role: Role) -> NewToken {
    NewToken {
        username: username.into(),
        role,
        created_by: "system".into(),
    }
}

#[test]
fn create_and_validate_session() {
    let storage = temp_storage();
    let (_, refresh) = create_token(&storage, &new_token("dev1", Role::Admin)).expect("create");

    let (sid, user, role, _exp) = create_session(&storage, &refresh).expect("create session");
    assert_eq!(user, "dev1");
    assert_eq!(role, Role::Admin);

    let ctx = validate_session(&storage, &sid).expect("validate");
    assert_eq!(ctx, Some(("dev1".into(), Role::Admin)));
}

#[test]
fn revoked_refresh_invalidates_session() {
    let storage = temp_storage();
    let (id, refresh) = create_token(&storage, &new_token("dev1", Role::NonAdmin)).unwrap();
    let (sid, _, _, _) = create_session(&storage, &refresh).expect("create session");

    assert!(validate_session(&storage, &sid).unwrap().is_some());

    assert!(revoke_token_by_id(&storage, &id, "admin").unwrap());
    assert!(validate_session(&storage, &sid).unwrap().is_none());
}

#[test]
fn revoked_by_username_purges_sessions() {
    let storage = temp_storage();
    let (_, r1) = create_token(&storage, &new_token("dev1", Role::NonAdmin)).unwrap();
    let (_, r2) = create_token(&storage, &new_token("dev2", Role::Admin)).unwrap();
    let (s1, _, _, _) = create_session(&storage, &r1).unwrap();
    let (s2, _, _, _) = create_session(&storage, &r2).unwrap();

    // Unknown user revokes nothing.
    assert!(
        !arags_storage::sqlite::tokens::revoke_token_by_username(&storage, "nobody", "admin")
            .unwrap()
    );
    assert!(validate_session(&storage, &s1).unwrap().is_some());

    // Revoking all of dev1's tokens kills dev1's live session only.
    assert!(
        arags_storage::sqlite::tokens::revoke_token_by_username(&storage, "dev1", "admin").unwrap()
    );
    assert!(validate_session(&storage, &s1).unwrap().is_none());
    assert!(validate_session(&storage, &s2).unwrap().is_some());
}

#[test]
fn expired_refresh_rejected() {
    let storage = temp_storage();
    let (_, refresh) = create_token(&storage, &new_token("dev1", Role::NonAdmin)).unwrap();

    storage
        .connection()
        .unwrap()
        .execute(|conn| {
            conn.execute(
                "UPDATE auth_tokens SET expires_at = 1 WHERE token_hash = ?1",
                params![hash_refresh(&refresh)],
            )?;
            Ok(())
        })
        .unwrap();

    assert!(create_session(&storage, &refresh).is_err());
}

#[test]
fn prune_revokes_all() {
    let storage = temp_storage();
    for u in ["dev1", "dev2"] {
        create_token(&storage, &new_token(u, Role::NonAdmin)).unwrap();
    }
    let n = revoke_all_tokens(&storage, "admin").unwrap();
    assert_eq!(n, 2);
    assert!(list_tokens(&storage).unwrap().iter().all(|t| t.revoked));
}

#[test]
fn plaintext_never_persisted() {
    let storage = temp_storage();
    let (_, refresh) = create_token(&storage, &new_token("dev1", Role::Admin)).unwrap();
    let rows = list_tokens(&storage).unwrap();
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].token_hash, refresh);
    assert_eq!(rows[0].token_hash, hash_refresh(&refresh));
}
