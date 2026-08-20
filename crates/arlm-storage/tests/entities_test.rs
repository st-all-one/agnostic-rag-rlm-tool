#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use arlm_storage::Storage;

#[test]
fn test_extract_entities_rust() {
    let content = "
        use crate::auth::validate_token;
        use std::collections::HashMap;

        fn check_session(req: &Request) -> Result<Session> {
            let token = extract_token(req)?;
            validate_token(token)
        }

        struct UserContext {
            user_id: i64,
            session: Session,
        }
    ";

    let entities = Storage::extract_entities(content, "src/auth/middleware.rs");
    assert!(!entities.is_empty());
    assert!(entities.contains(&"check_session".to_string()));
    assert!(entities.contains(&"validate_token".to_string()));
    assert!(entities.contains(&"middleware".to_string()));
}

#[test]
fn test_extract_entities_python() {
    let content = "
        from flask import Flask, request
        import json

        def authenticate_user(user_id: int) -> bool:
            return True

        class AuthService:
            pass
    ";

    let entities = Storage::extract_entities(content, "auth/service.py");
    assert!(!entities.is_empty());
    assert!(entities.contains(&"authenticate_user".to_string()));
    assert!(entities.contains(&"service".to_string()));
}

#[test]
fn test_extract_entities_dedup() {
    let content = "fn validate_token() { validate_token(); }";
    let entities = Storage::extract_entities(content, "token.rs");
    let count = entities.iter().filter(|e| *e == "validate_token").count();
    assert_eq!(count, 1);
}

#[test]
fn test_extract_entities_max_limit() {
    let mut content = String::new();
    for i in 0..20 {
        use std::fmt::Write;
        let _ = writeln!(content, "fn function_{i}() {{ }}");
    }
    let entities = Storage::extract_entities(&content, "many.rs");
    assert!(entities.len() <= 10);
}

#[test]
fn test_extract_entities_file_stem() {
    let entities = Storage::extract_entities("", "src/main.rs");
    assert!(entities.contains(&"main".to_string()));
}
