use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::error::{DomainError, DomainResult};
use crate::ids::{CardTemplateId, ChannelId};
use crate::{Labels, Timestamp};

/// 重复告警（AlertManager 按 repeat_interval 的重复通知）处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepeatPolicy {
    /// 不发送（默认）：仅推进内部状态。
    #[default]
    Suppress,
    /// 发送：重复通知照发。
    Send,
    /// 发送并移除旧告警：发新消息后撤回/删除旧消息。
    SendAndRevokePrevious,
}

/// 恢复告警（全部 resolved）处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedPolicy {
    /// 发送：单独发一条恢复消息（默认）。
    #[default]
    Send,
    /// 发送并删除旧告警：发恢复消息后撤回/删除旧消息。
    SendAndDeletePrevious,
    /// 更新旧告警：把旧消息原地更新为已恢复，不发新消息。
    UpdatePrevious,
}

/// 分组内容变化（新增告警 / 部分恢复，仍处于 firing）处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupUpdatePolicy {
    /// 更新旧告警：原地更新当前活跃消息（默认）。
    #[default]
    UpdatePrevious,
    Send,
    Suppress,
}

/// 标签匹配运算符，语义与 Prometheus label matcher 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchOp {
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "!=")]
    Neq,
    #[serde(rename = "=~")]
    Regex,
    #[serde(rename = "!~")]
    NotRegex,
}

impl MatchOp {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchOp::Eq => "=",
            MatchOp::Neq => "!=",
            MatchOp::Regex => "=~",
            MatchOp::NotRegex => "!~",
        }
    }
}

/// 单条标签匹配器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelMatcher {
    pub name: String,
    pub op: MatchOp,
    pub value: String,
}

impl LabelMatcher {
    pub fn new(name: impl Into<String>, op: MatchOp, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            op,
            value: value.into(),
        }
    }

    pub fn validate(&self) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::validation("label matcher 的 name 不能为空"));
        }
        if matches!(self.op, MatchOp::Regex | MatchOp::NotRegex) {
            regex::Regex::new(&self.value).map_err(|e| {
                DomainError::validation(format!(
                    "label matcher `{} {} {}` 的正则非法: {e}",
                    self.name,
                    self.op.as_str(),
                    self.value
                ))
            })?;
        }
        Ok(())
    }

    /// Prometheus 语义：`!=` / `!~` 在标签**不存在**时也匹配。
    pub fn matches(&self, labels: &Labels) -> bool {
        let actual = labels.get(&self.name);
        match self.op {
            MatchOp::Eq => actual.map(|v| v == &self.value).unwrap_or(false),
            MatchOp::Neq => actual.map(|v| v != &self.value).unwrap_or(true),
            MatchOp::Regex => match actual {
                Some(v) => regex::Regex::new(&self.value)
                    .map(|re| re.is_match(v))
                    .unwrap_or(false),
                None => false,
            },
            MatchOp::NotRegex => match actual {
                Some(v) => regex::Regex::new(&self.value)
                    .map(|re| !re.is_match(v))
                    .unwrap_or(true),
                None => true,
            },
        }
    }
}

/// 渠道的通知策略。与 `Channel` 一对一。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPolicy {
    pub channel_id: ChannelId,
    pub card_template_id: Option<CardTemplateId>,
    /// 「未解决告警置顶」（R4，默认关闭）：分组仍有 firing 告警时，把当前活跃卡片
    /// 置顶（`POST /open-apis/im/v1/pins`）；全部恢复后取消置顶。
    ///
    /// 置顶失败（如群设置只允许群主/管理员 Pin、机器人权限不足）不阻塞投递，
    /// 只记日志与指标，下一次事件会重试。
    pub pin_unresolved: bool,
    pub repeat_policy: RepeatPolicy,
    pub resolved_policy: ResolvedPolicy,
    pub group_update_policy: GroupUpdatePolicy,
    pub label_matchers: Vec<LabelMatcher>,
    /// 聚合窗口（秒），0 表示关闭。
    pub aggregation_window_secs: u32,
    /// 同一渠道两次发送的最小间隔（秒），0 表示关闭。
    pub min_send_interval_secs: u32,
    /// 单条消息最多展示多少条告警，超出折叠为 "+N more"。
    pub max_alerts_per_message: u32,
    /// 静默期截止时间，未到点则抑制发送。
    pub mute_until: Option<Timestamp>,
    pub updated_at: Timestamp,
}

