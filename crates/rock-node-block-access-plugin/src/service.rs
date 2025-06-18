use rock_node_core::{block_reader::BlockReader, MetricsRegistry};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::Block,
    org::hiero::block::api::{
        block_access_service_server::BlockAccessService, block_response, BlockRequest,
        BlockResponse,
    },
};
use prost::Message;
use std::{sync::Arc, time::Instant, convert::TryFrom};
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

        let mut request_type = "by_number";

        // Step 1: Parse the request specifier. This now returns a Result and doesn't try to exit early.
        let block_number_result = match inner_request.block_specifier {
            Some(spec) => match spec {
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::BlockNumber(num) => {
                    if num == u64::MAX {
                        request_type = "latest";
                        Ok(self.block_reader.get_latest_persisted_block_number())
                    } else {
                        Ok(num as i64)
                    }
                },
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::RetrieveLatest(true) => {
                    request_type = "latest";
                    Ok(self.block_reader.get_latest_persisted_block_number())
                },
                _ => {
                    warn!("Invalid block_specifier in request");
                    Err(block_response::Code::InvalidRequest)
                }
            },
            None => {
                warn!("Missing block_specifier in request");
                Err(block_response::Code::InvalidRequest)
            }
        };

        // Step 2: Handle the result of the parsing.
        let block_number_to_fetch = match block_number_result {
            Ok(num) => num,
            Err(code) => {
                let response = BlockResponse { status: code as i32, block: None };
                return self.record_metrics(response, start_time, request_type);
            }
        };

        // Step 3: Continue with the rest of the logic.
        if block_number_to_fetch < 0 {
            debug!("DB is empty or latest block could not be determined.");
            let response = BlockResponse { status: block_response::Code::NotFound as i32, block: None };
            return self.record_metrics(response, start_time, request_type);
        }
        let block_number_u64 = block_number_to_fetch as u64;

        let block_read_result = match self.block_reader.read_block(block_number_u64) {
            Ok(result) => result,
            Err(e) => {
                error!("Database error while fetching block #{}: {}", block_number_u64, e);
                let response = BlockResponse { status: block_response::Code::Unknown as i32, block: None };
                return self.record_metrics(response, start_time, request_type);
            }
        };

        let response = match block_read_result {
            Some(block_bytes) => {
                if block_bytes.is_empty() {
                    warn!("Block #{} was found in storage, but its content is empty. This indicates a data pipeline issue.", block_number_u64);
                    BlockResponse { status: block_response::Code::Unknown as i32, block: None }
                } else {
                    match Block::decode(block_bytes.as_slice()) {
                        Ok(block) => {
                            debug!("Successfully decoded block #{}, returning SUCCESS.", block_number_u64);
                            if request_type == "latest" {
                                self.metrics.block_access_latest_available_block.set(block_number_to_fetch);
                            }
                            BlockResponse {
                                status: block_response::Code::Success as i32,
                                block: Some(block),
                            }
                        }
                        Err(e) => {
                            error!("Failed to decode non-empty block #{} from storage bytes: {}", block_number_u64, e);
                            BlockResponse { status: block_response::Code::Unknown as i32, block: None }
                        }
                    }
                }
            }
            None => {
                let earliest = self.block_reader.get_earliest_persisted_block_number();
                let latest = self.block_reader.get_latest_persisted_block_number();

                let status_code = if block_number_to_fetch < earliest || block_number_to_fetch > latest {
                    debug!("Block #{} is outside of this node's range [{} - {}].", block_number_u64, earliest, latest);
                    block_response::Code::NotAvailable
                } else {
                    warn!("Block #{} was not found but is within the expected range [{} - {}].", block_number_u64, earliest, latest);
                    block_response::Code::NotFound
                };
                BlockResponse { status: status_code as i32, block: None }
            }
        };

        self.record_metrics(response, start_time, request_type)
    }
}

impl BlockAccessServiceImpl {
    fn record_metrics(
        &self,
        response: BlockResponse,
        start_time: Instant,
        request_type: &'static str,
    ) -> Result<Response<BlockResponse>, Status> {
        // Updated to use TryFrom, per the compiler warning
        let status_enum = block_response::Code::try_from(response.status).unwrap_or(block_response::Code::Unknown);
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

        Ok(Response::new(response))
    }
}
