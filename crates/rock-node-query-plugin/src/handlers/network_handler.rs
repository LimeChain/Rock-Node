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
            }
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
            }
            None => {
                trace!("No account found for the given account_id");
                GetAccountDetailsResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidAccountId, 0)),
                    account_details: None,
                }
            }
        };
        Ok(response)
    }

    async fn get_token_relationships(
        &self,
        account: &rock_node_protobufs::proto::Account,
    ) -> Result<Vec<TokenRelationship>, Status> {
        let mut relationships = Vec::new();
        let mut current_token_id = account.head_token_id.clone();

        let state_id = StateIdentifier::StateIdTokenRelations as u32;

        for _ in 0..account.number_associations {
            if current_token_id.is_none() {
                break;
            }

            let map_key = MapChangeKey {
                key_choice: Some(map_change_key::KeyChoice::TokenRelationshipKey(
                    rock_node_protobufs::proto::TokenAssociation {
                        token_id: current_token_id.clone(),
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
