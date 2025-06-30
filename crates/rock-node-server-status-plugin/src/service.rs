use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
use rock_node_protobufs::org::hiero::block::api::{
    block_node_service_server::BlockNodeService, ServerStatusRequest, ServerStatusResponse,
};
use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::{debug, error}; // FIX: Added error to tracing imports

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

        // FIX: Handle the new Result<Option<u64>> return type properly.
        // We need to convert this into two separate values:
        // 1. A u64 for the gRPC response (0 for None).
        // 2. An i64 for the Prometheus gauge (-1 for None).

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
            }
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
            }
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

        // FIX: Convert the values to i64 for the gauge, using -1 as the sentinel for "None".
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
    use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
    use rock_node_protobufs::org::hiero::block::api::ServerStatusRequest;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct MockBlockReader {
        // We can still use i64 internally for the mock's state.
        earliest_block: i64,
        latest_block: i64,
    }

    // FIX: Update the mock to implement the new, correct trait signatures.
    impl BlockReader for MockBlockReader {
        fn read_block(&self, _block_number: u64) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
            if self.earliest_block < 0 {
                Ok(None)
            } else {
                Ok(Some(self.earliest_block as u64))
            }
        }

        fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
            if self.latest_block < 0 {
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
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
        }
    }

    #[tokio::test]
    async fn test_server_status_with_populated_range() {
        let reader = MockBlockReader {
            earliest_block: 100,
            latest_block: 5000,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 100);
        assert_eq!(response.last_available_block, 5000);
    }

    #[tokio::test]
    async fn test_server_status_with_empty_db() {
        let reader = MockBlockReader {
            earliest_block: -1,
            latest_block: -1,
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
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        assert_eq!(response.first_available_block, 1);
        assert_eq!(response.last_available_block, 1);
    }
}
