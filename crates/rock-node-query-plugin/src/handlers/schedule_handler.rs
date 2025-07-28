use anyhow::Result;
use prost::Message;
use rock_node_core::StateReader;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::output::{
        map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
    },
    proto::{ResponseCodeEnum, ScheduleGetInfoQuery, ScheduleGetInfoResponse, ScheduleInfo},
};
use std::sync::Arc;
use tonic::Status;
use tracing::trace;

/// Contains the business logic for handling all queries related to the `ScheduleService`.
#[derive(Debug)]
pub struct ScheduleQueryHandler {
    state_reader: Arc<dyn StateReader>,
}

impl ScheduleQueryHandler {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }

    /// Get the schedule info for a given schedule_id.
    ///
    /// # Arguments
    ///
    /// * `query` - The ScheduleGetInfoQuery containing the schedule_id
    ///
    /// # Returns
    ///
    /// * `ScheduleGetInfoResponse` - The response containing the schedule info
    pub async fn get_schedule_info(
        &self,
        query: ScheduleGetInfoQuery,
    ) -> Result<ScheduleGetInfoResponse, Status> {
        trace!("Entering get_schedule_info for query: {:?}", query);

        let schedule_id = query.schedule_id.ok_or_else(|| {
            Status::invalid_argument("Missing schedule_id in ScheduleGetInfoQuery")
        })?;

        let state_id = StateIdentifier::StateIdSchedulesById as u32;

        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::ScheduleIdKey(
                schedule_id.clone(),
            )),
        };

        let db_key = [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat();
        let schedule_bytes = self
            .state_reader
            .get_state_value(&db_key)
            .map_err(|e| Status::internal(format!("Failed to query state: {}", e)))?;

        let response = match schedule_bytes {
            Some(bytes) => {
                let map_change_value: MapChangeValue = MapChangeValue::decode(bytes.as_slice())
                    .map_err(|e| {
                        Status::internal(format!("Failed to decode MapChangeValue: {}", e))
                    })?;

                if let Some(map_change_value::ValueChoice::ScheduleValue(schedule)) =
                    map_change_value.value_choice
                {
                    let schedule_info = ScheduleInfo {
                        schedule_id: Some(schedule_id.clone()),
                        memo: schedule.memo,
                        admin_key: schedule.admin_key,
                        payer_account_id: schedule.payer_account_id,
                        scheduled_transaction_body: schedule.scheduled_transaction,
                        signers: Some(rock_node_protobufs::proto::KeyList {
                            keys: schedule.signatories,
                        }),
                        creator_account_id: schedule.scheduler_account_id,
                        wait_for_expiry: schedule.wait_for_expiry,
                        ..Default::default()
                    };

                    ScheduleGetInfoResponse {
                        header: Some(build_response_header(ResponseCodeEnum::Ok, 0)),
                        schedule_info: Some(schedule_info),
                    }
                } else {
                    return Err(Status::internal(
                        "State inconsistency: Expected Schedule value, found other type",
                    ));
                }
            }
            None => {
                trace!("No schedule found for the given schedule_id");
                ScheduleGetInfoResponse {
                    header: Some(build_response_header(
                        ResponseCodeEnum::InvalidScheduleId,
                        0,
                    )),
                    schedule_info: None,
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
