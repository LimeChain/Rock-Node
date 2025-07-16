use anyhow::Result;
use hex;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::account::StakedId as AccountStakedId;
use rock_node_protobufs::proto::staking_info::StakedId as InfoStakedId;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        account_id::Account, crypto_get_info_response, CryptoGetInfoQuery, CryptoGetInfoResponse,
        Duration, ResponseCodeEnum, StakingInfo, Timestamp,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::{debug, trace};

/// Contains the business logic for handling all queries related to the `CryptoService`.
#[derive(Debug)]
pub struct CryptoQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl CryptoQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the account info for a given account_id.
    ///
    /// # Arguments
    ///
    /// * `query` - The CryptoGetInfoQuery containing the account_id
    ///
    /// # Returns
    ///
    /// * `CryptoGetInfoResponse` - The response containing the account info
    pub async fn get_account_info(
        &self,
        query: CryptoGetInfoQuery,
    ) -> Result<CryptoGetInfoResponse, Status> {
        trace!("Entering get_account_info for query: {:?}", query);

        // 1. Get the account_id from the query
        let account_id = query
            .account_id
            .ok_or_else(|| Status::invalid_argument("Missing account_id in CryptoGetInfoQuery"))?;

        debug!("Retrieved account_id: {:?}", account_id);
        // 2. Get the state_id and map_key from the account_id
        let state_id = StateIdentifier::StateIdAccounts as u32;

        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id)),
        };

        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        debug!("Constructed db_key with length: {}", db_key.len());

        // 3. Get the account_bytes from the state_reader
        let account_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        debug!(
            "Retrieved account_bytes of length: {}",
            account_bytes.as_ref().map_or(0, |b| b.len())
        );

        // 4. Decode the account_bytes into a MapChangeValue
        let response = match account_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;
                debug!("Decoded map_change_value: {:?}", map_change_value);

                // 5. Decode the MapChangeValue into an Account
                if let Some(map_change_value::ValueChoice::AccountValue(account)) =
                    map_change_value.value_choice
                {
                    // 6. Map the Account to the AccountInfo
                    let account_info = crypto_get_info_response::AccountInfo {
                        account_id: Some(account.account_id.clone().unwrap()),
                        contract_account_id: if account.smart_contract {
                            if !account.alias.is_empty() {
                                hex::encode(account.alias.clone())
                            } else {
                                if let Some(Account::AccountNum(num)) =
                                    account.account_id.as_ref().unwrap().account
                                {
                                    format!("0.0.{}", num)
                                } else {
                                    String::new()
                                }
                            }
                        } else {
                            String::new()
                        },
                        memo: account.memo,
                        key: account.key,
                        balance: account.tinybar_balance as u64,
                        receiver_sig_required: account.receiver_sig_required,
                        deleted: account.deleted,
                        auto_renew_period: if account.auto_renew_seconds > 0 {
                            Some(Duration {
                                seconds: account.auto_renew_seconds,
                            })
                        } else {
                            None
                        },
                        expiration_time: if account.expiration_second > 0 {
                            Some(Timestamp {
                                seconds: account.expiration_second,
                                nanos: 0,
                            })
                        } else {
                            None
                        },
                        staking_info: Some(StakingInfo {
                            staked_to_me: account.staked_to_me,
                            stake_period_start: if account.stake_period_start > 0 {
                                Some(Timestamp {
                                    seconds: account.stake_period_start,
                                    nanos: 0,
                                })
                            } else {
                                None
                            },
                            decline_reward: account.decline_reward,
                            pending_reward: 0,
                            staked_id: account.staked_id.map(|id| match id {
                                AccountStakedId::StakedNodeId(node) => {
                                    InfoStakedId::StakedNodeId(node)
                                }
                                AccountStakedId::StakedAccountId(acc) => {
                                    InfoStakedId::StakedAccountId(acc)
                                }
                            }),
                        }),
                        ethereum_nonce: account.ethereum_nonce,
                        owned_nfts: account.number_owned_nfts,
                        max_automatic_token_associations: account.max_auto_associations,
                        alias: account.alias,

                        ..Default::default()
                    };

                    // 7. Return the AccountInfo
                    CryptoGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        account_info: Some(account_info),
                    }
                } else {
                    // This is an internal error: the key for an account pointed to data of the wrong type.
                    return Err(Status::internal(
                        "State inconsistency: Expected Account value, found other type",
                    ));
                }
            }
            None => {
                trace!("No account found for the given account_id");
                CryptoGetInfoResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidAccountId, 0)),
                    account_info: None,
                }
            }
        };

        trace!(
            "Exiting get_account_info with response code: {:?}",
            response
                .header
                .as_ref()
                .map(|h| h.node_transaction_precheck_code)
        );

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