impl NotificationPolicy {
    pub const DEFAULT_MAX_ALERTS: u32 = 20;
    pub const MAX_ALERTS_LIMIT: u32 = 200;
    pub const MAX_WINDOW_SECS: u32 = 3600;

    pub fn default_for(channel_id: ChannelId) -> Self {
        Self {
            channel_id,
            card_template_id: None,
            pin_unresolved: false,
            repeat_policy: RepeatPolicy::Suppress,
            resolved_policy: ResolvedPolicy::Send,
            group_update_policy: GroupUpdatePolicy::UpdatePrevious,
            label_matchers: Vec::new(),
            aggregation_window_secs: 0,
            min_send_interval_secs: 0,
            max_alerts_per_message: Self::DEFAULT_MAX_ALERTS,
            mute_until: None,
            updated_at: crate::now(),
        }
    }

    /// 校验策略。`channel` 目前只用于将来的渠道级交叉校验，保留参数以稳定调用点。
    pub fn validate(&self, _channel: &Channel) -> DomainResult<()> {
        for m in &self.label_matchers {
            m.validate()?;
        }
        if self.aggregation_window_secs > Self::MAX_WINDOW_SECS {
            return Err(DomainError::validation(format!(
                "聚合窗口不能超过 {} 秒",
                Self::MAX_WINDOW_SECS
            )));
        }
        if self.min_send_interval_secs > Self::MAX_WINDOW_SECS {
            return Err(DomainError::validation(format!(
                "最小发送间隔不能超过 {} 秒",
                Self::MAX_WINDOW_SECS
            )));
        }
        if self.max_alerts_per_message == 0 || self.max_alerts_per_message > Self::MAX_ALERTS_LIMIT
        {
            return Err(DomainError::validation(format!(
                "单条消息告警上限必须在 1..={} 之间",
                Self::MAX_ALERTS_LIMIT
            )));
        }
        Ok(())
    }

    /// 静默期是否生效。
    pub fn is_muted(&self, now: Timestamp) -> bool {
        self.mute_until.map(|t| t > now).unwrap_or(false)
    }

    /// 标签匹配：matcher 之间为 AND；空集合表示全部通过。
    pub fn matches_labels(&self, labels: &Labels) -> bool {
        self.label_matchers.iter().all(|m| m.matches(labels))
    }

    pub fn has_filters(&self) -> bool {
        !self.label_matchers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_channel() -> Channel {
        Channel::from_chat("oc_1", "g", None, false)
    }

    #[test]
    fn defaults_are_sane_for_a_group_channel() {
        let p = NotificationPolicy::default_for(group_channel().id.clone());
        assert!(p.validate(&group_channel()).is_ok());
    }

    #[test]
    fn matcher_semantics_follow_prometheus() {
        let mut labels = Labels::new();
        labels.insert("severity".into(), "critical".into());

        assert!(LabelMatcher::new("severity", MatchOp::Eq, "critical").matches(&labels));
        assert!(!LabelMatcher::new("severity", MatchOp::Eq, "warning").matches(&labels));
        assert!(LabelMatcher::new("severity", MatchOp::Neq, "warning").matches(&labels));
        assert!(LabelMatcher::new("severity", MatchOp::Regex, "crit.*").matches(&labels));
        assert!(!LabelMatcher::new("severity", MatchOp::NotRegex, "crit.*").matches(&labels));

        // 不存在的标签：!= 与 !~ 匹配，= 与 =~ 不匹配
        assert!(LabelMatcher::new("absent", MatchOp::Neq, "x").matches(&labels));
        assert!(LabelMatcher::new("absent", MatchOp::NotRegex, "x").matches(&labels));
        assert!(!LabelMatcher::new("absent", MatchOp::Eq, "x").matches(&labels));
        assert!(!LabelMatcher::new("absent", MatchOp::Regex, "x").matches(&labels));
    }

    #[test]
    fn empty_matchers_pass_everything() {
        let p = NotificationPolicy::default_for(ChannelId::new());
        assert!(p.matches_labels(&Labels::new()));
        assert!(!p.has_filters());
    }

    #[test]
    fn invalid_regex_rejected() {
        let m = LabelMatcher::new("a", MatchOp::Regex, "([");
        assert!(m.validate().is_err());
    }

    #[test]
    fn bounds_are_enforced() {
        let c = group_channel();
        let mut p = NotificationPolicy::default_for(c.id.clone());
        p.max_alerts_per_message = 0;
        assert!(p.validate(&c).is_err());
        p.max_alerts_per_message = 20;
        p.aggregation_window_secs = 99999;
        assert!(p.validate(&c).is_err());
    }
}
