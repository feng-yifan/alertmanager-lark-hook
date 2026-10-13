//! 未认证入口的限流。
//!
//! hook 接收端点不需要认证（AlertManager 只带 token），任何来源都能打它，
//! 每次请求都会产生一次 DB 查询、并在 token 有效时触发飞书调用。这里做一层
//! **宽松**的固定窗口限流：目的是挡住单个来源把实例打满，而不是精细限速
//! ——AlertManager 在告警风暴时会有正当的突发，限太紧会丢真告警。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 固定窗口计数器（按 key）。
///
/// 只在进程内有效 —— 本项目明确单副本部署（嵌入式 SQLite 单写者 + 内存态聚合器 /
/// 限流器 / 卡片序号），与 `deploy/kubernetes.yaml` 的 `replicas: 1` 约束一致。
#[derive(Debug)]
pub struct FixedWindowLimiter {
    window: Duration,
    limit: u32,
    buckets: Mutex<HashMap<String, (Instant, u32)>>,
}

impl FixedWindowLimiter {
    /// `limit` 为 `0` 表示禁用限流。
    pub fn new(window: Duration, limit: u32) -> Self {
        Self {
            window,
            limit,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 记一次请求；返回 `Some(retry_after_secs)` 表示应拒绝本次请求。
    ///
    /// key 用来源 IP；取不到 IP 时退化为同一个共享桶（宁可一起限，也不要完全不限）。
    pub fn check(&self, key: Option<&str>) -> Option<u64> {
        if self.limit == 0 {
            return None;
        }
        let key = key.unwrap_or("-").to_string();
        let now = Instant::now();
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // 顺手回收过期窗口，避免 key 空间无限增长（每个来源 IP 一个 key）。
        buckets.retain(|_, (started, _)| now.duration_since(*started) < self.window);

        let entry = buckets.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
        if entry.1 > self.limit {
            let elapsed = now.duration_since(entry.0);
            let remaining = self.window.saturating_sub(elapsed);
            return Some(remaining.as_secs().max(1));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_is_enforced_per_key_and_window() {
        let limiter = FixedWindowLimiter::new(Duration::from_secs(10), 3);
        for _ in 0..3 {
            assert_eq!(limiter.check(Some("1.1.1.1")), None, "额度内应放行");
        }
        let retry = limiter.check(Some("1.1.1.1")).expect("超限应被拒绝");
        assert!((1..=10).contains(&retry), "retry_after 应在窗口内: {retry}");

        // 其它来源不受影响（一个被刷不能牵连所有人，包括 AlertManager）
        assert_eq!(limiter.check(Some("2.2.2.2")), None);

        // 取不到 IP 时退化为共享桶：仍然受限，不会变成完全不限
        let shared = FixedWindowLimiter::new(Duration::from_secs(10), 1);
        assert_eq!(shared.check(None), None);
        assert!(shared.check(None).is_some());
    }

    #[test]
    fn zero_limit_disables_throttling() {
        let limiter = FixedWindowLimiter::new(Duration::from_secs(10), 0);
        for _ in 0..1000 {
            assert_eq!(limiter.check(Some("1.1.1.1")), None);
        }
    }

    #[test]
    fn expired_keys_are_reclaimed() {
        let limiter = FixedWindowLimiter::new(Duration::from_millis(1), 1);
        assert_eq!(limiter.check(Some("a")), None);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(limiter.check(Some("b")), None);
        // 过期窗口已被回收，只剩刚写入的那个
        assert_eq!(
            limiter
                .buckets
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .len(),
            1
        );
    }
}
