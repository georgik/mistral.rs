//! Wizard of Oz integration tests
//!
//! Tests the WOZ server endpoints and tool dispatch flow.

use mistralrs_server_core::wizard_ofoz;
use reqwest::Client;
use serde_json::json;
use tokio::net::TcpListener;

#[tokio::test]
async fn test_wizard_dispatch_endpoint() {
    // Find available port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Start WOZ server
    let app = wizard_ofoz::create_wizard_router();
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Test 1: Root endpoint returns HTML
    let resp = client.get(&format!("{}/", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let html = resp.text().await.unwrap();
    assert!(html.contains("Wizard of Oz"));

    // Test 2: Status endpoint
    let resp = client.get(&format!("{}/status", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["pending_calls"].as_array().unwrap().len(), 0);

    // Test 3: Dispatch endpoint with tool call
    let tool_call = json!({
        "name": "get_weather",
        "arguments": {
            "location": "Prague"
        }
    });

    let resp = client.post(&format!("{}/dispatch", base_url))
        .json(&tool_call)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let response: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(response["status"], "pending");
    assert!(response["call_id"].is_string());
    let call_id = response["call_id"].as_str().unwrap();

    // Test 4: Poll endpoint returns waiting
    let resp = client.get(&format!("{}/poll/{}", base_url, call_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202); // ACCEPTED = waiting

    // Test 5: Respond to tool call
    let wizard_response = json!({
        "call_id": call_id,
        "response": "{\"temperature\": 20, \"condition\": \"sunny\"}",
        "use_model": false
    });

    let resp = client.post(&format!("{}/respond", base_url))
        .json(&wizard_response)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Test 6: Poll again, should get response
    let resp = client.get(&format!("{}/poll/{}", base_url, call_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200); // OK = has response

    let body = resp.text().await.unwrap();
    assert!(body.contains("temperature"));
}

#[tokio::test]
async fn test_wizard_dispatch_invalid_method() {
    // GET on /dispatch should return 405
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let app = wizard_ofoz::create_wizard_router();
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = Client::new();

    // GET request should fail with 405
    let resp = client.get(&format!("http://127.0.0.1:{}/dispatch", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
}

#[tokio::test]
async fn test_wizard_dispatch_format_validation() {
    // Test that server accepts the correct HTTP tool dispatch format
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let app = wizard_ofoz::create_wizard_router();
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = Client::new();

    // Correct format: {"name": "...", "arguments": {...}}
    let tool_call = json!({
        "name": "search_web",
        "arguments": {
            "query": "rust testing"
        }
    });

    let resp = client.post(&format!("http://127.0.0.1:{}/dispatch", port))
        .json(&tool_call)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let response: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(response["status"], "pending");
}
