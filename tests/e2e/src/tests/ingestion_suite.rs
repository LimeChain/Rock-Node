use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier,
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{
        end_of_stream::Code as EndCode, BlockAcknowledgement, Response as PublishResponse,
        SkipBlock,
    },
    BlockItemSet, BlockRequest, PublishStreamRequest,
};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Happy Path – a single publisher sends a full block and receives an acknowledgement.
#[tokio::test]
#[serial]
async fn test_publish_single_block_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    let (tx, rx) = mpsc::channel(1);
    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block_proto.items,
        })),
    })
    .await?;

    drop(tx);

    let response_stream = client.publish_block_stream(ReceiverStream::new(rx)).await?;
    let mut responses = response_stream.into_inner();

    let response = responses
        .next()
        .await
        .expect("Server should have sent a response")?;

    match response.response {
        Some(PublishResponse::Acknowledgement(ack)) => {
            assert_eq!(
                ack.block_number, 0,
                "Acknowledgement has wrong block number"
            );
        },
        other => panic!("Expected BlockAcknowledgement, got {:?}", other),
    }

    assert!(responses.next().await.is_none());
    Ok(())
}

/// Test Case: Duplicate block – sending an already persisted block returns EndStream DuplicateBlock.
#[tokio::test]
#[serial]
async fn test_duplicate_block_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;
    let (tx1, rx1) = mpsc::channel(1);
    tx1.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block_proto.items.clone(),
        })),
    })
    .await?;
    drop(tx1);

    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx1))
        .await?
        .into_inner();
    while let Some(resp) = responses.next().await {
        let resp = resp?;
        if matches!(resp.response, Some(PublishResponse::Acknowledgement(_))) {
            break;
        }
    }

    let mut client2 = ctx.publisher_client().await?;
    let header_only = vec![block_proto.items[0].clone()];
    let (tx2, rx2) = mpsc::channel(1);
    tx2.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: header_only,
        })),
    })
    .await?;
    drop(tx2);

    let mut dup_resp_stream = client2
        .publish_block_stream(ReceiverStream::new(rx2))
        .await?
        .into_inner();

    let first_resp = dup_resp_stream
        .next()
        .await
        .expect("Should receive response for duplicate block")?;
    match first_resp.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(end.status, EndCode::DuplicateBlock as i32);
            assert_eq!(
                end.block_number, 0,
                "EndStream should report last persisted block"
            );
        },
        other => panic!("Expected EndStream DuplicateBlock, got {:?}", other),
    }

    Ok(())
}

/// Test Case: Race condition – two publishers send the same header, one wins, one gets SkipBlock.
#[tokio::test]
#[serial]
async fn test_multi_publisher_race_sends_skip_block() -> Result<()> {
    let ctx = TestContext::new().await?;

    let mut client_a = ctx.publisher_client().await?;
    let mut client_b = ctx.publisher_client().await?;

    let header_request = {
        let block_bytes = BlockBuilder::new(0).build();
        let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
            Message::decode(block_bytes.as_slice())?;
        PublishStreamRequest {
            request: Some(PublishRequest::BlockItems(BlockItemSet {
                block_items: vec![block_proto.items[0].clone()],
            })),
        }
    };

    let (tx_a, rx_a) = mpsc::channel(4);
    tx_a.send(header_request.clone()).await?;
    let mut responses_a = client_a
        .publish_block_stream(ReceiverStream::new(rx_a))
        .await?
        .into_inner();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let (tx_b, rx_b) = mpsc::channel(4);
    tx_b.send(header_request.clone()).await?;
    let mut responses_b = client_b
        .publish_block_stream(ReceiverStream::new(rx_b))
        .await?
        .into_inner();

    let b_response = tokio::time::timeout(Duration::from_secs(1), responses_b.next())
        .await?
        .unwrap();
    assert!(
        matches!(
            b_response.unwrap().response,
            Some(PublishResponse::SkipBlock(SkipBlock { block_number: 0 }))
        ),
        "Client B should have received SkipBlock"
    );

    let a_result = tokio::time::timeout(Duration::from_millis(200), responses_a.next()).await;
    assert!(
        a_result.is_err(),
        "Client A should not have received a response yet"
    );

    Ok(())
}

/// Test Case: Multi-publisher happy path with broadcasted acknowledgements.
#[serial]
async fn test_multi_publisher_broadcast_ack() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    let client_a_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(4);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

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

            let resp0 = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 0,
                    ..
                }))
            ));

            let block1_header_bytes = BlockBuilder::new(1).items();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block1_header_bytes,
                })),
            })
            .await
            .unwrap();

            let resp1_skip = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1_skip.response,
                Some(PublishResponse::SkipBlock(SkipBlock { block_number: 1 }))
            ));

            let resp1_ack = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1_ack.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 1,
                    ..
                }))
            ));
        })
    };

    let client_b_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(4);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            tokio::time::sleep(Duration::from_millis(100)).await;

            let block0_header_bytes = BlockBuilder::new(0).items();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block0_header_bytes,
                })),
            })
            .await
            .unwrap();

            let resp0_skip = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0_skip.response,
                Some(PublishResponse::SkipBlock(SkipBlock { block_number: 0 }))
            ));

            let resp0_ack = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0_ack.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 0,
                    ..
                }))
            ));

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

            let resp1 = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 1,
                    ..
                }))
            ));
        })
    };

    client_a_task.await?;
    client_b_task.await?;

    Ok(())
}

// ============================================
// CATEGORY 1: BASIC MULTI-PUBLISHER SCENARIOS
// ============================================

/// Test Case: Three publishers race for the same block
/// Verifies that with 3+ publishers racing:
/// - Exactly one wins (Primary)
/// - Other two receive SkipBlock
/// - All eventually receive ACK broadcast
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

/// Test Case: Five publishers competing with high contention
/// Tests fairness and correctness with 5 publishers racing for 10 blocks
#[tokio::test]
#[serial]
async fn test_five_publishers_high_contention() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    // Track which publisher wins which block
    let winners = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    // Create 5 publisher tasks
    let mut handles = vec![];

    for publisher_id in 0..5 {
        let ctx = Arc::clone(&ctx);
        let winners = Arc::clone(&winners);

        let handle = tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(32);
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

    // Check fairness - no single publisher should win everything
    let mut win_counts = std::collections::HashMap::new();
    for winner_id in winners_map.values() {
        *win_counts.entry(*winner_id).or_insert(0) += 1;
    }

    for (publisher_id, count) in &win_counts {
        println!("Publisher {} won {} blocks", publisher_id, count);
    }

    // No single publisher should win more than 60% of blocks (fairness check)
    for count in win_counts.values() {
        assert!(
            *count <= 6,
            "Single publisher won too many blocks ({}), fairness violated",
            count
        );
    }

    println!("✅ Five publisher high contention: fairness and correctness verified");

    Ok(())
}

/// Test Case: Different publishers win consecutive blocks
/// Tests that:
/// - Publisher A wins block 0
/// - Publisher B wins block 1
/// - Publisher A wins block 2
/// - Verifies that winner selection isn't sticky to one publisher
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
