use prost::Message;
use rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader;
use rock_node_protobufs::com::hedera::hapi::block::stream::{
    block_item::Item as BlockItemType, Block, BlockItem, BlockProof,
};

/// Utility to construct valid `Block` protobuf objects for testing purposes.
#[derive(Debug)]
pub struct BlockBuilder {
    block_number: u64,
    items: Vec<BlockItem>,
}

impl BlockBuilder {
    /// Start building a new block with the required `BlockHeader` already in place.
    pub fn new(block_number: u64) -> Self {
        let header = BlockHeader {
            hapi_proto_version: None,
            software_version: None,
            number: block_number,
            block_timestamp: None,
            hash_algorithm: 0,
        };
        let header_item = BlockItem {
            item: Some(BlockItemType::BlockHeader(header)),
        };
        Self {
            block_number,
            items: vec![header_item],
        }
    }

    /// Returns the items accumulated so far (useful for tests that only need the header).
    pub fn items(&self) -> Vec<BlockItem> {
        self.items.clone()
    }

    /// Finalise the block by appending a dummy `BlockProof` and returning the encoded bytes.
    pub fn build(mut self) -> Vec<u8> {
        let proof = BlockProof {
            block: self.block_number,
            previous_block_root_hash: Vec::new(),
            start_of_block_state_root_hash: Vec::new(),
            block_signature: Vec::new(),
            sibling_hashes: Vec::new(),
            verification_reference: None,
        };
        self.items.push(BlockItem {
            item: Some(BlockItemType::BlockProof(proof)),
        });

        let block = Block { items: self.items };
        block.encode_to_vec()
    }
}
