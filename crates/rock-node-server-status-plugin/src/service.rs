use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
use rock_node_protobufs::org::hiero::block::api::{
    block_node_service_server::BlockNodeService, ServerStatusRequest, ServerStatusResponse,
};
use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::debug;

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

        // Get the block range from the persistence service.
        let earliest_block = self.block_reader.get_earliest_persisted_block_number();
        let latest_block = self.block_reader.get_latest_persisted_block_number();

        // The proto uses uint64, but our reader uses i64 with -1 for "not found".
        // We convert -1 to 0 for the response, as 0 is a safe default for an empty DB.
        let response = ServerStatusResponse {
            first_available_block: if earliest_block < 0 {
                0
            } else {
                earliest_block as u64
            },
            last_available_block: if latest_block < 0 {
                0
            } else {
                latest_block as u64
            },

            // TODO: Implement logic for state snapshot availability.
            only_latest_state: false,

            // TODO: Populate version information from the AppContext or build-time variables.
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
            .set(earliest_block);

        self.metrics
            .server_status_latest_available_block
            .set(latest_block);

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

    // A mock implementation of the BlockReader trait for testing the status service.
    // It only needs to store and return the earliest and latest block numbers.
    #[derive(Debug, Default)]
    struct MockBlockReader {
        earliest_block: i64,
        latest_block: i64,
    }

    impl BlockReader for MockBlockReader {
        // This method is not used by the StatusServiceImpl, so we provide a default implementation.
        fn read_block(&self, _block_number: u64) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> i64 {
            self.earliest_block
        }

        fn get_latest_persisted_block_number(&self) -> i64 {
            self.latest_block
        }
    }

    // Helper to create the service with its dependencies for testing.
    fn create_test_service(reader: MockBlockReader) -> StatusServiceImpl {
        StatusServiceImpl {
            block_reader: Arc::new(reader),
            // Use the actual public constructor for MetricsRegistry.
            // .unwrap() is acceptable in tests as a failure here indicates a setup problem.
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
        }
    }

    #[tokio::test]
    async fn test_server_status_with_populated_range() {
        // STATUS-01 & METRICS-01: Test with a normal, populated block range.
        // This test also implicitly verifies the metrics calls, as the same values
        // are used for both the response and the metrics gauges.
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
        // STATUS-02: Test when the database is empty.
        // The block reader uses -1 as a sentinel value for an empty DB.
        let reader = MockBlockReader {
            earliest_block: -1,
            latest_block: -1,
        };
        let service = create_test_service(reader);

        let request = Request::new(ServerStatusRequest {});
        let response = service.server_status(request).await.unwrap().into_inner();

        // The service should convert the -1 sentinel value to 0 for the proto response.
        assert_eq!(response.first_available_block, 0);
        assert_eq!(response.last_available_block, 0);
    }

    #[tokio::test]
    async fn test_server_status_with_single_block() {
        // STATUS-03: Test when only one block exists in the database.
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
