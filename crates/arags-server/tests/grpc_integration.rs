//! End-to-end gRPC integration tests for `arags-server`.
//!
//! These tests spin up a *real* `tonic` server (the generated
//! `AragsServiceServer` wrapping `AragsGrpcService`) on an ephemeral port and
//! drive it with the *generated* `AragsServiceClient` over a real channel —
//! exercising the full transport (auth handshake, client-streaming
//! `index_project`, unary `claim_rlm_job`), not just the handler functions in
//! isolation. This closes the gap noted in `agnostic-rlm-rs-b020`, whose client
//! tests could not spin an in-process server.
//!
//! Everything is storage-only: `AppState::with_vector_stores(...)` is built with
//! `None` vector stores and a fallback embedder (no model weights), so the tests
//! are hermetic and reliable in CI (no Ollama / network / weights).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;
use std::time::Instant;

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Server};

use arags_proto::proto::arags_service_client::AragsServiceClient;
use arags_proto::proto::arags_service_server::AragsServiceServer;
use arags_proto::proto::{
    AuthRefreshRequest, ClaimRlmJobRequest, CompleteRlmJobRequest, IndexChunk, IndexFile,
    IndexInit, index_chunk,
};
use arags_server::grpc::AragsGrpcService;
use arags_server::state::AppState;
use arags_storage::Role;
use arags_storage::Storage;
use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, NewRlmJob, rlm_job_key};
use arags_storage::tokens::{NewToken, create_token};

use arags_core::rlm_attestation::sign_rlm_submission;

use arags_server::config::ServerConfig;

/// A running in-process server plus the handles needed to talk to it.
struct TestServer {
    addr: std::net::SocketAddr,
    storage: Storage,
    refresh_token: String,
    handle: tokio::task::JoinHandle<()>,
}

/// Build a storage-only `AppState` (no vector stores, no embedder weights) and
/// an `arags-server` binary config with RLM/exploration hooks off so the test
/// stays hermetic and deterministic.
fn storage_only_state(storage: Storage) -> AppState {
    let mut cfg = ServerConfig::default();
    cfg.exploration.enabled = false;
    cfg.rlm.enabled = false;
    AppState::with_vector_stores(storage.clone(), cfg, None, None, None, None)
        .expect("build AppState")
}

/// Start a real `tonic` server on an ephemeral port and return its address plus
/// a handle to abort it.
async fn spawn_server(state: AppState) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let svc = AragsServiceServer::new(AragsGrpcService::new(state));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(svc)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("server run");
    });
    (addr, handle)
}

/// Seed a refresh token in the storage and start a server. Returns the harness.
async fn start_harness() -> TestServer {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open(dir.path()).expect("open storage");
    // Refresh token the client uses to obtain a session via `AuthRefresh`.
    let (_, refresh_token) = create_token(
        &storage,
        &NewToken {
            username: "itest".to_string(),
            role: Role::NonAdmin,
            created_by: "itest".to_string(),
        },
    )
    .expect("seed refresh token");

    let state = storage_only_state(storage.clone());
    let (addr, handle) = spawn_server(state).await;

    // Keep the tempdir alive for the server's lifetime.
    std::mem::forget(dir);

    TestServer {
        addr,
        storage,
        refresh_token,
        handle,
    }
}

/// Build an authenticated `index_project` request streaming `chunks`.
fn index_request(chunks: Vec<IndexChunk>, session: &str) -> Request<ReceiverStream<IndexChunk>> {
    let (tx, rx) = mpsc::channel::<IndexChunk>(chunks.len().max(2));
    for c in chunks {
        tx.try_send(c).expect("send chunk");
    }
    drop(tx);
    let mut req = Request::new(ReceiverStream::new(rx));
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    req.metadata_mut().insert("authorization", bearer);
    req
}

