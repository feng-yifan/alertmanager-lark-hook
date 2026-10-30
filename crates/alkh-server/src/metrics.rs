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
    pub messages_pinned_total: AtomicU64,
    pub messages_unpinned_total: AtomicU64,
    pub pins_ignored_total: AtomicU64,
    pub deliveries_failed_total: AtomicU64,
    pub deliveries_suppressed_total: AtomicU64,
    pub delivery_retries_total: AtomicU64,
    pub aggregator_flushed_total: AtomicU64,
    pub aggregator_requeued_total: AtomicU64,
    pub aggregator_dropped_total: AtomicU64,
    pub lark_api_errors_total: AtomicU64,
    pub render_failures_total: AtomicU64,
}

impl alkh_domain::ports::MetricsSink for Metrics {
    fn incr(&self, counter: alkh_domain::ports::Counter, by: u64) {
        use alkh_domain::ports::Counter;
        let target: &AtomicU64 = match counter {
            Counter::RenderFailure => &self.render_failures_total,
            Counter::LarkApiError => &self.lark_api_errors_total,
            Counter::DeliveryRecalled => &self.deliveries_recalled_total,
            Counter::MessagePinned => &self.messages_pinned_total,
            Counter::MessageUnpinned => &self.messages_unpinned_total,
            Counter::PinsIgnored => &self.pins_ignored_total,
        };
        Self::add(target, by);
    }
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
            "alkh_messages_pinned_total",
            "Alert cards pinned while unresolved",
            Self::get(&self.messages_pinned_total),
        );
        gauge(
            &mut out,
            "alkh_messages_unpinned_total",
            "Alert cards unpinned after resolution / policy change",
            Self::get(&self.messages_unpinned_total),
        );
        gauge(
            &mut out,
            "alkh_pins_ignored_total",
            "Alert groups set to ignore-and-unpin via card button",
            Self::get(&self.pins_ignored_total),
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
            "alkh_aggregator_requeued_total",
            "Aggregated events requeued after a recoverable failure",
            Self::get(&self.aggregator_requeued_total),
        );
        gauge(
            &mut out,
            "alkh_aggregator_dropped_total",
            "Aggregated events abandoned (unrecoverable)",
            Self::get(&self.aggregator_dropped_total),
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
        // 逐个断言「每个声明的计数器都被渲染」。写死数量既容易过时，也抓不到
        // 「声明了却忘了渲染」这种等于「永远不动的指标」的问题。
        let expected = [
            "alkh_http_requests_total",
            "alkh_http_errors_total",
            "alkh_hook_received_total",
            "alkh_hook_skipped_total",
            "alkh_hook_rejected_total",
            "alkh_alerts_firing_total",
            "alkh_alerts_resolved_total",
            "alkh_deliveries_sent_total",
            "alkh_deliveries_updated_total",
            "alkh_deliveries_recalled_total",
            "alkh_messages_pinned_total",
            "alkh_messages_unpinned_total",
            "alkh_pins_ignored_total",
            "alkh_deliveries_failed_total",
            "alkh_deliveries_suppressed_total",
            "alkh_delivery_retries_total",
            "alkh_aggregator_flushed_total",
            "alkh_aggregator_requeued_total",
            "alkh_aggregator_dropped_total",
            "alkh_lark_api_errors_total",
            "alkh_render_failures_total",
        ];
        for name in expected {
            assert!(
                text.contains(&format!("# HELP {name} ")),
                "指标未渲染: {name}"
            );
        }
        assert_eq!(
            text.lines().filter(|l| l.starts_with("# HELP")).count(),
            expected.len(),
            "渲染出的指标数量应与声明一致（多出来说明有指标没被登记）"
        );
    }
}
