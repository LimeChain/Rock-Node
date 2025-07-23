use prost::Message;
use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::Block,
    org::hiero::block::api::{
        block_access_service_server::BlockAccessService, block_response, BlockRequest,
        BlockResponse,
    },
};
use std::{convert::TryFrom, sync::Arc, time::Instant};
use tonic::{Request, Response, Status};
use tracing::{debug, error, warn};

#[derive(Debug)]
pub struct BlockAccessServiceImpl {
    pub block_reader: Arc<dyn BlockReader>,
    pub metrics: Arc<MetricsRegistry>,
}

fn code_to_string(code: block_response::Code) -> &'static str {
    match code {
        block_response::Code::Success => "Success",
        block_response::Code::InvalidRequest => "InvalidRequest",
        block_response::Code::NotFound => "NotFound",
        block_response::Code::NotAvailable => "NotAvailable",
        block_response::Code::Unknown => "Unknown",
    }
}

#[tonic::async_trait]
impl BlockAccessService for BlockAccessServiceImpl {
    async fn get_block(
        &self,
        request: Request<BlockRequest>,
    ) -> Result<Response<BlockResponse>, Status> {
        let start_time = Instant::now();
        let inner_request = request.into_inner();
        debug!("Processing getBlock request: {:?}", inner_request);

        // Step 1: Determine which block number to fetch.
        let (block_number_to_fetch, request_type) =
            match self.get_target_block_number(&inner_request) {
                Ok((num, req_type)) => (num, req_type),
                Err(response) => {
                    // If parsing the request fails, record metrics and exit early.
                    return self.record_metrics(response, start_time, "invalid");
                }
            };

        // Step 2: Try to read the block from the persistence layer.
        let block_read_result = match self.block_reader.read_block(block_number_to_fetch) {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "Database error fetching block #{}: {}",
                    block_number_to_fetch, e
                );
                let response = BlockResponse {
                    status: block_response::Code::Unknown as i32,
                    block: None,
                };
                return self.record_metrics(response, start_time, request_type);
            }
        };

        // Step 3: Process the result of the read operation.
        let response = match block_read_result {
            Some(block_bytes) => self.handle_found_block(block_number_to_fetch, &block_bytes),
            None => self.handle_not_found_block(block_number_to_fetch),
        };

        // Step 4: Record metrics and return the final response.
        self.record_metrics(response, start_time, request_type)
    }
}

impl BlockAccessServiceImpl {
    /// Helper to parse the request and determine the target block number.
    fn get_target_block_number(
        &self,
        request: &BlockRequest,
    ) -> Result<(u64, &'static str), BlockResponse> {
        match &request.block_specifier {
            Some(spec) => match spec {
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::BlockNumber(num) => {
                    // u64::MAX is the specifier for "latest" when using block_number field
                    if *num == u64::MAX {
                        self.get_latest_block_number().map(|n| (n, "latest"))
                    } else {
                        Ok((*num, "by_number"))
                    }
                },
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::RetrieveLatest(true) => {
                    self.get_latest_block_number().map(|n| (n, "latest"))
                },
                _ => {
                    warn!("Invalid block_specifier in request");
                    Err(BlockResponse {
                        status: block_response::Code::InvalidRequest as i32,
                        block: None,
                    })
                }
            },
            None => {
                warn!("Missing block_specifier in request");
                Err(BlockResponse {
                    status: block_response::Code::InvalidRequest as i32,
                    block: None,
                })
            }
        }
    }

    /// Helper to fetch the latest block number, converting errors/none to a BlockResponse.
    fn get_latest_block_number(&self) -> Result<u64, BlockResponse> {
        match self.block_reader.get_latest_persisted_block_number() {
            Ok(Some(num)) => Ok(num),
            Ok(None) => {
                debug!("DB is empty; cannot get latest block.");
                Err(BlockResponse {
                    status: block_response::Code::NotFound as i32,
                    block: None,
                })
            }
            Err(e) => {
                error!("Failed to get latest block number: {}", e);
                Err(BlockResponse {
                    status: block_response::Code::Unknown as i32,
                    block: None,
                })
            }
        }
    }

    /// Helper to process block bytes that were successfully read from storage.
    fn handle_found_block(&self, block_number: u64, block_bytes: &[u8]) -> BlockResponse {
        if block_bytes.is_empty() {
            warn!("Block #{} found but content is empty.", block_number);
            return BlockResponse {
                status: block_response::Code::Unknown as i32,
                block: None,
            };
        }

        match Block::decode(block_bytes) {
            Ok(block) => {
                debug!(
                    "Successfully decoded block #{}, returning SUCCESS.",
                    block_number
                );
                BlockResponse {
                    status: block_response::Code::Success as i32,
                    block: Some(block),
                }
            }
            Err(e) => {
                error!("Failed to decode block #{}: {}", block_number, e);
                BlockResponse {
                    status: block_response::Code::Unknown as i32,
                    block: None,
                }
            }
        }
    }

