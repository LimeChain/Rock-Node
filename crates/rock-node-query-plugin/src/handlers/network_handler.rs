use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        get_account_details_response, GetAccountDetailsQuery, GetAccountDetailsResponse,
        GrantedCryptoAllowance, GrantedNftAllowance, GrantedTokenAllowance,
        NetworkGetVersionInfoQuery, NetworkGetVersionInfoResponse, ResponseCodeEnum,
        TokenRelationship,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `NetworkService`.
#[derive(Debug)]
pub struct NetworkQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl NetworkQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the version info for the network.
    ///
    /// # Arguments
    ///
    /// * `query` - The NetworkGetVersionInfoQuery
    ///
    /// # Returns
    ///
    /// * `NetworkGetVersionInfoResponse` - The response containing the version info
    pub async fn get_version_info(
        &self,
        _query: NetworkGetVersionInfoQuery,
    ) -> Result<NetworkGetVersionInfoResponse, Status> {
        trace!("Entering get_version_info");

        let state_id = StateIdentifier::StateIdBlockStreamInfo as u32;
        let block_stream_info_bytes = self
            .state_reader
            .get_state_value(&state_id.to_be_bytes())
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let (hapi_proto_version, hedera_services_version) = match block_stream_info_bytes {
            Some(bytes) => {
                let block_stream_info =
                    rock_node_protobufs::com::hedera::hapi::node::state::blockstream::BlockStreamInfo::decode(bytes.as_slice())
                        .map_err(|e| Status::internal(format!("Failed to decode BlockStreamInfo: {}", e)))?;
                let software_version = block_stream_info
                    .creation_software_version
                    .unwrap_or_default();

                // Assuming HAPI version is the same as services version for now
                (Some(software_version.clone()), Some(software_version))
            },
            None => (None, None),
        };

        let response = NetworkGetVersionInfoResponse {
            header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
            hapi_proto_version,
            hedera_services_version,
        };

        Ok(response)
    }

    /// Get the account details for a given account_id.
    ///
    /// # Arguments
    ///
    /// * `query` - The GetAccountDetailsQuery containing the account_id
    ///
    /// # Returns
    ///
    /// * `GetAccountDetailsResponse` - The response containing the account details
    pub async fn get_account_details(
        &self,
        query: GetAccountDetailsQuery,
    ) -> Result<GetAccountDetailsResponse, Status> {
        trace!("Entering get_account_details for query: {:?}", query);

        let account_id = query.account_id.ok_or_else(|| {
            Status::invalid_argument("Missing account_id in GetAccountDetailsQuery")
        })?;

        let state_id = StateIdentifier::StateIdAccounts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id.clone())),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let account_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match account_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::AccountValue(account)) =
                    map_change_value.value_choice
                {
                    let token_relationships = self.get_token_relationships(&account).await?;
                    let account_details = get_account_details_response::AccountDetails {
                        account_id: Some(account.account_id.clone().unwrap()),
                        contract_account_id: if account.smart_contract {
                            hex::encode(&account.alias)
                        } else {
                            "".to_string()
                        },
                        deleted: account.deleted,
                        proxy_received: account.staked_to_me,
                        key: account.key,
                        balance: account.tinybar_balance as u64,
                        receiver_sig_required: account.receiver_sig_required,
                        expiration_time: Some(rock_node_protobufs::proto::Timestamp {
                            seconds: account.expiration_second,
                            nanos: 0,
                        }),
                        auto_renew_period: Some(rock_node_protobufs::proto::Duration {
                            seconds: account.auto_renew_seconds,
                        }),
                        token_relationships,
                        memo: account.memo,
                        owned_nfts: account.number_owned_nfts,
                        max_automatic_token_associations: account.max_auto_associations,
                        alias: account.alias,
                        ledger_id: vec![], // Not supported yet
                        granted_crypto_allowances: account
                            .crypto_allowances
                            .into_iter()
                            .map(|a| GrantedCryptoAllowance {
                                spender: a.spender_id,
                                amount: a.amount,
                            })
                            .collect(),
                        granted_nft_allowances: account
                            .approve_for_all_nft_allowances
                            .into_iter()
                            .map(|a| GrantedNftAllowance {
                                token_id: a.token_id,
                                spender: a.spender_id,
                            })
                            .collect(),
                        granted_token_allowances: account
                            .token_allowances
                            .into_iter()
                            .map(|a| GrantedTokenAllowance {
                                token_id: a.token_id,
                                spender: a.spender_id,
                                amount: a.amount,
                            })
                            .collect(),
                        ..Default::default()
                    };
                    GetAccountDetailsResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        account_details: Some(account_details),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Account value, found other type",
                    ));
                }
            },
            None => {
                trace!("No account found for the given account_id");
                GetAccountDetailsResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidAccountId, 0)),
                    account_details: None,
                }
            },
        };
        Ok(response)
    }

    async fn get_token_relationships(
        &self,
        account: &rock_node_protobufs::proto::Account,
    ) -> Result<Vec<TokenRelationship>, Status> {
        let mut relationships = Vec::new();
        let mut current_token_id = account.head_token_id;

        let state_id = StateIdentifier::StateIdTokenRelations as u32;

        for _ in 0..account.number_associations {
            if current_token_id.is_none() {
                break;
            }

            let map_key = MapChangeKey {
                key_choice: Some(map_change_key::KeyChoice::TokenRelationshipKey(
                    rock_node_protobufs::proto::TokenAssociation {
                        token_id: current_token_id,
                        account_id: account.account_id.clone(),
                    },
                )),
            };

            let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();
            let token_relation_bytes = self
                .state_reader
                .get_state_value(&db_key)
                .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?
                .ok_or_else(|| Status::internal("Token relationship not found in state"))?;

            let map_change_value: MapChangeValue =
                MapChangeValue::decode(token_relation_bytes.as_slice()).map_err(|e| {
                    Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                })?;

            if let Some(map_change_value::ValueChoice::TokenRelationValue(token_relation)) =
                map_change_value.value_choice
            {
                relationships.push(TokenRelationship {
                    token_id: token_relation.token_id,
                    balance: token_relation.balance as u64,
                    kyc_status: if token_relation.kyc_granted { 1 } else { 2 },
                    freeze_status: if token_relation.frozen { 1 } else { 2 },
                    decimals: 0, // This should be fetched from the token itself
                    automatic_association: token_relation.automatic_association,
                    ..Default::default()
                });
                current_token_id = token_relation.next_token;
            } else {
                return Err(Status::internal(
                    "State inconsistency: Expected TokenRelation value, found other type",
                ));
            }
        }
        Ok(relationships)
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
    use rock_node_protobufs::{
        com::hedera::hapi::node::state::blockstream::BlockStreamInfo,
        proto::{account_id, Account, AccountId, SemanticVersion, TokenId, TokenRelation},
    };
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

    #[tokio::test]
    async fn test_get_version_info_found() {
        let mut mock_reader = MockStateReader::default();
        let state_id = StateIdentifier::StateIdBlockStreamInfo as u32;
        let version = SemanticVersion {
            major: 1,
            minor: 2,
            patch: 3,
            pre: "test".to_string(),
            build: "test".to_string(),
            ..Default::default()
        };
        let block_stream_info = BlockStreamInfo {
            creation_software_version: Some(version.clone()),
            ..Default::default()
        };
        mock_reader.insert(
            state_id.to_be_bytes().to_vec(),
            block_stream_info.encode_to_vec(),
        );

        let handler = NetworkQueryHandler::new(Arc::new(mock_reader));
        let query = NetworkGetVersionInfoQuery { header: None };
        let response = handler.get_version_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        assert_eq!(response.hapi_proto_version.unwrap(), version);
        assert_eq!(response.hedera_services_version.unwrap(), version);
    }

    #[tokio::test]
    async fn test_get_version_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = NetworkQueryHandler::new(Arc::new(mock_reader));
        let query = NetworkGetVersionInfoQuery { header: None };
        let response = handler.get_version_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        assert!(response.hapi_proto_version.is_none());
        assert!(response.hedera_services_version.is_none());
    }

    #[tokio::test]
    async fn test_get_account_details_found() {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(1001)),
        };
        let account = Account {
            account_id: Some(account_id.clone()),
            memo: "test_memo".to_string(),
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account)),
        };

        let mut mock_reader = MockStateReader::default();
        let state_id = StateIdentifier::StateIdAccounts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id.clone())),
        };
        let key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = NetworkQueryHandler::new(Arc::new(mock_reader));
        let query = GetAccountDetailsQuery {
            account_id: Some(account_id.clone()),
            header: None,
        };

        let response = handler.get_account_details(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let details = response.account_details.unwrap();
        assert_eq!(details.memo, "test_memo");
        assert_eq!(details.account_id.unwrap(), account_id);
    }

    #[tokio::test]
    async fn test_get_account_details_with_token_rels() {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(1002)),
        };
        let token_id1 = TokenId {
            token_num: 2001,
            ..Default::default()
        };
        let token_id2 = TokenId {
            token_num: 2002,
            ..Default::default()
        };

        let account = Account {
            account_id: Some(account_id.clone()),
            number_associations: 2,
            head_token_id: Some(token_id1.clone()),
            ..Default::default()
        };
        let token_rel1 = TokenRelation {
            account_id: Some(account_id.clone()),
            token_id: Some(token_id1.clone()),
            balance: 100,
            next_token: Some(token_id2.clone()),
            ..Default::default()
        };
        let token_rel2 = TokenRelation {
            account_id: Some(account_id.clone()),
            token_id: Some(token_id2.clone()),
            balance: 200,
            ..Default::default()
        };

        let mut mock_reader = MockStateReader::default();
        // Insert Account
        let acc_map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id.clone())),
        };
        let acc_map_val = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account)),
        };
        let acc_key = [
            (StateIdentifier::StateIdAccounts as u32)
                .to_be_bytes()
                .as_slice(),
            &acc_map_key.encode_to_vec(),
        ]
        .concat();
        mock_reader.insert(acc_key, acc_map_val.encode_to_vec());

        // Insert TokenRels
        let rel_state_id = StateIdentifier::StateIdTokenRelations as u32;
        let rel1_map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TokenRelationshipKey(
                rock_node_protobufs::proto::TokenAssociation {
                    token_id: Some(token_id1.clone()),
                    account_id: Some(account_id.clone()),
                },
            )),
        };
        let rel1_map_val = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::TokenRelationValue(
                token_rel1,
            )),
        };
        let rel1_key = [
            rel_state_id.to_be_bytes().as_slice(),
            &rel1_map_key.encode_to_vec(),
        ]
        .concat();
        mock_reader.insert(rel1_key, rel1_map_val.encode_to_vec());

        let rel2_map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TokenRelationshipKey(
                rock_node_protobufs::proto::TokenAssociation {
                    token_id: Some(token_id2.clone()),
                    account_id: Some(account_id.clone()),
                },
            )),
        };
        let rel2_map_val = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::TokenRelationValue(
                token_rel2,
            )),
        };
        let rel2_key = [
            rel_state_id.to_be_bytes().as_slice(),
            &rel2_map_key.encode_to_vec(),
        ]
        .concat();
        mock_reader.insert(rel2_key, rel2_map_val.encode_to_vec());

        let handler = NetworkQueryHandler::new(Arc::new(mock_reader));
        let query = GetAccountDetailsQuery {
            account_id: Some(account_id.clone()),
            header: None,
        };

        let response = handler.get_account_details(query).await.unwrap();
        let details = response.account_details.unwrap();
        assert_eq!(details.token_relationships.len(), 2);
        assert_eq!(details.token_relationships[0].balance, 100);
        assert_eq!(details.token_relationships[1].balance, 200);
    }
}
