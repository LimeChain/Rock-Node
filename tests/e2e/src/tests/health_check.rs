use crate::common::TestContext;
use anyhow::Result;
use serial_test::serial;

/// Test that the node starts and the health check is OK.
#[tokio::test]
#[serial]
async fn test_node_starts_and_health_check_is_ok() -> Result<()> {
    let ctx = TestContext::new().await?;

    let port = ctx.http_port().await?;
    let health_url = format!("http://localhost:{}/livez", port);

    let client = reqwest::Client::new();
    let response = client.get(&health_url).send().await?;

    assert!(response.status().is_success(), "Health check failed");
    let body = response.text().await?;
    assert_eq!(body, "OK");

    Ok(())
}

/// Test that multiple containers can run independently.
#[tokio::test]
#[serial]
async fn test_multiple_containers_independence() -> Result<()> {
    let ctx1 = TestContext::new().await?;
    let port1 = ctx1.http_port().await?;

    let ctx2 = TestContext::new().await?;
    let port2 = ctx2.http_port().await?;

    assert_ne!(
        port1, port2,
        "Containers should have different ports for independence"
    );

    let client = reqwest::Client::new();

    let response1 = client
        .get(format!("http://localhost:{}/livez", port1))
        .send()
        .await?;
    let response2 = client
        .get(format!("http://localhost:{}/livez", port2))
        .send()
        .await?;

    assert!(response1.status().is_success());
    assert!(response2.status().is_success());

    Ok(())
}