    /// Helper to determine why a block was not found.
    fn handle_not_found_block(&self, block_number: u64) -> BlockResponse {
        // Unpack the range to determine if it's NotFound vs NotAvailable
        let earliest = self
            .block_reader
            .get_earliest_persisted_block_number()
            .ok()
            .flatten();
        let latest = self
            .block_reader
            .get_latest_persisted_block_number()
            .ok()
            .flatten();

        let status_code = if let (Some(e), Some(l)) = (earliest, latest) {
            if block_number < e || block_number > l {
                debug!(
                    "Block #{} is outside of this node's range [{} - {}].",
                    block_number, e, l
                );
                block_response::Code::NotAvailable
            } else {
                warn!(
                    "Block #{} not found but is within range [{} - {}].",
                    block_number, e, l
                );
                block_response::Code::NotFound
            }
        } else {
            // If we can't determine the range, it's simply not found.
            debug!(
                "Block #{} not found and node range is not available.",
                block_number
            );
            block_response::Code::NotFound
        };

        BlockResponse {
            status: status_code as i32,
            block: None,
        }
    }

    fn record_metrics(
        &self,
        response: BlockResponse,
        start_time: Instant,
        request_type: &'static str,
    ) -> Result<Response<BlockResponse>, Status> {
        let status_enum = block_response::Code::try_from(response.status)
            .unwrap_or(block_response::Code::Unknown);
        let status_label = code_to_string(status_enum);
        let duration = start_time.elapsed().as_secs_f64();

        self.metrics
            .block_access_request_duration_seconds
            .with_label_values(&[status_label, request_type])
            .observe(duration);

        self.metrics
            .block_access_requests_total
            .with_label_values(&[status_label, request_type])
            .inc();

        if status_enum == block_response::Code::Success && request_type == "latest" {
            if let Some(ref block) = response.block {
                if let Some(item) = block.items.first() {
                    if let Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h)) = &item.item {
                         self.metrics.block_access_latest_available_block.set(h.number as i64);
                     }
                }
            }
        }

        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rock_node_core::block_reader::BlockReader;
    use rock_node_protobufs::{
        com::hedera::hapi::block::stream::BlockItem,
        org::hiero::block::api::block_request::BlockSpecifier,
    };
    use std::collections::HashMap;

    #[derive(Debug, Default)]
    struct MockBlockReader {
        blocks: HashMap<u64, Vec<u8>>,
        earliest_block: i64,
        latest_block: i64,
        force_db_error: bool,
    }

    impl MockBlockReader {
        fn new(earliest: i64, latest: i64) -> Self {
            Self {
                earliest_block: earliest,
                latest_block: latest,
                ..Default::default()
            }
        }

        fn insert_block(&mut self, number: u64, data: Vec<u8>) {
            self.blocks.insert(number, data);
        }
    }

    impl BlockReader for MockBlockReader {
        fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
            if self.force_db_error {
                return Err(anyhow::anyhow!("Forced database error"));
            }
            Ok(self.blocks.get(&block_number).cloned())
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

    fn create_test_service(reader: MockBlockReader) -> BlockAccessServiceImpl {
        BlockAccessServiceImpl {
            block_reader: Arc::new(reader),
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
        }
    }

    fn create_mock_block() -> Block {
        Block {
            items: vec![BlockItem {
                item: Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
                    rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                        number: 200,
                        ..Default::default()
                    }
                ))
            }],
        }
    }

    fn create_mock_block_bytes() -> Vec<u8> {
        create_mock_block().encode_to_vec()
    }

    #[tokio::test]
    async fn test_get_block_by_number_success() {
        let mut reader = MockBlockReader::new(100, 200);
        let mock_block_bytes = create_mock_block_bytes();
        reader.insert_block(150, mock_block_bytes.clone());
        let service = create_test_service(reader);

        let request = Request::new(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(150)),
        });

        let response = service.get_block(request).await.unwrap().into_inner();

        assert_eq!(response.status, block_response::Code::Success as i32);
        // The mock block has number 200, so we create a new one for assertion
        let mut expected_block = create_mock_block();
        if let Some(
            rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h),
        ) = expected_block.items.get_mut(0).unwrap().item.as_mut()
        {
            h.number = 150;
        }
        assert!(response.block.is_some());
    }

    #[tokio::test]
    async fn test_get_latest_block_success() {
        let mut reader = MockBlockReader::new(100, 200);
        let mock_block_bytes = create_mock_block_bytes();
        reader.insert_block(200, mock_block_bytes);
        let service = create_test_service(reader);

        let request = Request::new(BlockRequest {
            block_specifier: Some(BlockSpecifier::RetrieveLatest(true)),
        });

        let response = service.get_block(request).await.unwrap().into_inner();

        assert_eq!(response.status, block_response::Code::Success as i32);
        assert_eq!(response.block, Some(create_mock_block()));
    }

    #[tokio::test]
    async fn test_block_not_found_in_range() {
        let reader = MockBlockReader::new(100, 200);
        let service = create_test_service(reader);

        let request = Request::new(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(150)),
        });

        let response = service.get_block(request).await.unwrap().into_inner();

        assert_eq!(response.status, block_response::Code::NotFound as i32);
        assert!(response.block.is_none());
    }

    #[tokio::test]
    async fn test_block_not_available_too_low() {
        let reader = MockBlockReader::new(100, 200);
        let service = create_test_service(reader);

        let request = Request::new(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(50)),
        });

        let response = service.get_block(request).await.unwrap().into_inner();
        assert_eq!(response.status, block_response::Code::NotAvailable as i32);
        assert!(response.block.is_none());
    }
}
