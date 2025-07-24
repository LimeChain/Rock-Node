use anyhow::Result;
use hex;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::account::StakedId as AccountStakedId;
use rock_node_protobufs::proto::crypto_get_account_balance_query;
use rock_node_protobufs::proto::staking_info::StakedId as InfoStakedId;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        account_id::Account as AccountIdType, crypto_get_info_response,
        CryptoGetAccountBalanceQuery, CryptoGetAccountBalanceResponse,
        CryptoGetAccountRecordsQuery, CryptoGetAccountRecordsResponse, CryptoGetInfoQuery,
        CryptoGetInfoResponse, Duration, ResponseCodeEnum, StakingInfo, Timestamp,
        TransactionGetReceiptQuery, TransactionGetReceiptResponse, TransactionGetRecordQuery,
        TransactionGetRecordResponse, TransactionRecord,
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

        let account_id = query
            .account_id
            .ok_or_else(|| Status::invalid_argument("Missing account_id in CryptoGetInfoQuery"))?;

        debug!("Retrieved account_id: {:?}", account_id);
        let state_id = StateIdentifier::StateIdAccounts as u32;

        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id)),
        };

        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        debug!("Constructed db_key with length: {}", db_key.len());

        let account_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        debug!(
            "Retrieved account_bytes of length: {}",
            account_bytes.as_ref().map_or(0, |b| b.len())
        );

        let response = match account_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;
                debug!("Decoded map_change_value: {:?}", map_change_value);

                if let Some(map_change_value::ValueChoice::AccountValue(account)) =
                    map_change_value.value_choice
                {
                    let account_info = crypto_get_info_response::AccountInfo {
                        account_id: Some(account.account_id.clone().unwrap()),
                        contract_account_id: if account.smart_contract {
                            if !account.alias.is_empty() {
                                hex::encode(account.alias.clone())
                            } else {
                                if let Some(AccountIdType::AccountNum(num)) =
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

                    CryptoGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        account_info: Some(account_info),
                    }
                } else {
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

    pub async fn get_account_balance(
        &self,
        query: CryptoGetAccountBalanceQuery,
    ) -> Result<CryptoGetAccountBalanceResponse, Status> {
        let account_id = query
            .balance_source
            .ok_or_else(|| {
                Status::invalid_argument("Missing balance_source in CryptoGetAccountBalanceQuery")
            })
            .and_then(|source| match source {
                crypto_get_account_balance_query::BalanceSource::AccountId(id) => Ok(id),
                crypto_get_account_balance_query::BalanceSource::ContractId(_) => Err(
                    Status::unimplemented("Contract balance query not supported"),
                ),
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

        match account_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::AccountValue(account)) =
                    map_change_value.value_choice
                {
                    Ok(CryptoGetAccountBalanceResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        account_id: Some(account_id),
                        balance: account.tinybar_balance as u64,
                        token_balances: vec![], // Deprecated
                    })
                } else {
                    Err(Status::internal(
                        "State inconsistency: Expected Account value, found other type",
                    ))
                }
            }
            None => Ok(CryptoGetAccountBalanceResponse {
                header: Some(build_response_header(ResponseCodeEnum::InvalidAccountId, 0)),
                account_id: Some(account_id),
                balance: 0,
                token_balances: vec![],
            }),
        }
    }

    pub async fn get_account_records(
        &self,
        query: CryptoGetAccountRecordsQuery,
    ) -> Result<CryptoGetAccountRecordsResponse, Status> {
        let account_id = query.account_id.ok_or_else(|| {
            Status::invalid_argument("Missing account_id in CryptoGetAccountRecordsQuery")
        })?;

        // This is a simplified implementation. A full implementation would require
        // iterating through recent blocks or a dedicated index.
        // For now, we'll return an empty list as we don't store historical records
        // in a way that's easily queryable by account ID.
        Ok(CryptoGetAccountRecordsResponse {
            header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
            account_id: Some(account_id),
            records: vec![],
        })
    }

    pub async fn get_transaction_receipt(
        &self,
        query: TransactionGetReceiptQuery,
    ) -> Result<TransactionGetReceiptResponse, Status> {
        // This query is difficult to implement without a proper transaction index.
        // Consensus Node stores the transaction receipts in the state in a temporary record cache, but we don't have a way to query them. Because they are not shared in the block stream.
        // Instead we might need to reconstruct the receipt.
        // For now, we'll return a `RECEIPT_NOT_FOUND` status.
        Ok(TransactionGetReceiptResponse {
            header: Some(build_response_header(ResponseCodeEnum::ReceiptNotFound, 0)),
            receipt: None,
            duplicate_transaction_receipts: vec![],
            child_transaction_receipts: vec![],
        })
    }

    pub async fn get_transaction_record(
        &self,
        query: TransactionGetRecordQuery,
    ) -> Result<TransactionGetRecordResponse, Status> {
        let transaction_id = query.transaction_id.ok_or_else(|| {
            Status::invalid_argument("Missing transaction_id in TransactionGetRecordQuery")
        })?;
        // Consensus Node stores the transaction records in the state in a temporary record cache, but we don't have a way to query them. Because they are not shared in the block stream.
        // As with receipts, this is hard to implement without an index.
        // We will return a `RECORD_NOT_FOUND` status.
        Ok(TransactionGetRecordResponse {
            header: Some(build_response_header(ResponseCodeEnum::RecordNotFound, 0)),
            transaction_record: Some(TransactionRecord {
                receipt: None,
                transaction_hash: vec![],
                consensus_timestamp: None,
                transaction_id: Some(transaction_id),
                memo: "".to_string(),
                transaction_fee: 0,
                body: None,
                transfer_list: None,
                token_transfer_lists: vec![],
                schedule_ref: None,
                assessed_custom_fees: vec![],
                automatic_token_associations: vec![],
                parent_consensus_timestamp: None,
                alias: vec![],
                ethereum_hash: vec![],
                paid_staking_rewards: vec![],
                entropy: None,
                evm_address: vec![],
                new_pending_airdrops: vec![],
            }),
            duplicate_transaction_records: vec![],
            child_transaction_records: vec![],
        })
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