#[tokio::test]
async fn grpc_index_project_persists_chunks_end_to_end() {
    let server = start_harness().await;

    let mut client = AragsServiceClient::connect(format!("http://{}", server.addr))
        .await
        .expect("connect client");

    // Real auth handshake: AuthRefresh with the seeded refresh token.
    let auth = client
        .auth_refresh(AuthRefreshRequest {
            refresh_token: server.refresh_token.clone(),
        })
        .await
        .expect("auth_refresh");
    let session = auth.into_inner().session_token;

    let chunks = vec![
        IndexChunk {
            body: Some(index_chunk::Body::Init(IndexInit {
                project: "itest".to_string(),
                root_path: "/tmp/itest".to_string(),
                force_include: vec![],
                exclude_patterns: vec![],
            })),
        },
        IndexChunk {
            body: Some(index_chunk::Body::File(IndexFile {
                rel_path: "src/main.rs".to_string(),
                content: b"fn main() {}".to_vec(),
                compressed: false,
                size_bytes: 12,
            })),
        },
    ];

    let start = Instant::now();
    let resp = client
        .index_project(index_request(chunks, &session))
        .await
        .expect("index_project Ok");
    let elapsed_ms = start.elapsed().as_millis() as u64;
    tracing::debug!(?elapsed_ms, "grpc integration: index_project round-trip");

    let inner = resp.into_inner();
    assert!(inner.chunks_created >= 1, "expected >=1 chunk persisted");

    // Assert the SAME storage reflects the indexing end-to-end.
    let total = server.storage.count_all_chunks().expect("count_all_chunks");
    assert!(total > 0, "indexed chunks must be persisted to storage");

    // Second RPC over the same channel: claim an RLM job after seeding one.
    let job = NewRlmJob {
        buffer_id: Some(1),
        project: "itest".to_string(),
        level: 1,
        subject: "src/main.rs".to_string(),
        payload: "{}".to_string(),
        priority: 5,
        quorum_slots: 1,
    };
    let (job_id, _gen) = server
        .storage
        .enqueue_rlm_job(&job, &[])
        .expect("seed rlm job");
    assert!(job_id > 0);

    let mut claim_req = Request::new(ClaimRlmJobRequest {
        lease_ms: DEFAULT_RLM_LEASE_MS,
        max_level: 0,
    });
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    claim_req.metadata_mut().insert("authorization", bearer);
    let claim = client
        .claim_rlm_job(claim_req)
        .await
        .expect("claim_rlm_job Ok");
    assert!(
        claim.into_inner().available,
        "pending rlm job must be claimable over gRPC"
    );

    server.handle.abort();
}

/// Issue `agnostic-rlm-rs-7222`: a mutating RPC must record an append-only
/// audit-log entry attributed to the authenticated user. `index_project` is the
/// simplest mutating path that needs no extra config; we drive a real gRPC
/// round-trip and assert the storage reflects the audit row.
#[tokio::test]
async fn grpc_index_project_writes_audit_log() {
    let server = start_harness().await;
    let mut client = connect_client(server.addr).await;

    let auth = client
        .auth_refresh(AuthRefreshRequest {
            refresh_token: server.refresh_token.clone(),
        })
        .await
        .expect("auth_refresh");
    let session = auth.into_inner().session_token;

    let chunks = vec![
        IndexChunk {
            body: Some(index_chunk::Body::Init(IndexInit {
                project: "itest".to_string(),
                root_path: "/tmp/itest".to_string(),
                force_include: vec![],
                exclude_patterns: vec![],
            })),
        },
        IndexChunk {
            body: Some(index_chunk::Body::File(IndexFile {
                rel_path: "src/main.rs".to_string(),
                content: b"fn main() {}".to_vec(),
                compressed: false,
                size_bytes: 12,
            })),
        },
    ];

    let resp = client
        .index_project(index_request(chunks, &session))
        .await
        .expect("index_project Ok");
    assert!(resp.into_inner().chunks_created >= 1);

    // The SAME storage must carry an audit row for user "itest", action "index".
    let entries = server
        .storage
        .list_audit_log("", "itest", 100)
        .expect("list audit log");
    let idx: Vec<_> = entries.iter().filter(|e| e.action == "index").collect();
    assert!(!idx.is_empty(), "index_project must write an audit entry");
    assert_eq!(idx[0].username, "itest");
    assert_eq!(idx[0].project, "itest");

    server.handle.abort();
}

/// Connect a fresh gRPC client to a running harness.
async fn connect_client(
    addr: std::net::SocketAddr,
) -> arags_proto::proto::arags_service_client::AragsServiceClient<tonic::transport::Channel> {
    AragsServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("connect client")
}

/// Seed a quorum (n=3) RLM job, claim it as the session's volunteer, and return
/// `(job_id, generation)` so the caller can complete it.
async fn seed_and_claim_rlm_job(server: &TestServer, session: &str) -> (i64, i64) {
    let job = NewRlmJob {
        buffer_id: Some(1),
        project: "itest".to_string(),
        level: 1,
        subject: "src/main.rs".to_string(),
        payload: "{}".to_string(),
        priority: 5,
        quorum_slots: 3,
    };
    let (job_id, _gen) = server
        .storage
        .enqueue_rlm_job(&job, &[])
        .expect("seed rlm job");
    assert!(job_id > 0);

    let mut client = connect_client(server.addr).await;
    let mut claim_req = Request::new(ClaimRlmJobRequest {
        lease_ms: DEFAULT_RLM_LEASE_MS,
        max_level: 0,
    });
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    claim_req.metadata_mut().insert("authorization", bearer);
    let claimed = client
        .claim_rlm_job(claim_req)
        .await
        .expect("claim_rlm_job Ok")
        .into_inner();
    assert!(claimed.available, "seeded job must be claimable");
    (job_id, claimed.generation)
}

