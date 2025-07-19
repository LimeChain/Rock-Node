use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest,
};
use serial_test::serial;

/// Test Case: Get Existing Block
///
/// Objective: Verify that a client can fetch a specific, single historical
/// block by its number.
#[tokio::test]
#[serial]
async fn test_get_block_by_number_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    publish_blocks(&ctx, 0, 5).await?;
    let mut access_client = ctx.access_client().await?;

    let request = BlockRequest {
        block_specifier: Some(BlockSpecifier::BlockNumber(3)),
    };
    let response = access_client.get_block(request).await?.into_inner();

    assert_eq!(
        response.status,
        block_response::Code::Success as i32,
        "Response status should be SUCCESS",
    );

    let block = response.block.expect("Response should contain a block");

    let header = block.items.iter().find_map(|item| match &item.item {
        Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h)) => Some(h),
        _ => None,
    }).expect("Returned block must have a header");

    assert_eq!(header.number, 3, "Returned block has the wrong number");
    Ok(())
}

/// Test Case: Get Non-Existent Block
///
/// Objective: Verify that requesting a block that has not been published
/// results in a NOT_AVAILABLE status.
#[tokio::test]
#[serial]
async fn test_get_non_existent_block() -> Result<()> {
    let ctx = TestContext::new().await?;
    publish_blocks(&ctx, 0, 5).await?;
    let mut access_client = ctx.access_client().await?;

    let request = BlockRequest {
        block_specifier: Some(BlockSpecifier::BlockNumber(10)),
    };
    let response = access_client.get_block(request).await?.into_inner();
    println!("Response: {:?}", response);
    assert_eq!(
        response.status,
        block_response::Code::NotAvailable as i32,
        "Response status should be NOT_AVAILABLE",
    );
    assert!(
        response.block.is_none(),
        "Response should not contain a block"
    );
    Ok(())
}
