use rock_node_core::block_reader::BlockReader;
use rock_node_protobufs::org::hiero::block::api::{
    block_node_service_server::BlockNodeService, ServerStatusRequest, ServerStatusResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::debug;

#[derive(Debug)]
pub struct StatusServiceImpl {
    pub block_reader: Arc<dyn BlockReader>,
}

#[tonic::async_trait]
impl BlockNodeService for StatusServiceImpl {
    async fn server_status(
        &self,
        request: Request<ServerStatusRequest>,
    ) -> Result<Response<ServerStatusResponse>, Status> {
        debug!("Processing serverStatus request: {:?}", request);

        // Get the block range from the persistence service.
        let earliest_block = self.block_reader.get_earliest_persisted_block_number();
        let latest_block = self.block_reader.get_latest_persisted_block_number();

        // The proto uses uint64, but our reader uses i64 with -1 for "not found".
        // We convert -1 to 0 for the response, as 0 is a safe default for an empty DB.
        let response = ServerStatusResponse {
            first_available_block: if earliest_block < 0 { 0 } else { earliest_block as u64 },
            last_available_block: if latest_block < 0 { 0 } else { latest_block as u64 },

            // TODO: Implement logic for state snapshot availability.
            only_latest_state: false,

            // TODO: Populate version information from the AppContext or build-time variables.
            version_information: None,
        };

        Ok(Response::new(response))
    }
}
