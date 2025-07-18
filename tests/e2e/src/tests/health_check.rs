use crate::common::TestContext;
use anyhow::Result;
use std::process::Command;

/// Ensure the Docker image exists.
fn ensure_docker_image_exists() -> Result<()> {
    let check_output = Command::new("docker")
        .args(&["image", "inspect", "rock-node-e2e:latest"])
        .output()?;

    if check_output.status.success() {
        println!("Docker image 'rock-node-e2e:latest' already exists, skipping build.");
        return Ok(());
    }

    println!("Building Docker image for E2E tests...");
    let output = Command::new("docker")
        .args(&[
            "build",
            "-t",
            "rock-node-e2e:latest",
            "-f",
            "Dockerfile.e2e",
            "../..",
        ])
        .current_dir(".")
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to build Docker image: {}", stderr);
    }

    println!("Docker image built successfully.");
    Ok(())
}

/// Test that the node starts and the health check is OK.
#[tokio::test]
async fn test_node_starts_and_health_check_is_ok() -> Result<()> {
    ensure_docker_image_exists()?;

    let ctx = TestContext::new().await?;

    let port = ctx.http_port().await?;
    let health_url = format!("http://localhost:{}/livez", port);
    println!("🔍 Probing health endpoint: {}", health_url);

    let client = reqwest::Client::new();
    let response = client.get(&health_url).send().await?;

    assert!(response.status().is_success(), "Health check failed");
    let body = response.text().await?;
    assert_eq!(body, "OK");

    println!("✅ Health Check Test Passed!");

    Ok(())
}

/// Test that multiple containers can run independently.
#[tokio::test]
async fn test_multiple_containers_independence() -> Result<()> {
    ensure_docker_image_exists()?;

    // Create first independent container
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
        .get(&format!("http://localhost:{}/livez", port1))
        .send()
        .await?;
    let response2 = client
        .get(&format!("http://localhost:{}/livez", port2))
        .send()
        .await?;

    assert!(response1.status().is_success());
    assert!(response2.status().is_success());

    println!(
        "✅ Independence Test Passed! Containers running on ports {} and {}",
        port1, port2
    );

    Ok(())
}
