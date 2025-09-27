use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        contract_get_info_response::ContractInfo, contract_id::Contract as ContractIdType,
        AccountId, ContractGetBytecodeQuery, ContractGetBytecodeResponse, ContractGetInfoQuery,
        ContractGetInfoResponse, ResponseCodeEnum,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `SmartContractService`.
#[derive(Debug)]
pub struct ContractQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl ContractQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the contract info for a given contract_id.
    pub async fn get_contract_info(
        &self,
        query: ContractGetInfoQuery,
    ) -> Result<ContractGetInfoResponse, Status> {
        trace!("Entering get_contract_info for query: {:?}", query);

        let contract_id = query.contract_id.ok_or_else(|| {
            Status::invalid_argument("Missing contract_id in ContractGetInfoQuery")
        })?;

        // Contracts are a type of account, so we query the accounts state.
        let account_id = AccountId {
            shard_num: contract_id.shard_num,
            realm_num: contract_id.realm_num,
            account: Some(rock_node_protobufs::proto::account_id::Account::AccountNum(
                if let Some(ContractIdType::ContractNum(num)) = contract_id.contract {
                    num
                } else {
                    return Err(Status::invalid_argument("EVM address not supported yet"));
                },
            )),
        };

        let state_id = StateIdentifier::StateIdAccounts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id)),
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
                    if !account.smart_contract {
                        return Ok(ContractGetInfoResponse {
                            header: Some(build_response_header(
                                ResponseCodeEnum::InvalidContractId,
                                0,
                            )),
                            contract_info: None,
                        });
                    }

                    let contract_info = ContractInfo {
                        contract_id: Some(contract_id.clone()),
                        account_id: account.account_id,
                        contract_account_id: hex::encode(&account.alias),
                        admin_key: account.key,
                        expiration_time: Some(rock_node_protobufs::proto::Timestamp {
                            seconds: account.expiration_second,
                            nanos: 0,
                        }),
                        auto_renew_period: Some(rock_node_protobufs::proto::Duration {
                            seconds: account.auto_renew_seconds,
                        }),
                        storage: account.contract_kv_pairs_number as i64,
                        memo: account.memo,
                        balance: account.tinybar_balance as u64,
                        deleted: account.deleted,
                        ..Default::default()
                    };
                    ContractGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        contract_info: Some(contract_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Account value for contract, found other type",
                    ));
                }
            },
            None => {
                trace!("No contract found for the given contract_id");
                ContractGetInfoResponse {
                    header: Some(build_response_header(
                        ResponseCodeEnum::InvalidContractId,
                        0,
                    )),
                    contract_info: None,
                }
            },
        };
        Ok(response)
    }

    /// Get the bytecode for a given contract_id.
    pub async fn get_contract_bytecode(
        &self,
        query: ContractGetBytecodeQuery,
    ) -> Result<ContractGetBytecodeResponse, Status> {
        trace!("Entering get_contract_bytecode for query: {:?}", query);

        let contract_id = query.contract_id.ok_or_else(|| {
            Status::invalid_argument("Missing contract_id in ContractGetBytecodeQuery")
        })?;

        let state_id = StateIdentifier::StateIdContractBytecode as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::ContractIdKey(contract_id)),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let bytecode_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match bytecode_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::BytecodeValue(bytecode)) =
                    map_change_value.value_choice
                {
                    ContractGetBytecodeResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        bytecode: bytecode.code,
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Bytecode value, found other type",
                    ));
                }
            },
            None => {
                trace!("No bytecode found for the given contract_id");
                ContractGetBytecodeResponse {
                    header: Some(build_response_header(
                        ResponseCodeEnum::InvalidContractId,
                        0,
                    )),
                    bytecode: vec![],
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
    use rock_node_protobufs::proto::{account_id, contract_id, Account, Bytecode, ContractId};
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

    // Helper function to generate the database key for contract info (via account).
    fn generate_contract_info_db_key(contract_id: &ContractId) -> Vec<u8> {
        let account_id = AccountId {
            shard_num: contract_id.shard_num,
            realm_num: contract_id.realm_num,
            account: Some(account_id::Account::AccountNum(
                if let Some(ContractIdType::ContractNum(num)) = contract_id.contract {
                    num
                } else {
                    0
                },
            )),
        };
        let state_id = StateIdentifier::StateIdAccounts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id)),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    // Helper function to generate the database key for bytecode.
    fn generate_bytecode_db_key(contract_id: &ContractId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdContractBytecode as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::ContractIdKey(
                contract_id.clone(),
            )),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    #[tokio::test]
    async fn test_get_contract_info_found() {
        let contract_id = ContractId {
            contract: Some(contract_id::Contract::ContractNum(7001)),
            ..Default::default()
        };
        let account = Account {
            account_id: Some(AccountId {
                account: Some(account_id::Account::AccountNum(7001)),
                ..Default::default()
            }),
            memo: "test_contract".to_string(),
            smart_contract: true,
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_contract_info_db_key(&contract_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = ContractQueryHandler::new(Arc::new(mock_reader));
        let query = ContractGetInfoQuery {
            contract_id: Some(contract_id.clone()),
            header: None,
        };

        let response = handler.get_contract_info(query).await.unwrap();
        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let info = response.contract_info.unwrap();
        assert_eq!(info.memo, "test_contract");
        assert_eq!(info.contract_id.unwrap(), contract_id);
    }

    #[tokio::test]
    async fn test_get_contract_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = ContractQueryHandler::new(Arc::new(mock_reader));
        let query = ContractGetInfoQuery {
            contract_id: Some(ContractId {
                contract: Some(contract_id::Contract::ContractNum(7002)),
                ..Default::default()
            }),
            header: None,
        };

        let response = handler.get_contract_info(query).await.unwrap();
        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidContractId as i32
        );
        assert!(response.contract_info.is_none());
    }

    #[tokio::test]
    async fn test_get_contract_bytecode_found() {
        let contract_id = ContractId {
            contract: Some(contract_id::Contract::ContractNum(7003)),
            ..Default::default()
        };
        let bytecode = Bytecode {
            code: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::BytecodeValue(bytecode)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_bytecode_db_key(&contract_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = ContractQueryHandler::new(Arc::new(mock_reader));
        let query = ContractGetBytecodeQuery {
            contract_id: Some(contract_id),
            header: None,
        };

        let response = handler.get_contract_bytecode(query).await.unwrap();
        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        assert_eq!(response.bytecode, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[tokio::test]
    async fn test_get_contract_bytecode_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = ContractQueryHandler::new(Arc::new(mock_reader));
        let query = ContractGetBytecodeQuery {
            contract_id: Some(ContractId {
                contract: Some(contract_id::Contract::ContractNum(7004)),
                ..Default::default()
            }),
            header: None,
        };

        let response = handler.get_contract_bytecode(query).await.unwrap();
        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidContractId as i32
        );
        assert!(response.bytecode.is_empty());
    }
}
