use prometheus::{
    self, CounterVec, Encoder, GaugeVec, HistogramVec, IntCounter, IntGauge, Opts, Registry,
    TextEncoder,
};

/// A registry for all Prometheus metrics in the application.
///
/// This struct holds the collectors for all our metrics. It is created once
/// when the application starts and shared via the `AppContext`.
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    // The internal registry that holds all the metric collectors.
    registry: Registry,

    // --- Core Metrics ---
    pub blocks_acknowledged: IntCounter,
    pub active_publish_sessions: IntGauge,

    // --- Block Access Plugin Metrics ---
    pub block_access_requests_total: CounterVec,
    pub block_access_request_duration_seconds: HistogramVec,
    pub block_access_latest_available_block: IntGauge,

    // --- Status Plugin Metrics ---
    pub server_status_requests_total: CounterVec,
    pub server_status_request_duration_seconds: HistogramVec,
    pub server_status_earliest_available_block: IntGauge,
    pub server_status_latest_available_block: IntGauge,

    // --- Publish Plugin Metrics ---
    pub publish_blocks_received_total: CounterVec,
    pub publish_items_processed_total: IntCounter,
    pub publish_persistence_duration_seconds: HistogramVec,
    pub publish_lifecycle_duration_seconds: HistogramVec,
    pub publish_header_to_proof_duration_seconds: HistogramVec,
    pub publish_average_header_to_proof_time_seconds: GaugeVec,
    pub publish_responses_sent_total: CounterVec,

    // --- Persistence Plugin Metrics ---
    pub persistence_writes_total: CounterVec,
    pub persistence_write_duration_seconds: HistogramVec,
    pub persistence_event_duration_seconds: HistogramVec,
    pub persistence_reads_total: CounterVec,
    pub persistence_read_duration_seconds: HistogramVec,
    pub persistence_archival_cycles_total: IntCounter,
    pub persistence_archival_cycle_duration_seconds: HistogramVec,
    pub persistence_hot_tier_block_count: IntGauge,
    pub persistence_cold_tier_block_count: IntGauge,

    // --- Subscriber Plugin Metrics ---
    pub subscriber_active_sessions: IntGauge,
    pub subscriber_blocks_sent_total: CounterVec,
    pub subscriber_sessions_total: CounterVec,
    pub subscriber_average_inter_block_time_seconds: GaugeVec,

    // --- Backfill Plugin Metrics ---
    pub backfill_gaps_found_total: IntCounter,
    pub backfill_blocks_fetched_total: CounterVec,
    pub backfill_peer_connection_attempts_total: CounterVec,
    pub backfill_stream_duration_seconds: HistogramVec,
    pub backfill_active_streams: IntGauge,
    pub backfill_latest_continuous_block: IntGauge,
}

impl MetricsRegistry {
    /// Creates a new `MetricsRegistry` and registers all the defined metrics.
    pub fn new() -> Result<Self, prometheus::Error> {
        Self::with_registry(Registry::new())
    }

