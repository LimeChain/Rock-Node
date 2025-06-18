use prometheus::{self, CounterVec, Encoder, HistogramVec, IntCounter, IntGauge, Opts, Registry, TextEncoder};

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
}

impl MetricsRegistry {
    /// Creates a new `MetricsRegistry` and registers all the defined metrics.
    /// This should be called once at application startup.
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        // --- Core Metrics Initialization ---
        let blocks_acknowledged = IntCounter::with_opts(Opts::new(
            "rocknode_blocks_acknowledged",
            "Total number of blocks acknowledged",
        ))?;
        registry.register(Box::new(blocks_acknowledged.clone()))?;

        let active_publish_sessions = IntGauge::with_opts(Opts::new(
            "rocknode_active_publish_sessions",
            "Number of currently active gRPC publisher sessions",
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

        Ok(Self {
            registry,
            blocks_acknowledged,
            active_publish_sessions,
            block_access_requests_total,
            block_access_request_duration_seconds,
            block_access_latest_available_block,
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
