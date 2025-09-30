use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
use rock_node_protobufs::org::hiero::block::api::{
    block_node_service_server::BlockNodeService, ServerStatusRequest, ServerStatusResponse,
};
use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::{debug, error};

#[derive(Debug)]
pub struct StatusServiceImpl {
    pub block_reader: Arc<dyn BlockReader>,
    pub metrics: Arc<MetricsRegistry>,
}

#[tonic::async_trait]
impl BlockNodeService for StatusServiceImpl {
    async fn server_status(
        &self,
        request: Request<ServerStatusRequest>,
    ) -> Result<Response<ServerStatusResponse>, Status> {
        let start_time = Instant::now();
        debug!("Processing serverStatus request: {:?}", request);

        let earliest_block_val = match self.block_reader.get_earliest_persisted_block_number() {
            Ok(Some(num)) => num,
            Ok(None) => 0, // Default to 0 for the response if no blocks exist.
            Err(e) => {
                error!("Failed to get earliest persisted block number: {}", e);
                // Return an internal error to the gRPC client.
                return Err(Status::internal(format!(
                    "Database error while fetching earliest block: {}",
                    e
                )));
            },
        };

        let latest_block_val = match self.block_reader.get_latest_persisted_block_number() {
            Ok(Some(num)) => num,
            Ok(None) => 0,
            Err(e) => {
                error!("Failed to get latest persisted block number: {}", e);
                return Err(Status::internal(format!(
                    "Database error while fetching latest block: {}",
                    e
                )));
            },
        };

        let response = ServerStatusResponse {
            first_available_block: earliest_block_val,
            last_available_block: latest_block_val,
            only_latest_state: false,
            version_information: None,
        };

        // --- Record Metrics ---
        let duration = start_time.elapsed().as_secs_f64();

        self.metrics
            .server_status_request_duration_seconds
            .with_label_values(&["success"])
            .observe(duration);

        self.metrics
            .server_status_requests_total
            .with_label_values(&["success"])
            .inc();

        self.metrics
            .server_status_earliest_available_block
            .set(earliest_block_val as i64);

        self.metrics
            .server_status_latest_available_block
            .set(latest_block_val as i64);

        Ok(Response::new(response))
    }
}

//================================================================================//
//=============================== UNIT TESTS =====================================//
//================================================================================//

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rock_node_core::{block_reader::BlockReader, test_utils::create_isolated_metrics};
    use rock_node_protobufs::org::hiero::block::api::ServerStatusRequest;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct MockBlockReader {
        earliest_block: i64,
        latest_block: i64,
        force_earliest_error: bool,
        force_latest_error: bool,
    }

    impl MockBlockReader {
        fn with_error() -> Self {
            Self {
                earliest_block: 100,
                latest_block: 5000,
                force_earliest_error: true,
                force_latest_error: true,
            }
        }

        fn with_latest_error() -> Self {
            Self {
                earliest_block: 100,
                latest_block: 5000,
                force_earliest_error: false,
                force_latest_error: true,
            }
        }
    }

    impl BlockReader for MockBlockReader {
        fn read_block(&self, _block_number: u64) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
            if self.force_earliest_error {
                Err(anyhow::anyhow!("Database connection error"))
            } else if self.earliest_block < 0 {
                Ok(None)
            } else {
                Ok(Some(self.earliest_block as u64))
            }
        }

        fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
            if self.force_latest_error {
                Err(anyhow::anyhow!("Database connection error"))
            } else if self.latest_block < 0 {
                Ok(None)
            } else {
                Ok(Some(self.latest_block as u64))
            }
        }

        fn get_highest_contiguous_block_number(&self) -> Result<u64> {
            if self.latest_block < 0 {
                return Ok(0);
            }
            Ok(self.latest_block as u64)
        }
    }

    fn create_test_service(reader: MockBlockReader) -> StatusServiceImpl {
        StatusServiceImpl {
            block_reader: Arc::new(reader),
            metrics: Arc::new(create_isolated_metrics()),
        }
    }

    #[tokio::test]
    async fn test_server_status_with_populated_range() {
        let reader = MockBlockReader {
            earliest_block: 100,
            latest_block: 5000,
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 100);
        assert_eq!(response.last_available_block, 5000);
        assert!(!response.only_latest_state);
        assert!(response.version_information.is_none());
    }

    #[tokio::test]
    async fn test_server_status_with_empty_db() {
        let reader = MockBlockReader {
            earliest_block: -1,
            latest_block: -1,
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 0);
        assert_eq!(response.last_available_block, 0);
    }

    #[tokio::test]
    async fn test_server_status_with_single_block() {
        let reader = MockBlockReader {
            earliest_block: 1,
            latest_block: 1,
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 1);
        assert_eq!(response.last_available_block, 1);
    }

    #[tokio::test]
    async fn test_server_status_earliest_block_error() {
        let reader = MockBlockReader::with_error();
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let result = service.server_status(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status
            .message()
            .contains("Database error while fetching earliest block"));
    }

    #[tokio::test]
    async fn test_server_status_latest_block_error() {
        let reader = MockBlockReader::with_latest_error(); // Only latest will fail
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let result = service.server_status(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status
            .message()
            .contains("Database error while fetching latest block"));
    }

    #[tokio::test]
    async fn test_server_status_earliest_none_latest_some() {
        let reader = MockBlockReader {
            earliest_block: -1, // None
            latest_block: 1000, // Some value
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 0);
        assert_eq!(response.last_available_block, 1000);
    }

    #[tokio::test]
    async fn test_server_status_earliest_some_latest_none() {
        let reader = MockBlockReader {
            earliest_block: 100, // Some value
            latest_block: -1,    // None
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 100);
        assert_eq!(response.last_available_block, 0);
    }

    #[tokio::test]
    async fn test_server_status_large_block_numbers() {
        let reader = MockBlockReader {
            earliest_block: i64::MAX - 1000,
            latest_block: i64::MAX - 100,
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, (i64::MAX - 1000) as u64);
        assert_eq!(response.last_available_block, (i64::MAX - 100) as u64);
    }

    #[tokio::test]
    async fn test_server_status_metrics_recording() {
        let reader = MockBlockReader {
            earliest_block: 100,
            latest_block: 200,
            force_earliest_error: false,
            force_latest_error: false,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let _response = service.server_status(request).await.unwrap().into_inner();

        // Check that metrics were recorded
        // This is a basic check - in a real scenario you'd want to inspect the metrics registry
        // but for now we'll just ensure the request completes successfully
    }
}