    /// Creates a new `MetricsRegistry` with a custom registry (useful for testing).
    pub fn with_registry(registry: Registry) -> Result<Self, prometheus::Error> {
        let blocks_acknowledged = IntCounter::with_opts(Opts::new(
            "rocknode_blocks_acknowledged",
            "Total number of blocks acknowledged by the publish plugin after successful persistence.",
        ))?;
        registry.register(Box::new(blocks_acknowledged.clone()))?;

        let active_publish_sessions = IntGauge::with_opts(Opts::new(
            "rocknode_active_publish_sessions",
            "Number of currently active gRPC publisher sessions.",
        ))?;
        registry.register(Box::new(active_publish_sessions.clone()))?;

        // --- Block Access Plugin Metrics Initialization ---
        let block_access_requests_total = CounterVec::new(
            Opts::new(
                "rocknode_block_access_requests_total",
                "The total number of getBlock gRPC requests processed.",
            ),
            &["status", "request_type"],
        )?;
        registry.register(Box::new(block_access_requests_total.clone()))?;

        let block_access_request_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_block_access_request_duration_seconds",
                "The duration of getBlock gRPC requests from start to finish, in seconds.",
            )
            .into(),
            &["status", "request_type"],
        )?;
        registry.register(Box::new(block_access_request_duration_seconds.clone()))?;

        let block_access_latest_available_block = IntGauge::with_opts(Opts::new(
            "rocknode_block_access_latest_available_block",
            "The most recent block number known to be available for serving.",
        ))?;
        registry.register(Box::new(block_access_latest_available_block.clone()))?;

        // --- Status Plugin Metrics Initialization ---
        let server_status_requests_total = CounterVec::new(
            Opts::new(
                "rocknode_server_status_requests_total",
                "The total number of server_status gRPC requests processed.",
            ),
            &["status"],
        )?;
        registry.register(Box::new(server_status_requests_total.clone()))?;

        let server_status_request_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_server_status_request_duration_seconds",
                "The duration of server_status gRPC requests, in seconds.",
            )
            .into(),
            &["status"],
        )?;
        registry.register(Box::new(server_status_request_duration_seconds.clone()))?;

        let server_status_earliest_available_block = IntGauge::with_opts(Opts::new(
            "rocknode_server_status_earliest_available_block",
            "The earliest block number reported by the status service.",
        ))?;
        registry.register(Box::new(server_status_earliest_available_block.clone()))?;

        let server_status_latest_available_block = IntGauge::with_opts(Opts::new(
            "rocknode_server_status_latest_available_block",
            "The latest block number reported by the status service.",
        ))?;
        registry.register(Box::new(server_status_latest_available_block.clone()))?;

        // --- Publish Plugin Metrics Initialization ---
        let publish_blocks_received_total = CounterVec::new(
            Opts::new(
                "rocknode_publish_blocks_received_total",
                "Total number of block headers received from publishers, categorized by outcome.",
            ),
            &["outcome"],
        )?;
        registry.register(Box::new(publish_blocks_received_total.clone()))?;

        let publish_items_processed_total = IntCounter::with_opts(Opts::new(
            "rocknode_publish_items_processed_total",
            "Total number of individual BlockItem messages processed by primary sessions.",
        ))?;
        registry.register(Box::new(publish_items_processed_total.clone()))?;

        let publish_persistence_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_publish_persistence_duration_seconds",
                "Duration from block publish to persistence acknowledgement.",
            )
            .into(),
            &["outcome"],
        )?;
        registry.register(Box::new(publish_persistence_duration_seconds.clone()))?;

        let publish_lifecycle_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_publish_lifecycle_duration_seconds",
                "Duration from receiving a block header to sending acknowledgement (or timeout) back to publisher.",
            )
            .into(),
            &["outcome"],
        )?;
        registry.register(Box::new(publish_lifecycle_duration_seconds.clone()))?;

        let publish_header_to_proof_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_publish_header_to_proof_duration_seconds",
                "Duration from receiving a block header to receiving its corresponding proof.",
            )
            .into(),
            &[],
        )?;
        registry.register(Box::new(publish_header_to_proof_duration_seconds.clone()))?;

        let publish_average_header_to_proof_time_seconds = GaugeVec::new(
            Opts::new(
                "rocknode_publish_average_header_to_proof_time_seconds",
                "Average time between header and proof for each publisher session.",
            ),
            &["session_id"],
        )?;
        registry.register(Box::new(
            publish_average_header_to_proof_time_seconds.clone(),
        ))?;

        let publish_responses_sent_total = CounterVec::new(
            Opts::new(
                "rocknode_publish_responses_sent_total",
                "Total number of different response types sent back to publisher clients.",
            ),
            &["response_type"],
        )?;
        registry.register(Box::new(publish_responses_sent_total.clone()))?;

        let persistence_writes_total = CounterVec::new(
            Opts::new(
                "rocknode_persistence_writes_total",
                "Total number of block writes, by type (live, batch).",
            ),
            &["type"],
        )?;
        registry.register(Box::new(persistence_writes_total.clone()))?;

        let persistence_write_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_persistence_write_duration_seconds",
                "Duration of block writes, by type.",
            )
            .into(),
            &["type"],
        )?;
        registry.register(Box::new(persistence_write_duration_seconds.clone()))?;

        let persistence_event_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_persistence_event_duration_seconds",
                "Duration from receiving a block event to emitting BlockPersisted, end-to-end latency.",
            )
            .into(),
            &[], // no labels
        )?;
        registry.register(Box::new(persistence_event_duration_seconds.clone()))?;

        let persistence_reads_total = CounterVec::new(
            Opts::new(
                "rocknode_persistence_reads_total",
                "Total number of block reads, by tier (hot, cold, not_found).",
            ),
            &["tier"],
        )?;
        registry.register(Box::new(persistence_reads_total.clone()))?;

        let persistence_read_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_persistence_read_duration_seconds",
                "Duration of block reads, by tier.",
            )
            .into(),
            &["tier"],
        )?;
        registry.register(Box::new(persistence_read_duration_seconds.clone()))?;

        let persistence_archival_cycles_total = IntCounter::with_opts(Opts::new(
            "rocknode_persistence_archival_cycles_total",
            "Total number of hot-to-cold archival cycles completed.",
        ))?;
        registry.register(Box::new(persistence_archival_cycles_total.clone()))?;

        let persistence_archival_cycle_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_persistence_archival_cycle_duration_seconds",
                "Duration of the hot-to-cold archival cycle.",
            )
            .into(),
            &[], // No labels needed for this one
        )?;
        registry.register(Box::new(
            persistence_archival_cycle_duration_seconds.clone(),
        ))?;

        let persistence_hot_tier_block_count = IntGauge::with_opts(Opts::new(
            "rocknode_persistence_hot_tier_block_count",
            "The current number of blocks in the hot tier storage.",
        ))?;
        registry.register(Box::new(persistence_hot_tier_block_count.clone()))?;

        let persistence_cold_tier_block_count = IntGauge::with_opts(Opts::new(
            "rocknode_persistence_cold_tier_block_count",
            "The current number of blocks indexed in the cold tier storage.",
        ))?;
        registry.register(Box::new(persistence_cold_tier_block_count.clone()))?;

        // --- Subscriber Plugin Metrics Initialization ---
        let subscriber_active_sessions = IntGauge::with_opts(Opts::new(
            "rocknode_subscriber_active_sessions",
            "Number of currently active gRPC subscriber sessions.",
        ))?;
        registry.register(Box::new(subscriber_active_sessions.clone()))?;

        let subscriber_blocks_sent_total = CounterVec::new(
            Opts::new(
                "rocknode_subscriber_blocks_sent_total",
                "Total number of blocks sent to subscribers, by type (historical, live).",
            ),
            &["type"],
        )?;
        registry.register(Box::new(subscriber_blocks_sent_total.clone()))?;

        let subscriber_sessions_total = CounterVec::new(
            Opts::new(
                "rocknode_subscriber_sessions_total",
                "Total number of subscriber sessions, labeled by the final outcome.",
            ),
            &["outcome"], // E.g., 'completed', 'client_disconnect', 'invalid_request'
        )?;
        registry.register(Box::new(subscriber_sessions_total.clone()))?;

        let subscriber_average_inter_block_time_seconds = GaugeVec::new(
            Opts::new(
                "rocknode_subscriber_average_inter_block_time_seconds",
                "Average time between blocks (seconds) for each subscriber session.",
            ),
            &["session_id"],
        )?;
        registry.register(Box::new(
            subscriber_average_inter_block_time_seconds.clone(),
        ))?;

        // --- Backfill Plugin Metrics Initialization ---
        let backfill_gaps_found_total = IntCounter::with_opts(Opts::new(
            "rocknode_backfill_gaps_found_total",
            "Total number of block gaps detected by the GapFill mode.",
        ))?;
        registry.register(Box::new(backfill_gaps_found_total.clone()))?;

        let backfill_blocks_fetched_total = CounterVec::new(
            Opts::new(
                "rocknode_backfill_blocks_fetched_total",
                "Total number of blocks successfully fetched from peers.",
            ),
            &["mode"],
        )?;
        registry.register(Box::new(backfill_blocks_fetched_total.clone()))?;

        let backfill_peer_connection_attempts_total = CounterVec::new(
            Opts::new(
                "rocknode_backfill_peer_connection_attempts_total",
                "Total number of connection attempts to peers.",
            ),
            &["peer_address", "outcome"],
        )?;
        registry.register(Box::new(backfill_peer_connection_attempts_total.clone()))?;

        let backfill_stream_duration_seconds = HistogramVec::new(
            Opts::new(
                "rocknode_backfill_stream_duration_seconds",
                "Duration of a backfill stream from a single peer.",
            )
            .into(),
            &["peer_address", "mode"],
        )?;
        registry.register(Box::new(backfill_stream_duration_seconds.clone()))?;

        let backfill_active_streams = IntGauge::with_opts(Opts::new(
            "rocknode_backfill_active_streams",
            "Number of currently active backfill streams.",
        ))?;
        registry.register(Box::new(backfill_active_streams.clone()))?;

        let backfill_latest_continuous_block = IntGauge::with_opts(Opts::new(
            "rocknode_backfill_latest_continuous_block",
            "The latest block number fetched in Continuous mode.",
        ))?;
        registry.register(Box::new(backfill_latest_continuous_block.clone()))?;

        Ok(Self {
            registry,
            blocks_acknowledged,
            active_publish_sessions,
            block_access_requests_total,
            block_access_request_duration_seconds,
            block_access_latest_available_block,
            server_status_requests_total,
            server_status_request_duration_seconds,
            server_status_earliest_available_block,
            server_status_latest_available_block,
            publish_blocks_received_total,
            publish_items_processed_total,
            publish_persistence_duration_seconds,
            publish_lifecycle_duration_seconds,
            publish_header_to_proof_duration_seconds,
            publish_average_header_to_proof_time_seconds,
            publish_responses_sent_total,
            persistence_writes_total,
            persistence_write_duration_seconds,
            persistence_event_duration_seconds,
            persistence_reads_total,
            persistence_read_duration_seconds,
            persistence_archival_cycles_total,
            persistence_archival_cycle_duration_seconds,
            persistence_hot_tier_block_count,
            persistence_cold_tier_block_count,
            subscriber_active_sessions,
            subscriber_blocks_sent_total,
            subscriber_sessions_total,
            subscriber_average_inter_block_time_seconds,
            backfill_gaps_found_total,
            backfill_blocks_fetched_total,
            backfill_peer_connection_attempts_total,
            backfill_stream_duration_seconds,
            backfill_active_streams,
            backfill_latest_continuous_block,
        })
    }

    /// Gathers all registered metrics and encodes them into the Prometheus text format.
    pub fn gather(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();

        // Gather metrics from the registry.
        let metric_families = self.registry.gather();
        // Encode them into the buffer.
        if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
            // Log the error but return what we have
            eprintln!("Failed to encode metrics: {}", e);
        }

        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_metrics_registry_new_success() {
        let metrics = MetricsRegistry::new();
        assert!(metrics.is_ok());
    }

    #[test]
    fn test_metrics_registry_with_custom_registry() {
        let custom_registry = Registry::new();
        let metrics = MetricsRegistry::with_registry(custom_registry);
        assert!(metrics.is_ok());
    }

    #[test]
    fn test_all_core_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test core metrics can be accessed
        metrics.blocks_acknowledged.inc();
        assert_eq!(metrics.blocks_acknowledged.get(), 1);

        metrics.active_publish_sessions.set(5);
        assert_eq!(metrics.active_publish_sessions.get(), 5);
    }

    #[test]
    fn test_block_access_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test block access metrics
        metrics
            .block_access_requests_total
            .with_label_values(&["success", "get_block"])
            .inc();

        metrics.block_access_latest_available_block.set(100);
        assert_eq!(metrics.block_access_latest_available_block.get(), 100);

        // Test histogram metric
        metrics
            .block_access_request_duration_seconds
            .with_label_values(&["success", "get_block"])
            .observe(0.5);
    }

    #[test]
    fn test_publish_plugin_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test publish metrics
        metrics
            .publish_blocks_received_total
            .with_label_values(&["success"])
            .inc();

        metrics.publish_items_processed_total.inc();
        assert_eq!(metrics.publish_items_processed_total.get(), 1);

        metrics
            .publish_persistence_duration_seconds
            .with_label_values(&["success"])
            .observe(1.5);
    }

    #[test]
    fn test_persistence_plugin_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test persistence metrics
        metrics
            .persistence_writes_total
            .with_label_values(&["live"])
            .inc();

        metrics.persistence_archival_cycles_total.inc();
        assert_eq!(metrics.persistence_archival_cycles_total.get(), 1);

        metrics.persistence_hot_tier_block_count.set(50);
        metrics.persistence_cold_tier_block_count.set(1000);
        assert_eq!(metrics.persistence_hot_tier_block_count.get(), 50);
        assert_eq!(metrics.persistence_cold_tier_block_count.get(), 1000);
    }

    #[test]
    fn test_subscriber_plugin_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test subscriber metrics
        metrics.subscriber_active_sessions.set(3);
        assert_eq!(metrics.subscriber_active_sessions.get(), 3);

        metrics
            .subscriber_blocks_sent_total
            .with_label_values(&["live"])
            .inc();

        metrics
            .subscriber_sessions_total
            .with_label_values(&["completed"])
            .inc();

        metrics
            .subscriber_average_inter_block_time_seconds
            .with_label_values(&["session_123"])
            .set(2.5);
    }

    #[test]
    fn test_backfill_plugin_metrics_are_registered() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test backfill metrics
        metrics.backfill_gaps_found_total.inc();
        assert_eq!(metrics.backfill_gaps_found_total.get(), 1);

        metrics
            .backfill_blocks_fetched_total
            .with_label_values(&["GapFill"])
            .inc();

        metrics
            .backfill_peer_connection_attempts_total
            .with_label_values(&["peer1", "success"])
            .inc();

        metrics.backfill_active_streams.set(2);
        metrics.backfill_latest_continuous_block.set(12345);
        assert_eq!(metrics.backfill_active_streams.get(), 2);
        assert_eq!(metrics.backfill_latest_continuous_block.get(), 12345);
    }

    #[test]
    fn test_metric_isolation_in_tests() {
        // Create two separate metric registries
        let metrics1 = MetricsRegistry::new().unwrap();
        let metrics2 = MetricsRegistry::new().unwrap();

        // Modify metrics in first registry
        metrics1.blocks_acknowledged.inc();
        metrics1.active_publish_sessions.set(5);

        // Verify second registry is isolated
        assert_eq!(metrics1.blocks_acknowledged.get(), 1);
        assert_eq!(metrics1.active_publish_sessions.get(), 5);
        assert_eq!(metrics2.blocks_acknowledged.get(), 0);
        assert_eq!(metrics2.active_publish_sessions.get(), 0);
    }

    #[test]
    fn test_prometheus_export_format() {
        let metrics = MetricsRegistry::new().unwrap();

        // Set some metric values
        metrics.blocks_acknowledged.inc();
        metrics.active_publish_sessions.set(3);
        metrics
            .block_access_requests_total
            .with_label_values(&["success", "get_block"])
            .inc();

        // Export to Prometheus format
        let exported = metrics.gather();
        let exported_str = String::from_utf8(exported).unwrap();

        // Verify the output contains expected metric names
        assert!(exported_str.contains("rocknode_blocks_acknowledged"));
        assert!(exported_str.contains("rocknode_active_publish_sessions"));
        assert!(exported_str.contains("rocknode_block_access_requests_total"));

        // Verify it contains metric values
        assert!(exported_str.contains("rocknode_blocks_acknowledged 1"));
        assert!(exported_str.contains("rocknode_active_publish_sessions 3"));
    }

    #[test]
    fn test_gather_with_empty_metrics() {
        let metrics = MetricsRegistry::new().unwrap();

        // Export without setting any values
        let exported = metrics.gather();
        let exported_str = String::from_utf8(exported).unwrap();

        // Should still contain metric definitions (with 0 values)
        assert!(exported_str.contains("rocknode_blocks_acknowledged"));
        assert!(exported_str.contains("rocknode_active_publish_sessions"));
    }

    #[tokio::test]
    async fn test_concurrent_metric_updates() {
        let metrics = Arc::new(MetricsRegistry::new().unwrap());

        // Spawn multiple tasks that update metrics concurrently
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let metrics = Arc::clone(&metrics);
                tokio::spawn(async move {
                    for _ in 0..100 {
                        metrics.blocks_acknowledged.inc();
                        metrics.active_publish_sessions.set(i as i64);
                        metrics
                            .block_access_requests_total
                            .with_label_values(&["success", "get_block"])
                            .inc();
                    }
                })
            })
            .collect();

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify final counts
        assert_eq!(metrics.blocks_acknowledged.get(), 1000); // 10 tasks * 100 increments
        assert!(metrics.active_publish_sessions.get() >= 0 && metrics.active_publish_sessions.get() < 10);
    }

    #[test]
    fn test_metrics_registry_debug_format() {
        let metrics = MetricsRegistry::new().unwrap();
        let debug_str = format!("{:?}", metrics);
        assert!(debug_str.contains("MetricsRegistry"));
    }

    #[test]
    fn test_metrics_registry_clone() {
        let metrics1 = MetricsRegistry::new().unwrap();
        let metrics2 = metrics1.clone();

        // Modify metrics through first instance
        metrics1.blocks_acknowledged.inc();

        // Verify both instances see the change (shared underlying registry)
        assert_eq!(metrics1.blocks_acknowledged.get(), 1);
        assert_eq!(metrics2.blocks_acknowledged.get(), 1);
    }

    #[test]
    fn test_custom_registry_isolation() {
        let registry1 = Registry::new();
        let registry2 = Registry::new();

        let metrics1 = MetricsRegistry::with_registry(registry1).unwrap();
        let metrics2 = MetricsRegistry::with_registry(registry2).unwrap();

        // Modify metrics in first registry
        metrics1.blocks_acknowledged.inc();

        // Verify isolation between registries
        assert_eq!(metrics1.blocks_acknowledged.get(), 1);
        assert_eq!(metrics2.blocks_acknowledged.get(), 0);

        // Verify exports are isolated
        let export1 = String::from_utf8(metrics1.gather()).unwrap();
        let export2 = String::from_utf8(metrics2.gather()).unwrap();

        assert!(export1.contains("rocknode_blocks_acknowledged 1"));
        assert!(export2.contains("rocknode_blocks_acknowledged 0"));
    }

    #[test]
    fn test_histogram_and_gauge_vec_metrics() {
        let metrics = MetricsRegistry::new().unwrap();

        // Test histogram metrics
        metrics
            .block_access_request_duration_seconds
            .with_label_values(&["success", "get_block"])
            .observe(0.1);
        metrics
            .block_access_request_duration_seconds
            .with_label_values(&["success", "get_block"])
            .observe(0.2);

        // Test gauge vec metrics
        metrics
            .publish_average_header_to_proof_time_seconds
            .with_label_values(&["session_1"])
            .set(1.5);
        metrics
            .subscriber_average_inter_block_time_seconds
            .with_label_values(&["session_2"])
            .set(2.0);

        // Verify metrics were recorded by checking the export
        let exported = String::from_utf8(metrics.gather()).unwrap();
        assert!(exported.contains("rocknode_block_access_request_duration_seconds"));
        assert!(exported.contains("rocknode_publish_average_header_to_proof_time_seconds"));
        assert!(exported.contains("rocknode_subscriber_average_inter_block_time_seconds"));
    }
}
