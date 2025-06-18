use prometheus::{self, Encoder, IntCounter, IntGauge, Opts, Registry, TextEncoder};

/// A registry for all Prometheus metrics in the application.
///
/// This struct holds the collectors for all our metrics. It is created once
/// when the application starts and shared via the `AppContext`.
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    // The internal registry that holds all the metric collectors.
    registry: Registry,

    // --- DEFINE YOUR METRICS AS PUBLIC FIELDS ---

    /// A counter for the total number of blocks processed and persisted.
    pub blocks_acknowledged: IntCounter,
    /// A gauge representing the number of currently active publisher gRPC streams.
    pub active_publish_sessions: IntGauge,
    // Add more metrics here as your application grows.
}

impl MetricsRegistry {
    /// Creates a new `MetricsRegistry` and registers all the defined metrics.
    /// This should be called once at application startup.
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        // --- INITIALIZE AND REGISTER EACH METRIC ---

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

        Ok(Self {
            registry,
            blocks_acknowledged,
            active_publish_sessions,
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
