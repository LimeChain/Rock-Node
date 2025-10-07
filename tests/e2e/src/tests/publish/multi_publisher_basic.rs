use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{BlockAcknowledgement, Response as PublishResponse, SkipBlock},
    BlockItemSet, PublishStreamRequest,
};
use serial_test::serial;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Tests that when 3 publishers race for the same block
/// - Exactly one wins (no response initially)
/// - Exactly two lose (get SkipBlock)
#[tokio::test]
#[serial]
async fn test_three_publishers_race_for_same_block() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Create 3 publisher clients
    let mut client_a = ctx.publisher_client().await?;
    let mut client_b = ctx.publisher_client().await?;
    let mut client_c = ctx.publisher_client().await?;

    // Create the same block header for all three
    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;
    let header_only = vec![block_proto.items[0].clone()];

    // Create channels for all three publishers
    let (tx_a, rx_a) = mpsc::channel(8);
    let (tx_b, rx_b) = mpsc::channel(8);
    let (tx_c, rx_c) = mpsc::channel(8);

    // Open streams for all three FIRST
    let mut responses_a = client_a
        .publish_block_stream(ReceiverStream::new(rx_a))
        .await?
        .into_inner();

    let mut responses_b = client_b
        .publish_block_stream(ReceiverStream::new(rx_b))
        .await?
        .into_inner();

    let mut responses_c = client_c
        .publish_block_stream(ReceiverStream::new(rx_c))
        .await?
        .into_inner();

    // All three send the header simultaneously
    let header_request = PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: header_only,
        })),
    };

    tx_a.send(header_request.clone()).await?;
    tx_b.send(header_request.clone()).await?;
    tx_c.send(header_request.clone()).await?;

    // Wait for initial responses from all three
    // Note: The winner won't get an immediate response (timeout is expected)
    // Losers will get SkipBlock immediately

    let response_a =
        match tokio::time::timeout(Duration::from_millis(500), responses_a.next()).await {
            Ok(Some(Ok(resp))) => Some(resp),
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => return Err(anyhow::anyhow!("Client A stream closed")),
            Err(_) => None, // Timeout = winner (no immediate response)
        };

    let response_b =
        match tokio::time::timeout(Duration::from_millis(500), responses_b.next()).await {
            Ok(Some(Ok(resp))) => Some(resp),
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => return Err(anyhow::anyhow!("Client B stream closed")),
            Err(_) => None, // Timeout = winner (no immediate response)
        };

    let response_c =
        match tokio::time::timeout(Duration::from_millis(500), responses_c.next()).await {
            Ok(Some(Ok(resp))) => Some(resp),
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => return Err(anyhow::anyhow!("Client C stream closed")),
            Err(_) => None, // Timeout = winner (no immediate response)
        };

    // Count how many got SkipBlock
    let mut skip_count = 0;
    let mut no_response_count = 0;

    for (name, resp) in [("A", response_a), ("B", response_b), ("C", response_c)] {
        match resp {
            Some(resp) => match resp.response {
                Some(PublishResponse::SkipBlock(skip)) => {
                    assert_eq!(
                        skip.block_number, 0,
                        "{} got SkipBlock for wrong block",
                        name
                    );
                    skip_count += 1;
                },
                Some(PublishResponse::Acknowledgement(_)) => {
                    // Might get broadcast ACK from another completed block
                    // This shouldn't happen in this test, but handle it gracefully
                },
                other => {
                    panic!("{} got unexpected response: {:?}", name, other);
                },
            },
            None => {
                // This is the winner - no immediate response
                no_response_count += 1;
            },
        }
    }

    // Exactly two should have gotten SkipBlock, and one should have no response (winner)
    assert_eq!(
        skip_count, 2,
        "Expected exactly 2 publishers to get SkipBlock, got {}",
        skip_count
    );
    assert_eq!(
        no_response_count, 1,
        "Expected exactly 1 winner (no response), got {}",
        no_response_count
    );

    println!("✅ Three publisher race: 1 winner, 2 skip blocks confirmed");

    Ok(())
}

