//! Optional process RSS snapshots, only in the offline replay executable.
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, filter_fn},
    layer::Context,
    prelude::*,
};
struct Memory;
impl<S: Subscriber> Layer<S> for Memory {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        struct Phase(Option<String>);
        impl Visit for Phase {
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "phase" {
                    self.0 = Some(value.to_owned());
                }
            }
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }
        let mut phase = Phase(None);
        event.record(&mut phase);
        if let (Some(phase), Some(stats)) = (phase.0, memory_stats::memory_stats()) {
            eprintln!(
                "MEMORY_SAMPLE {}",
                serde_json::json!({"phase":phase,"rss_bytes":stats.physical_mem,"timestamp":chrono::Utc::now().to_rfc3339()})
            );
        }
    }
}
pub fn init() {
    let memory = (std::env::var("LOAD_PROFILE_MEMORY").as_deref() == Ok("1"))
        .then(|| Memory.with_filter(filter_fn(|metadata| metadata.target() == "search_memory")));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_target(true)
                .with_filter(LevelFilter::INFO),
        )
        .with(memory)
        .init();
}
