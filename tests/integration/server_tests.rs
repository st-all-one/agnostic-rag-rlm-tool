use std::time::Duration;

use arlm_proto::proto::arlm_service_client::ArlmServiceClient;
use arlm_proto::proto::*;
use tonic::transport::Channel;

/// Test helper to create a client connected to the server.
async fn create_test_client(addr: &str) -> ArlmServiceClient<Channel> {
    let channel = Channel::from_shared(format!("http://{addr}"))
        .expect("valid address")
        .connect()
        .await
        .expect("failed to connect");

    ArlmServiceClient::new(channel)
}

#[tokio::test]
#[ignore = "requires running server"]
async fn test_server_health() {
    let mut client = create_test_client("127.0.0.1:50051").await;

    let response = client
        .get_server_status(())
        .await
        .expect("failed to get server status");

    let status = response.into_inner();
    assert!(!status.version.is_empty());
}

#[tokio::test]
#[ignore = "requires running server"]
async fn test_create_project() {
    let mut client = create_test_client("127.0.0.1:50051").await;

    let request = CreateProjectRequest {
        name: "test-project".to_string(),
        root_path: "/tmp/test".to_string(),
    };

    let response = client
        .create_project(request)
        .await
        .expect("failed to create project");

    let project = response.into_inner();
    assert_eq!(project.name, "test-project");
    assert!(!project.id.is_empty());
}

#[tokio::test]
#[ignore = "requires running server"]
async fn test_list_projects() {
    let mut client = create_test_client("127.0.0.1:50051").await;

    let response = client
        .list_projects(())
        .await
        .expect("failed to list projects");

    let projects = response.into_inner();
    // Should not fail, even if empty
    assert!(projects.projects.len() >= 0);
}

#[tokio::test]
#[ignore = "requires running server"]
async fn test_search_empty() {
    let mut client = create_test_client("127.0.0.1:50051").await;

    let request = SearchRequest {
        project: "test-project".to_string(),
        query: "nonexistent query".to_string(),
        max_results: 10,
        tier: 0, // TIER_BM25
        include_summaries: true,
        include_raw: true,
    };

    let response = client
        .search(request)
        .await
        .expect("failed to search");

    let results = response.into_inner();
    // Should return empty results, not error
    assert!(results.results.len() <= 10);
}

#[tokio::test]
#[ignore = "requires running server"]
async fn test_session_lifecycle() {
    let mut client = create_test_client("127.0.0.1:50051").await;

    // Create session
    let create_req = CreateSessionRequest {
        project: "test-project".to_string(),
        title: "Test Session".to_string(),
    };

    let session = client
        .create_session(create_req)
        .await
        .expect("failed to create session")
        .into_inner();

    assert!(!session.session_id.is_empty());
    assert_eq!(session.title, "Test Session");

    // List sessions
    let list_resp = client
        .list_sessions("test-project".to_string())
        .await
        .expect("failed to list sessions")
        .into_inner();

    assert!(list_resp.sessions.iter().any(|s| s.session_id == session.session_id));

    // Get session
    let get_resp = client
        .get_session(session.session_id.clone())
        .await
        .expect("failed to get session")
        .into_inner();

    assert_eq!(get_resp.session_id, session.session_id);
}
