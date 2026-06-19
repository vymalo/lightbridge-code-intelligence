//! Prometheus metrics. A single global recorder is installed once; both the `serve` and
//! `dispatcher` roles expose its text rendering at `/metrics` (scraped by Alloy). Instrumentation
//! itself uses the `metrics` facade macros (`counter!`, `histogram!`) sprinkled across the code.

use std::sync::OnceLock;

use metrics::{counter, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the global Prometheus recorder (idempotent) and return a clonable render handle.
pub fn install() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("install prometheus recorder")
        })
        .clone()
}

// --- Instrumentation helpers (keep metric names in one place; see the Operations dashboard) ---

/// An HTTP request: count by method/route/status, latency by method/route.
pub fn http_request(method: &str, path: &str, status: &str, seconds: f64) {
    counter!(
        "http_requests_total",
        "method" => method.to_string(),
        "path" => path.to_string(),
        "status" => status.to_string(),
    )
    .increment(1);
    histogram!(
        "http_request_duration_seconds",
        "method" => method.to_string(),
        "path" => path.to_string(),
    )
    .record(seconds);
}

/// An accepted (verified, non-duplicate) webhook delivery, labelled by event type.
pub fn webhook_delivery(event: &str) {
    counter!("lci_webhook_deliveries_total", "event" => event.to_string()).increment(1);
}

pub fn webhook_signature_failure() {
    counter!("lci_webhook_signature_failures_total").increment(1);
}

pub fn webhook_duplicate() {
    counter!("lci_webhook_duplicate_deliveries_total").increment(1);
}

pub fn task_created() {
    counter!("lci_tasks_created_total").increment(1);
}

/// A dispatch attempt outcome: `launched` or `failed`.
pub fn dispatch_outcome(outcome: &str) {
    counter!("lci_dispatch_jobs_total", "outcome" => outcome.to_string()).increment(1);
}

/// Seconds spent claiming + launching a task's Job.
pub fn dispatch_claim_seconds(seconds: f64) {
    histogram!("lci_dispatch_claim_seconds").record(seconds);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_metrics_appear_in_the_render() {
        let handle = install();
        webhook_delivery("pull_request");
        webhook_signature_failure();
        dispatch_outcome("launched");
        let rendered = handle.render();
        assert!(rendered.contains("lci_webhook_deliveries_total"));
        assert!(rendered.contains("event=\"pull_request\""));
        assert!(rendered.contains("lci_webhook_signature_failures_total"));
        assert!(rendered.contains("outcome=\"launched\""));
    }
}