#[tokio::test]
async fn rlm_submission_valid_hmac_accepted() {
    let server = start_harness().await;
    let mut client = connect_client(server.addr).await;

    let auth = client
        .auth_refresh(AuthRefreshRequest {
            refresh_token: server.refresh_token.clone(),
        })
        .await
        .expect("auth_refresh");
    let session = auth.into_inner().session_token;

    let (job_id, generation) = seed_and_claim_rlm_job(&server, &session).await;
    let summary = "The module parses JSON configuration files and exposes a typed API.";

    // Correctly-signed submission (issue `agnostic-rlm-rs-64af`).
    let hmac = sign_rlm_submission(&session, job_id, generation, summary);

    let mut req = Request::new(CompleteRlmJobRequest {
        job_id,
        generation,
        summary_text: summary.to_string(),
        model: "llama3.2".to_string(),
        template_version: "arlm-v1".to_string(),
        token_count: 10,
        submission_hmac: hmac,
    });
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    req.metadata_mut().insert("authorization", bearer);

    let _resp = client
        .complete_rlm_job(req)
        .await
        .expect("valid HMAC accepted (Ok)");
    // The handler returns Ok even when the quorum is still pending; the real
    // assertion is that the submission below was staged (not rejected at the edge).

    let key = rlm_job_key("itest", 1, "src/main.rs");
    let staged = server
        .storage
        .list_pending("itest", "rlm_node", &key)
        .expect("list pending");
    assert_eq!(staged.len(), 1, "valid HMAC submission must be staged");
    assert_eq!(staged[0].candidate_text, summary);

    server.handle.abort();
}

#[tokio::test]
async fn rlm_submission_invalid_hmac_rejected() {
    let server = start_harness().await;
    let mut client = connect_client(server.addr).await;

    let auth = client
        .auth_refresh(AuthRefreshRequest {
            refresh_token: server.refresh_token.clone(),
        })
        .await
        .expect("auth_refresh");
    let session = auth.into_inner().session_token;

    let (job_id, generation) = seed_and_claim_rlm_job(&server, &session).await;
    let summary = "The module renders pixel shaders for the GPU compute pipeline.";

    // Tampered / missing HMAC: the server must reject the submission at the edge.
    let bad_hmac = "deadbeef".to_string();

    let mut req = Request::new(CompleteRlmJobRequest {
        job_id,
        generation,
        summary_text: summary.to_string(),
        model: "llama3.2".to_string(),
        template_version: "arlm-v1".to_string(),
        token_count: 10,
        submission_hmac: bad_hmac,
    });
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    req.metadata_mut().insert("authorization", bearer);

    let result = client.complete_rlm_job(req).await;
    assert!(
        result.is_err(),
        "invalid HMAC must reject the submission (handler errors)"
    );

    // Nothing was staged.
    let key = rlm_job_key("itest", 1, "src/main.rs");
    let staged = server
        .storage
        .list_pending("itest", "rlm_node", &key)
        .expect("list pending");
    assert!(staged.is_empty(), "rejected submission must NOT be staged");

    server.handle.abort();
}

#[tokio::test]
async fn grpc_disconnect_after_init_keeps_rlm_claim_working() {
    let server = start_harness().await;

    let mut client = AragsServiceClient::connect(format!("http://{}", server.addr))
        .await
        .expect("connect client");

    let auth = client
        .auth_refresh(AuthRefreshRequest {
            refresh_token: server.refresh_token.clone(),
        })
        .await
        .expect("auth_refresh");
    let session = auth.into_inner().session_token;

    // Seed a PENDING RLM job (the row that must remain claimable after a
    // mid-index disconnect — issue `agnostic-rlm-rs-ccc3`).
    let job = NewRlmJob {
        buffer_id: Some(1),
        project: "itest".to_string(),
        level: 1,
        subject: "src/main.rs".to_string(),
        payload: "{}".to_string(),
        priority: 5,
        quorum_slots: 1,
    };
    let (job_id, _gen) = server
        .storage
        .enqueue_rlm_job(&job, &[])
        .expect("seed rlm job");
    assert!(job_id > 0);

    // Stream sends Init then ENDS (simulated client disconnect right after Init).
    let chunks = vec![IndexChunk {
        body: Some(index_chunk::Body::Init(IndexInit {
            project: "itest".to_string(),
            root_path: "/tmp/itest".to_string(),
            force_include: vec![],
            exclude_patterns: vec![],
        })),
    }];
    let resp = client
        .index_project(index_request(chunks, &session))
        .await
        .expect("index_project must return Ok on clean disconnect");
    assert!(resp.into_inner().chunks_created >= 0);

    // Follow-up RLM claim must still succeed over the real transport — proving
    // the clean-disconnect fix holds end-to-end (no leaked connection/transaction
    // that would break subsequent RPCs until restart).
    let mut claim_req = Request::new(ClaimRlmJobRequest {
        lease_ms: DEFAULT_RLM_LEASE_MS,
        max_level: 0,
    });
    let bearer = MetadataValue::from_str(&format!("Bearer {session}")).expect("bearer");
    claim_req.metadata_mut().insert("authorization", bearer);
    let claim = client
        .claim_rlm_job(claim_req)
        .await
        .expect("claim_rlm_job Ok after disconnect");
    assert!(
        claim.into_inner().available,
        "rlm claim must work after disconnect over gRPC"
    );

    server.handle.abort();
}
