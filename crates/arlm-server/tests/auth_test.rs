//! Integration tests for plan 018 auth (refresh tokens, sessions, gates).

use std::str::FromStr;

use arlm_server::auth::{self, AuthContext, Role};
use arlm_storage::Storage;
use arlm_storage::tokens::{self, NewToken};
use tonic::Code;
use tonic::metadata::{MetadataMap, MetadataValue};

fn temp_storage() -> Storage {
    let dir = tempfile::tempdir().expect("tempdir");
    Storage::open(dir.path()).expect("open storage")
}

fn bearer_md(token: &str) -> MetadataMap {
    let mut md = MetadataMap::new();
    let value = MetadataValue::<tonic::metadata::Ascii>::from_str(&format!("Bearer {token}"))
        .expect("valid metadata value");
    md.insert("authorization", value);
    md
}

#[test]
fn refresh_roundtrip_yields_admin_context() {
    let storage = temp_storage();
    let (_, refresh) = tokens::create_token(
        &storage,
        &NewToken {
            username: "dev1".into(),
            role: Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("create token");

    let (session, user, role, _) =
        tokens::create_session(&storage, &refresh).expect("create session");
    assert_eq!(user, "dev1");
    assert_eq!(role, Role::Admin);

    let ctx = auth::authenticate(&bearer_md(&session), &storage).expect("authenticate");
    assert_eq!(ctx.username, "dev1");
    assert!(ctx.is_admin());
}

#[test]
fn missing_bearer_is_unauthenticated() {
    let storage = temp_storage();
    let err = auth::authenticate(&MetadataMap::new(), &storage).unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[test]
fn bad_bearer_is_unauthenticated() {
    let storage = temp_storage();
    let err = auth::authenticate(&bearer_md("not-a-real-session"), &storage).unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[test]
fn require_admin_gate() {
    let admin = AuthContext {
        username: "a".into(),
        role: Role::Admin,
    };
    let non_admin = AuthContext {
        username: "b".into(),
        role: Role::NonAdmin,
    };
    assert!(auth::require_admin(&admin).is_ok());
    let err = auth::require_admin(&non_admin).unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[test]
fn revoked_refresh_drops_session() {
    let storage = temp_storage();
    let (id, refresh) = tokens::create_token(
        &storage,
        &NewToken {
            username: "dev1".into(),
            role: Role::NonAdmin,
            created_by: "system".into(),
        },
    )
    .expect("create token");
    let (session, _, _, _) = tokens::create_session(&storage, &refresh).expect("create session");

    assert!(auth::authenticate(&bearer_md(&session), &storage).is_ok());
    assert!(tokens::revoke_token_by_id(&storage, &id, "admin").unwrap());
    assert!(auth::authenticate(&bearer_md(&session), &storage).is_err());
}

#[test]
fn plaintext_never_persisted_in_db() {
    let storage = temp_storage();
    let (_, refresh) = tokens::create_token(
        &storage,
        &NewToken {
            username: "dev1".into(),
            role: Role::Admin,
            created_by: "system".into(),
        },
    )
    .expect("create token");
    let rows = tokens::list_tokens(&storage).expect("list");
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].token_hash, refresh);
    assert_eq!(rows[0].token_hash, tokens::hash_refresh(&refresh));
}
