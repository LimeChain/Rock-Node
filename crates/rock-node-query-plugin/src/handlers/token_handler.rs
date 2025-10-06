use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        ResponseCodeEnum, TokenGetInfoQuery, TokenGetInfoResponse, TokenGetNftInfoQuery,
        TokenGetNftInfoResponse, TokenInfo, TokenNftInfo,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `TokenService`.
#[derive(Debug)]
pub struct TokenQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl TokenQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the token info for a given token_id.
    pub async fn get_token_info(
        &self,
        query: TokenGetInfoQuery,
    ) -> Result<TokenGetInfoResponse, Status> {
        trace!("Entering get_token_info for query: {:?}", query);

        let token_id = query
            .token
            .ok_or_else(|| Status::invalid_argument("Missing token_id in TokenGetInfoQuery"))?;

        let state_id = StateIdentifier::StateIdTokens as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TokenIdKey(token_id)),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let token_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match token_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::TokenValue(token)) =
                    map_change_value.value_choice
                {
                    let token_info = TokenInfo {
                        token_id: Some(token.token_id.unwrap()),
                        name: token.name,
                        symbol: token.symbol,
                        decimals: token.decimals as u32,
                        total_supply: token.total_supply as u64,
                        treasury: token.treasury_account_id,
                        admin_key: token.admin_key,
                        kyc_key: token.kyc_key,
                        freeze_key: token.freeze_key,
                        wipe_key: token.wipe_key,
                        supply_key: token.supply_key,
                        default_freeze_status: if token.accounts_frozen_by_default {
                            1
                        } else {
                            2
                        },
                        default_kyc_status: if token.accounts_kyc_granted_by_default {
                            1
                        } else {
                            2
                        },
                        deleted: token.deleted,
                        auto_renew_account: token.auto_renew_account_id,
                        auto_renew_period: Some(rock_node_protobufs::proto::Duration {
                            seconds: token.auto_renew_seconds,
                        }),
                        expiry: Some(rock_node_protobufs::proto::Timestamp {
                            seconds: token.expiration_second,
                            nanos: 0,
                        }),
                        memo: token.memo,
                        token_type: token.token_type,
                        supply_type: token.supply_type,
                        max_supply: token.max_supply,
                        fee_schedule_key: token.fee_schedule_key,
                        custom_fees: token.custom_fees,
                        pause_key: token.pause_key,
                        pause_status: if token.paused { 1 } else { 2 },
                        ledger_id: vec![],
                        metadata: token.metadata,
                        metadata_key: token.metadata_key,
                    };
                    TokenGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        token_info: Some(token_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Token value, found other type",
                    ));
                }
            },
            None => {
                trace!("No token found for the given token_id");
                TokenGetInfoResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidTokenId, 0)),
                    token_info: None,
                }
            },
        };
        Ok(response)
    }

    /// Get the NFT info for a given NftId.
    pub async fn get_token_nft_info(
        &self,
        query: TokenGetNftInfoQuery,
    ) -> Result<TokenGetNftInfoResponse, Status> {
        trace!("Entering get_token_nft_info for query: {:?}", query);

        let nft_id = query
            .nft_id
            .ok_or_else(|| Status::invalid_argument("Missing nft_id in TokenGetNftInfoQuery"))?;

        let state_id = StateIdentifier::StateIdNfts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::NftIdKey(nft_id)),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let nft_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match nft_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::NftValue(nft)) =
                    map_change_value.value_choice
                {
                    let nft_info = TokenNftInfo {
                        nft_id: nft.nft_id,
                        account_id: nft.owner_id,
                        creation_time: nft.mint_time,
                        metadata: nft.metadata,
                        ledger_id: vec![],
                        spender_id: nft.spender_id,
                    };
                    TokenGetNftInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        nft: Some(nft_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Nft value, found other type",
                    ));
                }
            },
            None => {
                trace!("No NFT found for the given nft_id");
                TokenGetNftInfoResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidNftId, 0)),
                    nft: None,
                }
            },
        };
        Ok(response)
    }
}

/// Helper function to create a standard response header.
fn build_response_header(
    code: ResponseCodeEnum,
    cost: u64,
) -> rock_node_protobufs::proto::ResponseHeader {
    rock_node_protobufs::proto::ResponseHeader {
        node_transaction_precheck_code: code as i32,
        cost,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_protobufs::proto::{account_id, AccountId, Nft, NftId, Token, TokenId};
    use std::collections::HashMap;

    /// A mock implementation of `StateReader` for controlled testing.
    #[derive(Debug, Default)]
    struct MockStateReader {
        state: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl MockStateReader {
        fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) {
            self.state.insert(key, value);
        }
    }

    impl StateReader for MockStateReader {
        fn get_state_value(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self.state.get(key).cloned())
        }
    }

    // Helper function to generate the database key for a token.
    fn generate_token_db_key(token_id: &TokenId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdTokens as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TokenIdKey(token_id.clone())),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    // Helper function to generate the database key for an NFT.
    fn generate_nft_db_key(nft_id: &NftId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdNfts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::NftIdKey(nft_id.clone())),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    #[tokio::test]
    async fn test_get_token_info_found() {
        let token_id = TokenId {
            token_num: 3001,
            ..Default::default()
        };
        let token = Token {
            token_id: Some(token_id.clone()),
            name: "Test Token".to_string(),
            symbol: "TST".to_string(),
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::TokenValue(token)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_token_db_key(&token_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = TokenQueryHandler::new(Arc::new(mock_reader));
        let query = TokenGetInfoQuery {
            token: Some(token_id.clone()),
            header: None,
        };

        let response = handler.get_token_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let info = response.token_info.unwrap();
        assert_eq!(info.name, "Test Token");
        assert_eq!(info.symbol, "TST");
        assert_eq!(info.token_id.unwrap(), token_id);
    }

    #[tokio::test]
    async fn test_get_token_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = TokenQueryHandler::new(Arc::new(mock_reader));
        let query = TokenGetInfoQuery {
            token: Some(TokenId {
                token_num: 3002,
                ..Default::default()
            }),
            header: None,
        };

        let response = handler.get_token_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidTokenId as i32
        );
        assert!(response.token_info.is_none());
    }

    #[tokio::test]
    async fn test_get_token_nft_info_found() {
        let nft_id = NftId {
            token_id: Some(TokenId {
                token_num: 4001,
                ..Default::default()
            }),
            serial_number: 1,
        };
        let owner_id = AccountId {
            account: Some(account_id::Account::AccountNum(101)),
            ..Default::default()
        };
        let nft = Nft {
            nft_id: Some(nft_id.clone()),
            owner_id: Some(owner_id.clone()),
            metadata: vec![1, 2, 3],
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::NftValue(nft)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_nft_db_key(&nft_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = TokenQueryHandler::new(Arc::new(mock_reader));
        let query = TokenGetNftInfoQuery {
            nft_id: Some(nft_id.clone()),
            header: None,
        };

        let response = handler.get_token_nft_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let info = response.nft.unwrap();
        assert_eq!(info.nft_id.unwrap(), nft_id);
        assert_eq!(info.account_id.unwrap(), owner_id);
        assert_eq!(info.metadata, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_get_token_nft_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = TokenQueryHandler::new(Arc::new(mock_reader));
        let query = TokenGetNftInfoQuery {
            nft_id: Some(NftId {
                token_id: Some(TokenId {
                    token_num: 4002,
                    ..Default::default()
                }),
                serial_number: 1,
            }),
            header: None,
        };

        let response = handler.get_token_nft_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidNftId as i32
        );
        assert!(response.nft.is_none());
    }
}
