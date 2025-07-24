use prost::Message;
use rock_node_protobufs::com::hedera::hapi::block::stream::output::{
    map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateChange, StateChanges,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::output::{
    state_change::ChangeOperation, MapUpdateChange,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::{
    block_item::Item as BlockItemType, Block, BlockItem, BlockProof,
};
use rock_node_protobufs::proto::{account_id, Account, AccountId, Timestamp};

/// Utility to construct valid `Block` protobuf objects for testing purposes.
#[derive(Debug)]
pub struct BlockBuilder {
    block_number: u64,
    items: Vec<BlockItem>,
}

impl BlockBuilder {
    /// Start building a new block with the required `BlockHeader` already in place.
    pub fn new(block_number: u64) -> Self {
        let header = rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
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

    /// Adds a state change for a simple account to the block.
    pub fn with_account_state_change(mut self, account_num: i64, memo: &str) -> Self {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(account_num)),
        };

        let account_value = Account {
            account_id: Some(account_id.clone()),
            memo: memo.to_string(),
            tinybar_balance: account_num,
            ..Default::default()
        };

        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id)),
        };

        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account_value)),
        };

        let state_change = StateChange {
            state_id: 2,
            change_operation: Some(ChangeOperation::MapUpdate(MapUpdateChange {
                key: Some(map_key),
                value: Some(map_value),
            })),
        };

        let state_changes_item = BlockItem {
            item: Some(BlockItemType::StateChanges(StateChanges {
                consensus_timestamp: Some(Timestamp {
                    seconds: self.block_number as i64,
                    nanos: 0,
                }),
                state_changes: vec![state_change],
            })),
        };

        self.items.push(state_changes_item);
        self
    }

    /// Returns the items accumulated so far (useful for tests that only need the header).
    pub fn items(&self) -> Vec<BlockItem> {
        self.items.clone()
    }

    /// Finalise the block by appending a dummy `BlockProof` and returning the encoded bytes.
    pub fn build(mut self) -> Vec<u8> {
        let proof = BlockProof {
            block: self.block_number,
            ..Default::default()
        };
        self.items.push(BlockItem {
            item: Some(BlockItemType::BlockProof(proof)),
        });

        let block = Block { items: self.items };
        block.encode_to_vec()
    }
}
