#![allow(
    unsafe_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::float_cmp,
    clippy::duration_suboptimal_units,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use arags_cli::output::json::JsonOutput;

#[test]
fn test_json_output_ok() {
    let output = JsonOutput::ok();
    let s = output.status.clone();
    assert_eq!(s, "ok");
}

#[test]
fn test_json_output_with_data() {
    let output = JsonOutput::ok().with_data(serde_json::json!({"count": 42}));
    let json = output.to_json_string();
    assert!(json.contains("42"));
}
