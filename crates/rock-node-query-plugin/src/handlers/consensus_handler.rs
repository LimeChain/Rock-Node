use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        ConsensusGetTopicInfoQuery, ConsensusGetTopicInfoResponse, ConsensusTopicInfo,
        ResponseCodeEnum,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `ConsensusService`.
#[derive(Debug)]
pub struct ConsensusQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl ConsensusQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the topic info for a given topic_id.
    ///
    /// # Arguments
    ///
    /// * `query` - The ConsensusGetTopicInfoQuery containing the topic_id
    ///
    /// # Returns
    ///
    /// * `ConsensusGetTopicInfoResponse` - The response containing the topic info
    pub async fn get_topic_info(
        &self,
        query: ConsensusGetTopicInfoQuery,
    ) -> Result<ConsensusGetTopicInfoResponse, Status> {
        trace!("Entering get_topic_info for query: {:?}", query);

        let topic_id = query.topic_id.ok_or_else(|| {
            Status::invalid_argument("Missing topic_id in ConsensusGetTopicInfoQuery")
        })?;

        let state_id = StateIdentifier::StateIdTopics as u32;

        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TopicIdKey(topic_id.clone())),
        };

        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();
        let topic_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match topic_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::TopicValue(topic)) =
                    map_change_value.value_choice
                {
                    let topic_info = ConsensusTopicInfo {
                        memo: topic.memo,
                        running_hash: topic.running_hash,
                        sequence_number: topic.sequence_number as u64,
                        expiration_time: Some(rock_node_protobufs::proto::Timestamp {
                            seconds: topic.expiration_second,
                            nanos: 0,
                        }),
                        admin_key: topic.admin_key,
                        submit_key: topic.submit_key,
                        auto_renew_period: Some(rock_node_protobufs::proto::Duration {
                            seconds: topic.auto_renew_period,
                        }),
                        auto_renew_account: topic.auto_renew_account_id,
                        ledger_id: vec![],
                        fee_schedule_key: topic.fee_schedule_key,
                        fee_exempt_key_list: topic.fee_exempt_key_list,
                        custom_fees: topic.custom_fees,
                    };

                    ConsensusGetTopicInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        topic_id: Some(topic_id),
                        topic_info: Some(topic_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Topic value, found other type",
                    ));
                }
            }
            None => {
                trace!("No topic found for the given topic_id");
                ConsensusGetTopicInfoResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidTopicId, 0)),
                    topic_id: Some(topic_id),
                    topic_info: None,
                }
            }
        };

        trace!(
            "Exiting get_topic_info with response code: {:?}",
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

#[cfg(test)]
mod tests {
    use rock_node_protobufs::proto::{Topic, TopicId};

    use super::*;
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

    // Helper function to generate the database key, mirroring the handler's logic.
    fn generate_db_key(topic_id: &TopicId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdTopics as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::TopicIdKey(topic_id.clone())),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    #[tokio::test]
    async fn test_get_topic_info_found() {
        let topic_id = TopicId {
            shard_num: 0,
            realm_num: 0,
            topic_num: 1001,
        };
        let topic = Topic {
            topic_id: Some(topic_id.clone()),
            memo: "test_memo".to_string(),
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::TopicValue(topic)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_db_key(&topic_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = ConsensusQueryHandler::new(Arc::new(mock_reader));
        let query = ConsensusGetTopicInfoQuery {
            topic_id: Some(topic_id.clone()),
            header: None,
        };

        let response = handler.get_topic_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let topic_info = response.topic_info.unwrap();
        assert_eq!(topic_info.memo, "test_memo");
        assert_eq!(response.topic_id.unwrap(), topic_id);
    }

    #[tokio::test]
    async fn test_get_topic_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = ConsensusQueryHandler::new(Arc::new(mock_reader));
        let query = ConsensusGetTopicInfoQuery {
            topic_id: Some(TopicId {
                shard_num: 0,
                realm_num: 0,
                topic_num: 1002,
            }),
            header: None,
        };

        let response = handler.get_topic_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidTopicId as i32
        );
        assert!(response.topic_info.is_none());
    }
}
