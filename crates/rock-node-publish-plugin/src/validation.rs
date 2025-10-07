use crate::error::{PublishError, Result};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::block_item::Item as BlockItemType,
    org::hiero::block::api::BlockItemSet,
};

/// Validates a BlockItemSet for basic format correctness
pub struct RequestValidator {
    max_items_per_set: usize,
}

impl RequestValidator {
    pub fn new(max_items_per_set: usize) -> Self {
        Self { max_items_per_set }
    }

    /// Validate block item set format
    pub fn validate_block_items(
        &self,
        block_item_set: &BlockItemSet,
        is_new_block: bool,
    ) -> Result<()> {
        // Check not empty
        if block_item_set.block_items.is_empty() {
            return Err(PublishError::EmptyBlockItems);
        }

        // Check item count
        let count = block_item_set.block_items.len();
        if count > self.max_items_per_set {
            return Err(PublishError::TooManyItems {
                count,
                max: self.max_items_per_set,
            });
        }

        // If this is a new block, first item must be a BlockHeader
        if is_new_block {
            let first_item = &block_item_set.block_items[0];
            if let Some(item_type) = &first_item.item {
                if !matches!(item_type, BlockItemType::BlockHeader(_)) {
                    // Extract block number if we can
                    let block_number = self.extract_block_number_from_any_item(item_type);
                    return Err(PublishError::MissingBlockHeader {
                        block_number: block_number.unwrap_or(-1),
                    });
                }
            } else {
                return Err(PublishError::MissingBlockHeader { block_number: -1 });
            }
        }

        Ok(())
    }

    /// Extract block number from any item type (for error reporting)
    fn extract_block_number_from_any_item(&self, item: &BlockItemType) -> Option<i64> {
        match item {
            BlockItemType::BlockHeader(header) => Some(header.number as i64),
            BlockItemType::BlockProof(proof) => Some(proof.block as i64),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_protobufs::com::hedera::hapi::block::stream::{
        output::BlockHeader, BlockItem, BlockProof,
    };

    fn make_header_item(block_num: u64) -> BlockItem {
        BlockItem {
            item: Some(BlockItemType::BlockHeader(BlockHeader {
                hapi_proto_version: None,
                software_version: None,
                number: block_num,
                block_timestamp: None,
                hash_algorithm: 0,
            })),
        }
    }

    fn make_proof_item(block_num: u64) -> BlockItem {
        BlockItem {
            item: Some(BlockItemType::BlockProof(BlockProof {
                block: block_num,
                ..Default::default()
            })),
        }
    }

    #[test]
    fn test_empty_block_items() {
        let validator = RequestValidator::new(1000);
        let empty_set = BlockItemSet {
            block_items: vec![],
        };

        let result = validator.validate_block_items(&empty_set, true);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PublishError::EmptyBlockItems));
    }

    #[test]
    fn test_too_many_items() {
        let validator = RequestValidator::new(5);
        let items = vec![make_header_item(1); 10]; // 10 items, max is 5
        let set = BlockItemSet { block_items: items };

        let result = validator.validate_block_items(&set, true);
        assert!(result.is_err());
        match result.unwrap_err() {
            PublishError::TooManyItems { count, max } => {
                assert_eq!(count, 10);
                assert_eq!(max, 5);
            },
            _ => panic!("Expected TooManyItems error"),
        }
    }

    #[test]
    fn test_new_block_missing_header() {
        let validator = RequestValidator::new(1000);
        // Start with proof instead of header
        let items = vec![make_proof_item(1)];
        let set = BlockItemSet { block_items: items };

        let result = validator.validate_block_items(&set, true);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PublishError::MissingBlockHeader { .. }
        ));
    }

    #[test]
    fn test_valid_new_block() {
        let validator = RequestValidator::new(1000);
        let items = vec![make_header_item(1), make_proof_item(1)];
        let set = BlockItemSet { block_items: items };

        let result = validator.validate_block_items(&set, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_continuation() {
        let validator = RequestValidator::new(1000);
        // Continuation doesn't need header
        let items = vec![make_proof_item(1)];
        let set = BlockItemSet { block_items: items };

        let result = validator.validate_block_items(&set, false);
        assert!(result.is_ok());
    }
}
