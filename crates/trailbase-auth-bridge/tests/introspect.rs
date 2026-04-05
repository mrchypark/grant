use trailbase_auth_bridge::{IntrospectionResponse, build_test_app};

#[tokio::test]
async fn introspect_returns_authenticated_trailbase_user_and_rejects_missing_secret() {
    let app = build_test_app("bridge-secret").await.expect("test app");
    let response: IntrospectionResponse = app
        .introspect("alice@example.com", "password123", "bridge-secret")
        .await
        .expect("introspection response");

    assert!(response.valid);
    assert!(response.active);
    assert!(response.verified);
    assert_eq!(response.email.as_deref(), Some("alice@example.com"));
    let err = app
        .introspect("alice@example.com", "password123", "")
        .await
        .expect_err("missing secret should fail");

    assert!(err.contains("401") || err.contains("403"));
}
