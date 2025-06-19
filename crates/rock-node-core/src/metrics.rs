use prometheus::{
    self, CounterVec, Encoder, HistogramVec, IntCounter, IntGauge, Opts, Registry, TextEncoder,
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
    pub publish_responses_sent_total: CounterVec,
}

impl MetricsRegistry {
    /// Creates a new `MetricsRegistry` and registers all the defined metrics.
    /// This should be called once at application startup.
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        // --- Core Metrics Initialization ---
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

        let publish_responses_sent_total = CounterVec::new(
            Opts::new(
                "rocknode_publish_responses_sent_total",
                "Total number of different response types sent back to publisher clients.",
            ),
            &["response_type"],
        )?;
        registry.register(Box::new(publish_responses_sent_total.clone()))?;

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
            publish_responses_sent_total,
        })
    }

    /// Gathers all registered metrics and encodes them into the Prometheus text format.
    pub fn gather(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();

        // Gather metrics from the registry.
        let metric_families = self.registry.gather();
        // Encode them into the buffer.
        encoder.encode(&metric_families, &mut buffer).unwrap();

        buffer
    }
}