/// Tests fairness in winner selection when multiple publishers compete for blocks.
/// Verifies:
/// - No single publisher monopolizes (max 60% wins)
/// - All publishers receive appropriate responses
/// - Most blocks are successfully processed
#[tokio::test]
#[serial]
async fn test_five_publishers_high_contention() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);
    let winners: Arc<Mutex<HashMap<u64, usize>>> = Arc::new(Mutex::new(HashMap::new()));

    let mut handles = vec![];

    // Spawn 5 publishers
    for publisher_id in 0..5 {
        let ctx = Arc::clone(&ctx);
        let winners = Arc::clone(&winners);

        let handle = tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(16);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            let mut my_wins = 0;

            // Try to publish blocks 0-9
            for block_num in 0u64..10 {
                let block_bytes = BlockBuilder::new(block_num).build();
                let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                    Message::decode(block_bytes.as_slice()).unwrap();

                // Send just the header to race
                let header = vec![block_proto.items[0].clone()];
                tx.send(PublishStreamRequest {
                    request: Some(PublishRequest::BlockItems(BlockItemSet {
                        block_items: header,
                    })),
                })
                .await
                .unwrap();

                // Wait for response - might receive broadcast ACKs or SkipBlock
                let mut got_skip = false;
                let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

                while tokio::time::Instant::now() < deadline {
                    if let Some(Ok(resp)) = tokio::time::timeout(
                        deadline - tokio::time::Instant::now(),
                        responses.next(),
                    )
                    .await
                    .ok()
                    .flatten()
                    {
                        match resp.response {
                            Some(PublishResponse::SkipBlock(skip)) => {
                                // We lost this block
                                if skip.block_number == block_num {
                                    got_skip = true;
                                    break;
                                }
                                // Else: SkipBlock for a future block, shouldn't happen but continue
                            },
                            Some(PublishResponse::Acknowledgement(ack)) => {
                                // Broadcast ACK from another publisher - ignore and keep waiting
                                if ack.block_number == block_num {
                                    // This shouldn't happen (we'd get SkipBlock first), but handle it
                                    got_skip = true;
                                    break;
                                }
                                // Continue waiting for SkipBlock
                            },
                            _ => {},
                        }
                    } else {
                        // Timeout - no response
                        break;
                    }
                }

                if !got_skip {
                    // No SkipBlock received, we might have won - complete the block
                    let proof = block_proto.items.last().cloned().unwrap();
                    tx.send(PublishStreamRequest {
                        request: Some(PublishRequest::BlockItems(BlockItemSet {
                            block_items: vec![proof],
                        })),
                    })
                    .await
                    .unwrap();

                    // Wait for ACK, ignoring any broadcast ACKs
                    let ack_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    while tokio::time::Instant::now() < ack_deadline {
                        if let Some(Ok(ack_resp)) = tokio::time::timeout(
                            ack_deadline - tokio::time::Instant::now(),
                            responses.next(),
                        )
                        .await
                        .ok()
                        .flatten()
                        {
                            if let Some(PublishResponse::Acknowledgement(ack)) = ack_resp.response {
                                if ack.block_number == block_num {
                                    my_wins += 1;
                                    winners.lock().await.insert(block_num, publisher_id);
                                    break;
                                }
                                // Else: ACK for different block (broadcast), continue waiting
                            }
                        } else {
                            break;
                        }
                    }
                }

                // Small delay between blocks
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            my_wins
        });

        handles.push(handle);
    }

    // Wait for all publishers to finish
    let mut results = vec![];
    for handle in handles {
        results.push(handle.await);
    }

    // Collect win counts
    let mut total_wins = 0;
    for (i, result) in results.iter().enumerate() {
        match result {
            Ok(wins) => {
                println!("Publisher {} won {} blocks", i, wins);
                total_wins += wins;
            },
            Err(e) => {
                println!("Publisher {} task failed: {}", i, e);
            },
        }
    }

    // Verify we have winners for most blocks (allowing for some contention issues)
    let winners_map = winners.lock().await;
    println!("Total blocks with confirmed winners: {}", winners_map.len());

    // At least 7 out of 10 blocks should have been completed successfully
    assert!(
        winners_map.len() >= 7,
        "Expected at least 7 blocks to be completed, got {}",
        winners_map.len()
    );

    // Fairness check: No single publisher should win more than 60% of blocks
    for (i, result) in results.iter().enumerate() {
        if let Ok(wins) = result {
            let win_rate = *wins as f64 / winners_map.len() as f64;
            assert!(
                win_rate <= 0.6,
                "Publisher {} won {:.0}% of blocks ({}), exceeds 60% fairness threshold",
                i,
                win_rate * 100.0,
                wins
            );
        }
    }

    println!("✅ Five publishers high contention: Fairness and completion verified");

    Ok(())
}

