//! Feedback + admin invalidation handler tests (plan 022). Shares the
//! storage/auth fixture from `tests.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tonic::Request;

use super::feedback::{
    handle_feedback_exploration, handle_invalidate_exploration, handle_review_exploration,
};
use super::handle_persist_exploration;
use super::search::handle_get_exploration_by_id;
use super::search::handle_search_explorations;
use super::tests::{bearer, fixture, persist_request};
use arags_proto::proto::{
    FeedbackExplorationRequest, FeedbackKind, InvalidateExplorationRequest, InvalidateMode,
    ReviewExplorationRequest, SearchExplorationsRequest,
};

#[tokio::test]
async fn feedback_contradictions_auto_retire_and_hide_from_search() {
    let fx = fixture();

    let persisted = handle_persist_exploration(
        &fx.state,
        persist_request(&fx.user_session, vec!["src/b.rs".into()]),
    )
    .await
    .unwrap()
    .into_inner();

    let feedback = |kind: FeedbackKind| {
        let mut r = Request::new(FeedbackExplorationRequest {
            exploration_id: persisted.exploration_id.clone(),
            kind: kind as i32,
        });
        *r.metadata_mut() = bearer(&fx.admin_session);
        r
    };

    let out = handle_feedback_exploration(&fx.state, feedback(FeedbackKind::Confirm))
        .await
        .unwrap()
        .into_inner();
    assert!(out.applied && out.confirmed == 1);

    // Default contradiction_limit is 3: two keep it alive, third retires.
    for _ in 0..2 {
        assert!(
            !handle_feedback_exploration(&fx.state, feedback(FeedbackKind::Contradict))
                .await
                .unwrap()
                .into_inner()
                .auto_retired
        );
    }
    let out = handle_feedback_exploration(&fx.state, feedback(FeedbackKind::Contradict))
        .await
        .unwrap()
        .into_inner();
    assert!(out.auto_retired, "third contradiction must retire the map");

    let get_req = || {
        let mut r = Request::new(arags_proto::proto::GetExplorationByIdRequest {
            exploration_id: persisted.exploration_id.clone(),
        });
        *r.metadata_mut() = bearer(&fx.user_session);
        r
    };
    let got = handle_get_exploration_by_id(&fx.state, get_req())
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    assert_eq!(got.status, "retired");
    assert!(got.body_markdown.contains("## Conexões"));
    assert!(!got.anchored_files.is_empty());
}

#[tokio::test]
async fn invalidate_requires_admin_and_modes_behave() {
    let fx = fixture();

    let persisted = handle_persist_exploration(
        &fx.state,
        persist_request(&fx.user_session, vec!["src/a.rs".into()]),
    )
    .await
    .unwrap()
    .into_inner();

    let invalidate = |session: &str, mode: InvalidateMode| {
        let mut r = Request::new(InvalidateExplorationRequest {
            exploration_id: persisted.exploration_id.clone(),
            mode: mode as i32,
            reason: "revisão manual".into(),
        });
        *r.metadata_mut() = bearer(session);
        r
    };

    // Non-admin denied; map untouched.
    let err = handle_invalidate_exploration(
        &fx.state,
        invalidate(&fx.user_session, InvalidateMode::Stale),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert_eq!(
        fx.storage
            .get_exploration_by_uuid(&persisted.exploration_id)
            .unwrap()
            .unwrap()
            .status,
        "fresh"
    );

    // Admin Stale keeps history with reason.
    let applied = handle_invalidate_exploration(
        &fx.state,
        invalidate(&fx.admin_session, InvalidateMode::Stale),
    )
    .await
    .unwrap()
    .into_inner()
    .applied;
    assert!(applied);
    let row = fx
        .storage
        .get_exploration_by_uuid(&persisted.exploration_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "stale");
    assert!(row.stale_reason.contains(&"revisão manual".to_string()));

    // Admin Delete hard-removes row and vector key.
    assert!(
        handle_invalidate_exploration(
            &fx.state,
            invalidate(&fx.admin_session, InvalidateMode::Delete)
        )
        .await
        .unwrap()
        .into_inner()
        .applied
    );
    assert!(
        fx.storage
            .get_exploration_by_uuid(&persisted.exploration_id)
            .unwrap()
            .is_none()
    );
}

/// Review gate (plan 023): with `[exploration] require_review`, non-admin
/// maps land as `pending_review`, never surface in search, and only an admin
/// can approve (→ fresh) or reject (→ retired) them.
#[tokio::test]
async fn review_gate_holds_non_admin_maps_until_approved() {
    let mut fx = fixture();
    fx.state.config.exploration.require_review = true;

    let persisted = handle_persist_exploration(
        &fx.state,
        persist_request(&fx.user_session, vec!["src/b.rs".into()]),
    )
    .await
    .unwrap()
    .into_inner();
    assert_eq!(persisted.reason, "pending admin review");

    // Row is pending; search must never surface it.
    let row = fx
        .storage
        .get_exploration_by_uuid(&persisted.exploration_id)
        .unwrap()
        .expect("row exists");
    assert_eq!(row.status, "pending_review");

    let search_req = || {
        let mut r = Request::new(SearchExplorationsRequest {
            project: "proj".into(),
            query: "anexos compartilhados\nresumo denso da conexão".into(),
            limit: 5,
            include_stale: true,
        });
        *r.metadata_mut() = bearer(&fx.user_session);
        r
    };
    let hits = handle_search_explorations(&fx.state, search_req())
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(
        hits.iter()
            .all(|h| h.exploration_id != persisted.exploration_id),
        "pending maps must be excluded even when include_stale"
    );

    // Non-admin cannot review.
    let review = |session: &str, approved: bool| {
        let mut r = Request::new(ReviewExplorationRequest {
            exploration_id: persisted.exploration_id.clone(),
            approved,
        });
        *r.metadata_mut() = bearer(session);
        r
    };
    assert_eq!(
        handle_review_exploration(&fx.state, review(&fx.user_session, true))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );

    // Admin approval flips to fresh, which surfaces again.
    let applied = handle_review_exploration(&fx.state, review(&fx.admin_session, true))
        .await
        .unwrap()
        .into_inner();
    assert!(applied.applied);

    let hits = handle_search_explorations(&fx.state, search_req())
        .await
        .unwrap()
        .into_inner()
        .hits;
    assert!(
        hits.iter()
            .any(|h| h.exploration_id == persisted.exploration_id),
        "approved map must surface again"
    );
}
