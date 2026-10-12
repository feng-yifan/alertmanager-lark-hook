//! 轻量 Prometheus 指标注册表（原子计数，避免引入额外依赖）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct Metrics {
    pub http_requests_total: AtomicU64,
    pub http_errors_total: AtomicU64,
    pub hook_received_total: AtomicU64,
    pub hook_skipped_total: AtomicU64,
    pub hook_rejected_total: AtomicU64,
    pub alerts_firing_total: AtomicU64,
    pub alerts_resolved_total: AtomicU64,
    pub deliveries_sent_total: AtomicU64,
    pub deliveries_updated_total: AtomicU64,
    pub deliveries_recalled_total: AtomicU64,
    pub deliveries_failed_total: AtomicU64,
    pub deliveries_suppressed_total: AtomicU64,
    pub delivery_retries_total: AtomicU64,
    pub aggregator_flushed_total: AtomicU64,
    pub lark_api_errors_total: AtomicU64,
    pub render_failures_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    /// Prometheus 文本格式。
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(2048);
        let gauge = |out: &mut String, name: &str, help: &str, value: u64| {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
            ));
        };
        gauge(
            &mut out,
            "alkh_http_requests_total",
            "HTTP requests received",
            Self::get(&self.http_requests_total),
        );
        gauge(
            &mut out,
            "alkh_http_errors_total",
            "HTTP responses with status >= 400",
            Self::get(&self.http_errors_total),
        );
        gauge(
            &mut out,
            "alkh_hook_received_total",
            "AlertManager webhook calls accepted",
            Self::get(&self.hook_received_total),
        );
        gauge(
            &mut out,
            "alkh_hook_skipped_total",
            "Webhook calls skipped by policy",
            Self::get(&self.hook_skipped_total),
        );
        gauge(
            &mut out,
            "alkh_hook_rejected_total",
            "Webhook calls rejected (bad token/payload)",
            Self::get(&self.hook_rejected_total),
        );
        gauge(
            &mut out,
            "alkh_alerts_firing_total",
            "Firing alerts processed",
            Self::get(&self.alerts_firing_total),
        );
        gauge(
            &mut out,
            "alkh_alerts_resolved_total",
            "Resolved alerts processed",
            Self::get(&self.alerts_resolved_total),
        );
        gauge(
            &mut out,
            "alkh_deliveries_sent_total",
            "Messages sent to Feishu",
            Self::get(&self.deliveries_sent_total),
        );
        gauge(
            &mut out,
            "alkh_deliveries_updated_total",
            "Messages updated in place",
            Self::get(&self.deliveries_updated_total),
        );
        gauge(
            &mut out,
            "alkh_deliveries_recalled_total",
            "Messages recalled/deleted",
            Self::get(&self.deliveries_recalled_total),
        );
        gauge(
            &mut out,
            "alkh_deliveries_failed_total",
            "Deliveries failed after retries",
            Self::get(&self.deliveries_failed_total),
        );
        gauge(
            &mut out,
            "alkh_deliveries_suppressed_total",
            "Deliveries suppressed by policy",
            Self::get(&self.deliveries_suppressed_total),
        );
        gauge(
            &mut out,
            "alkh_delivery_retries_total",
            "Delivery retry attempts",
            Self::get(&self.delivery_retries_total),
        );
        gauge(
            &mut out,
            "alkh_aggregator_flushed_total",
            "Aggregated events flushed",
            Self::get(&self.aggregator_flushed_total),
        );
        gauge(
            &mut out,
            "alkh_lark_api_errors_total",
            "Feishu OpenAPI errors",
            Self::get(&self.lark_api_errors_total),
        );
        gauge(
            &mut out,
            "alkh_render_failures_total",
            "Card render failures",
            Self::get(&self.render_failures_total),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_all_metric_names() {
        let m = Metrics::new();
        Metrics::inc(&m.hook_received_total);
        Metrics::add(&m.deliveries_sent_total, 3);
        let text = m.render();
        assert!(text.contains("alkh_hook_received_total 1"));
        assert!(text.contains("alkh_deliveries_sent_total 3"));
        assert!(text.contains("# TYPE alkh_http_requests_total counter"));
        assert_eq!(text.lines().filter(|l| l.starts_with("# HELP")).count(), 16);
    }
}