/// Tests that winner selection isn't sticky to one publisher.
/// Verifies:
/// - Publisher A wins block 0
/// - Publisher B wins block 1
/// - Publisher A wins block 2
/// - All blocks are persisted in order
#[tokio::test]
#[serial]
async fn test_different_publishers_win_consecutive_blocks() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    // Synchronization channels:
    // - block0_done: A signals when block 0 is complete
    // - block1_done: B signals when block 1 is complete
    let (block0_done_tx, mut block0_done_rx) = mpsc::channel::<()>(1);
    let (block1_done_tx, mut block1_done_rx) = mpsc::channel::<()>(1);

    // Publisher A: publishes blocks 0 and 2
    let publisher_a_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(16);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            // Block 0 - should WIN
            let block0_bytes = BlockBuilder::new(0).build();
            let block0_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block0_bytes.as_slice()).unwrap();

            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block0_proto.items,
                })),
            })
            .await
            .unwrap();

            // Wait for ACK for block 0
            let resp0 = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp0.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 0,
                        ..
                    }))
                ),
                "Publisher A should get ACK for block 0"
            );
            let _ = block0_done_tx.send(()).await; // Signal block 0 complete

            // Wait for block 1 to complete (B's turn)
            // First receive the ACK broadcast for block 1
            let resp1_ack = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp1_ack.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 1,
                        ..
                    }))
                ),
                "Publisher A should get ACK broadcast for block 1"
            );

            // Wait for signal that block 1 is complete
            let _ = block1_done_rx.recv().await;

            // Block 2 - should WIN
            let block2_bytes = BlockBuilder::new(2).build();
            let block2_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block2_bytes.as_slice()).unwrap();

            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block2_proto.items,
                })),
            })
            .await
            .unwrap();

            // Wait for ACK for block 2
            let resp2 = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp2.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 2,
                        ..
                    }))
                ),
                "Publisher A should get ACK for block 2"
            );

            println!("✅ Publisher A: Won blocks 0 and 2");
        })
    };

    // Publisher B: publishes block 1
    let publisher_b_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(16);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            // Wait for block 0 to complete (A's turn)
            let _ = block0_done_rx.recv().await;

            // Receive the ACK broadcast for block 0
            let resp0_ack = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp0_ack.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 0,
                        ..
                    }))
                ),
                "Publisher B should get ACK broadcast for block 0"
            );

            // Block 1 - should WIN
            let block1_bytes = BlockBuilder::new(1).build();
            let block1_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block1_bytes.as_slice()).unwrap();

            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block1_proto.items,
                })),
            })
            .await
            .unwrap();

            // Wait for ACK for block 1
            let resp1 = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp1.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 1,
                        ..
                    }))
                ),
                "Publisher B should get ACK for block 1"
            );
            let _ = block1_done_tx.send(()).await; // Signal block 1 complete

            // Wait for block 2 ACK broadcast from A
            let resp2_ack = responses.next().await.unwrap().unwrap();
            assert!(
                matches!(
                    resp2_ack.response,
                    Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                        block_number: 2,
                        ..
                    }))
                ),
                "Publisher B should get ACK broadcast for block 2"
            );

            println!("✅ Publisher B: Won block 1");
        })
    };

    // Wait for both to complete
    publisher_a_task.await?;
    publisher_b_task.await?;

    // Verify all 3 blocks were persisted
    use rock_node_protobufs::org::hiero::block::api::{
        block_request::BlockSpecifier, BlockRequest,
    };
    let access_client = ctx.access_client().await?;
    for block_num in 0u64..3 {
        let request = BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(block_num)),
        };
        let response = access_client.clone().get_block(request).await?.into_inner();

        assert!(
            response.block.is_some(),
            "Block {} should be persisted",
            block_num
        );
    }

    println!("✅ Different publishers win consecutive blocks: All blocks persisted correctly");

    Ok(())
}
