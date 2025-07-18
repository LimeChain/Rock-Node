use anyhow::Result;
use std::path::PathBuf;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    CopyDataSource, GenericImage, ImageExt,
};

pub struct TestContext {
    pub container: ContainerAsync<GenericImage>,
}

impl TestContext {
    /// Creates a new `TestContext` by starting an isolated Docker container
    /// running the `rock-node-e2e` image. This method:
    ///
    /// - Loads the E2E test configuration from `config/config.e2e.toml`.
    /// - Generates a unique container name for test isolation.
    /// - Starts a container with the configuration file mounted inside.
    /// - Waits for the node to signal readiness via a specific stdout message.
    ///
    /// Returns a `TestContext` containing the running container, which can be
    /// used to interact with the test node (e.g., to retrieve its HTTP port).
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file cannot be read, the container
    /// fails to start, or any required Docker operation fails.
    pub async fn new() -> Result<TestContext> {
        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../config/config.e2e.toml")
            .canonicalize()?;

        // Read the config file content
        let config_content = std::fs::read_to_string(&config_path)?;

        // Generate unique container name for isolation (Docker-friendly format)
        let thread_id = format!("{:?}", std::thread::current().id());
        let thread_id_clean = thread_id.replace("ThreadId(", "").replace(")", "");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let container_name = format!("rock-node-test-{}-{}", thread_id_clean, timestamp);

        println!("🚀 Starting isolated container: {}", container_name);

        let container = GenericImage::new("rock-node-e2e", "latest")
            .with_exposed_port(8080.tcp())
            .with_wait_for(WaitFor::message_on_stdout(
                "Rock Node running successfully.",
            ))
            .with_copy_to(
                "/config/config.toml",
                CopyDataSource::Data(config_content.into_bytes()),
            )
            .with_cmd(vec![
                "--config-path".to_string(),
                "/config/config.toml".to_string(),
            ])
            .with_container_name(&container_name)
            .start()
            .await?;

        println!("✅ Container started successfully: {}", container_name);
        Ok(TestContext { container })
    }

    /// Get the HTTP port of the container.
    pub async fn http_port(&self) -> Result<u16> {
        let port = self.container.get_host_port_ipv4(8080).await?;
        Ok(port)
    }
}
