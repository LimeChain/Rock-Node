use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{
        file_get_contents_response, file_get_info_response, FileGetContentsQuery,
        FileGetContentsResponse, FileGetInfoQuery, FileGetInfoResponse, ResponseCodeEnum,
        Timestamp,
    },
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `FileService`.
#[derive(Debug)]
pub struct FileQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl FileQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the file info for a given file_id.
    pub async fn get_file_info(
        &self,
        query: FileGetInfoQuery,
    ) -> Result<FileGetInfoResponse, Status> {
        trace!("Entering get_file_info for query: {:?}", query);

        let file_id = query
            .file_id
            .ok_or_else(|| Status::invalid_argument("Missing file_id in FileGetInfoQuery"))?;

        let state_id = StateIdentifier::StateIdFiles as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::FileIdKey(file_id.clone())),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let file_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match file_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::FileValue(file)) =
                    map_change_value.value_choice
                {
                    let file_info = file_get_info_response::FileInfo {
                        file_id: Some(file_id.clone()),
                        size: file.contents.len() as i64,
                        expiration_time: if file.expiration_second > 0 {
                            Some(Timestamp {
                                seconds: file.expiration_second,
                                nanos: 0,
                            })
                        } else {
                            None
                        },
                        deleted: file.deleted,
                        keys: file.keys,
                        memo: file.memo,
                        ledger_id: vec![], // Not supported yet
                    };
                    FileGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        file_info: Some(file_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected File value, found other type",
                    ));
                }
            }
            None => {
                trace!("No file found for the given file_id");
                FileGetInfoResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidFileId, 0)),
                    file_info: None,
                }
            }
        };
        Ok(response)
    }

    /// Get the file content for a given file_id.
    pub async fn get_file_content(
        &self,
        query: FileGetContentsQuery,
    ) -> Result<FileGetContentsResponse, Status> {
        trace!("Entering get_file_content for query: {:?}", query);

        let file_id = query
            .file_id
            .ok_or_else(|| Status::invalid_argument("Missing file_id in FileGetContentsQuery"))?;

        let state_id = StateIdentifier::StateIdFiles as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::FileIdKey(file_id.clone())),
        };
        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();

        let file_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match file_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::FileValue(file)) =
                    map_change_value.value_choice
                {
                    let file_contents = file_get_contents_response::FileContents {
                        file_id: Some(file_id.clone()),
                        contents: file.contents,
                    };
                    FileGetContentsResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        file_contents: Some(file_contents),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected File value, found other type",
                    ));
                }
            }
            None => {
                trace!("No file found for the given file_id");
                FileGetContentsResponse {
                    header: Some(build_response_header(ResponseCodeEnum::InvalidFileId, 0)),
                    file_contents: None,
                }
            }
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
    use rock_node_protobufs::proto::{File, FileId};
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
    fn generate_db_key(file_id: &FileId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdFiles as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::FileIdKey(file_id.clone())),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    #[tokio::test]
    async fn test_get_file_info_found() {
        let file_id = FileId {
            shard_num: 0,
            realm_num: 0,
            file_num: 42,
        };
        let file = File {
            file_id: Some(file_id.clone()),
            memo: "test-file".to_string(),
            contents: vec![1, 2, 3],
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::FileValue(file)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_db_key(&file_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = FileQueryHandler::new(Arc::new(mock_reader));
        let query = FileGetInfoQuery {
            file_id: Some(file_id.clone()),
            header: None,
        };

        let response = handler.get_file_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let file_info = response.file_info.unwrap();
        assert_eq!(file_info.memo, "test-file");
        assert_eq!(file_info.size, 3);
        assert_eq!(file_info.file_id.unwrap(), file_id);
    }

    #[tokio::test]
    async fn test_get_file_info_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = FileQueryHandler::new(Arc::new(mock_reader));
        let query = FileGetInfoQuery {
            file_id: Some(FileId {
                shard_num: 0,
                realm_num: 0,
                file_num: 99,
            }),
            header: None,
        };

        let response = handler.get_file_info(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidFileId as i32
        );
        assert!(response.file_info.is_none());
    }

    #[tokio::test]
    async fn test_get_file_content_found() {
        let file_id = FileId {
            shard_num: 0,
            realm_num: 0,
            file_num: 43,
        };
        let file_content_bytes = vec![10, 20, 30, 40, 50];
        let file = File {
            file_id: Some(file_id.clone()),
            contents: file_content_bytes.clone(),
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::FileValue(file)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_db_key(&file_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let handler = FileQueryHandler::new(Arc::new(mock_reader));
        let query = FileGetContentsQuery {
            file_id: Some(file_id.clone()),
            header: None,
        };

        let response = handler.get_file_content(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::Ok as i32
        );
        let file_contents = response.file_contents.unwrap();
        assert_eq!(file_contents.contents, file_content_bytes);
        assert_eq!(file_contents.file_id.unwrap(), file_id);
    }

    #[tokio::test]
    async fn test_get_file_content_not_found() {
        let mock_reader = MockStateReader::default();
        let handler = FileQueryHandler::new(Arc::new(mock_reader));
        let query = FileGetContentsQuery {
            file_id: Some(FileId {
                shard_num: 0,
                realm_num: 0,
                file_num: 100,
            }),
            header: None,
        };

        let response = handler.get_file_content(query).await.unwrap();

        assert_eq!(
            response.header.unwrap().node_transaction_precheck_code,
            ResponseCodeEnum::InvalidFileId as i32
        );
        assert!(response.file_contents.is_none());
    }
}
