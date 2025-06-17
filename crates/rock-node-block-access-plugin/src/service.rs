use rock_node_core::block_reader::BlockReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::Block,
    org::hiero::block::api::{
        block_access_service_server::BlockAccessService, block_response, BlockRequest,
        BlockResponse,
    },
};
use prost::Message;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, error, warn};

#[derive(Debug)]
pub struct BlockAccessServiceImpl {
    pub block_reader: Arc<dyn BlockReader>,
}

#[tonic::async_trait]
impl BlockAccessService for BlockAccessServiceImpl {
    async fn get_block(
        &self,
        request: Request<BlockRequest>,
    ) -> Result<Response<BlockResponse>, Status> {
        let inner_request = request.into_inner();
        debug!("Processing getBlock request: {:?}", inner_request);

        // --- 1. Determine which block to fetch ---
        let block_number_to_fetch = match inner_request.block_specifier {
            Some(spec) => match spec {
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::BlockNumber(num) => {
                    if num == u64::MAX { self.block_reader.get_latest_persisted_block_number() } else { num as i64 }
                },
                rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::RetrieveLatest(true) => {
                    self.block_reader.get_latest_persisted_block_number()
                },
                _ => {
                    warn!("Invalid block_specifier in request");
                    let response = BlockResponse { status: block_response::Code::InvalidRequest as i32, block: None };
                    return Ok(Response::new(response));
                }
            },
            None => {
                warn!("Missing block_specifier in request");
                let response = BlockResponse { status: block_response::Code::InvalidRequest as i32, block: None };
                return Ok(Response::new(response));
            }
        };

        // --- 2. Handle cases where no blocks exist ---
        if block_number_to_fetch < 0 {
            debug!("DB is empty or latest block could not be determined.");
            let response = BlockResponse { status: block_response::Code::NotFound as i32, block: None };
            return Ok(Response::new(response));
        }
        let block_number_u64 = block_number_to_fetch as u64;


        // --- 3. Attempt to read from storage ---
        let block_read_result = match self.block_reader.read_block(block_number_u64) {
            Ok(result) => result,
            Err(e) => {
                error!("Database error while fetching block #{}: {}", block_number_u64, e);
                let response = BlockResponse { status: block_response::Code::Unknown as i32, block: None };
                return Ok(Response::new(response));
            }
        };

        // --- 4. Process the result ---
        let response = match block_read_result {
            Some(block_bytes) => {
                if block_bytes.is_empty() {
                    warn!("Block #{} was found in storage, but its content is empty. This indicates a data pipeline issue.", block_number_u64);
                    BlockResponse { status: block_response::Code::Unknown as i32, block: None }
                } else {
                    match Block::decode(block_bytes.as_slice()) {
                        Ok(block) => {
                            debug!("Successfully decoded block #{}, returning SUCCESS.", block_number_u64);
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

        Ok(Response::new(response))
    }
}
