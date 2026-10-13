//! 预览 / 测试发送使用的 mock 数据。
//!
//! 三层语义，优先级从低到高：
//! 1. **基线**：内置样例分组（`firing` / `resolved`）或一条历史告警分组；
//! 2. **覆盖**：`variables` 里的逐项覆盖，支持 `label:` / `annotation:` 动态 key；
//! 3. 覆盖项即使不在基线里也会被写入渲染上下文（便于试写新标签）。
//!
//! mock 只影响**渲染**，不会改写被引用的历史分组：测试消息仍然复用渠道的
//! 「测试分组」行（见 `ChannelService::send_test`），避免污染真实告警数据。

use std::collections::BTreeMap;

use alkh_domain::alert::AlertGroup;
use alkh_domain::channel::Channel;
use alkh_domain::ids::{AlertGroupId, HookId};
use alkh_domain::template::RenderContext;
use alkh_domain::Timestamp;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::{AppError, AppResult};

/// 覆盖项数量上限（防止管理端误粘贴巨型 JSON）。
pub const MAX_MOCK_VARIABLES: usize = 200;
/// 单个覆盖值长度上限（字符）。
pub const MAX_MOCK_VALUE_CHARS: usize = 4_000;
/// 单个覆盖键长度上限（字符）。
pub const MAX_MOCK_KEY_CHARS: usize = 128;

/// 预览 / 测试发送的 mock 数据。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct MockSpec {
    /// 用一条历史告警分组作为基线数据源（设置后 `sample` 只作展示用途）。
    pub alert_group_id: Option<AlertGroupId>,
    /// 变量覆盖，优先级最高。
    pub variables: BTreeMap<String, String>,
}

impl MockSpec {
    pub fn is_empty(&self) -> bool {
        self.alert_group_id.is_none() && self.variables.is_empty()
    }

    /// 把覆盖项写入渲染上下文（未知 key 会新增，不做报错）。
    pub fn apply(&self, ctx: &mut RenderContext) {
        for (key, value) in &self.variables {
            ctx.set(key.clone(), value.clone());
        }
    }

    /// 管理端输入校验：只约束规模，不校验 key 是否存在。
    pub fn validate(&self) -> AppResult<()> {
        if self.variables.len() > MAX_MOCK_VARIABLES {
            return Err(AppError::validation(format!(
                "mock 变量最多 {} 个，当前 {} 个",
                MAX_MOCK_VARIABLES,
                self.variables.len()
            )));
        }
        for (key, value) in &self.variables {
            if key.trim().is_empty() {
                return Err(AppError::validation("mock 变量名不能为空"));
            }
            if key.chars().count() > MAX_MOCK_KEY_CHARS {
                return Err(AppError::validation(format!(
                    "mock 变量名 `{}` 超过 {} 字符",
                    truncate(key, 40),
                    MAX_MOCK_KEY_CHARS
                )));
            }
            if value.chars().count() > MAX_MOCK_VALUE_CHARS {
                return Err(AppError::validation(format!(
                    "mock 变量 `{}` 的值超过 {} 字符",
                    truncate(key, 40),
                    MAX_MOCK_VALUE_CHARS
                )));
            }
        }
        Ok(())
    }
}

/// 基线的数据来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MockSource {
    /// 内置样例分组。
    Builtin,
    /// 历史告警分组。
    AlertGroup,
}

/// `GET /card-templates/mock` 的响应：某一次基线渲染出的**全部**变量。
#[derive(Debug, Clone, Serialize)]
pub struct MockBaseline {
    pub source: MockSource,
    /// 内置样例：`firing` / `resolved`（历史分组时保留请求值）。
    pub sample: &'static str,
    /// 历史分组 ID（`source = alert_group` 时非空）。
    pub alert_group_id: Option<AlertGroupId>,
    /// 历史分组摘要，供 UI 回显。
    pub alert_group_label: Option<String>,
    /// 变量名 → 基线值。
    pub variables: BTreeMap<String, String>,
}

/// 解析 mock 的数据源分组：历史分组优先，否则退化为内置样例。
///
/// 历史分组只作为**数据源**：渲染时仍使用传入的 `channel`，
/// 因此 `channel_name` / `channel_id` 反映的是实际投递目标。
pub async fn resolve_mock_group(
    deps: &Deps,
    channel: &Channel,
    hook_id: HookId,
    sample_resolved: bool,
    mock: &MockSpec,
    now: Timestamp,
) -> AppResult<AlertGroup> {
    match &mock.alert_group_id {
        Some(id) => deps
            .groups
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("mock 引用的告警分组不存在")),
        None => Ok(crate::query::sample_alert_group(
            channel,
            hook_id,
            sample_resolved,
            now,
        )),
    }
}

/// 构造 mock 基线：解析数据源分组 → `build_context` → 不做覆盖。
pub async fn mock_baseline(
    deps: &Deps,
    channel: &Channel,
    sample_resolved: bool,
    mock: &MockSpec,
    now: Timestamp,
) -> AppResult<MockBaseline> {
    mock.validate()?;
    let group =
        resolve_mock_group(deps, channel, HookId::new(), sample_resolved, mock, now).await?;
    let policy = deps.policies.get(&channel.id).await?.unwrap_or_else(|| {
        alkh_domain::policy::NotificationPolicy::default_for(channel.id.clone())
    });
    let ctx = crate::render::build_context(&group, channel, &policy, now);

    let (source, label) = match &mock.alert_group_id {
        Some(_) => (MockSource::AlertGroup, Some(group_label(&group))),
        None => (MockSource::Builtin, None),
    };

    Ok(MockBaseline {
        source,
        sample: if sample_resolved {
            "resolved"
        } else {
            "firing"
        },
        alert_group_id: mock.alert_group_id.clone(),
        alert_group_label: label,
        variables: ctx.vars().clone(),
    })
}

fn group_label(group: &AlertGroup) -> String {
    let alertname = group.alertname.clone().unwrap_or_else(|| "-".into());
    format!(
        "{} · {} · {} 条告警",
        alertname,
        group.status.as_str(),
        group.alerts.len()
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_overrides_and_adds_unknown_keys() {
        let mut ctx = RenderContext::new().with("summary", "原值");
        let mock = MockSpec {
            alert_group_id: None,
            variables: [
                ("summary".to_string(), "覆盖值".to_string()),
                ("label:team".to_string(), "sre".to_string()),
            ]
            .into_iter()
            .collect(),
        };
        mock.apply(&mut ctx);
        assert_eq!(ctx.lookup("summary"), Some("覆盖值"));
        assert_eq!(ctx.lookup("label:team"), Some("sre"));
    }

    #[test]
    fn empty_mock_is_detected() {
        assert!(MockSpec::default().is_empty());
        let mock = MockSpec {
            alert_group_id: None,
            variables: [("a".to_string(), String::new())].into_iter().collect(),
        };
        assert!(!mock.is_empty());
    }

    #[test]
    fn validate_rejects_oversized_input() {
        let mut mock = MockSpec::default();
        for i in 0..=MAX_MOCK_VARIABLES {
            mock.variables.insert(format!("k{i}"), "v".into());
        }
        assert!(mock.validate().is_err());

        let mut long = MockSpec::default();
        long.variables
            .insert("summary".into(), "x".repeat(MAX_MOCK_VALUE_CHARS + 1));
        assert!(long.validate().is_err());

        let blank = MockSpec {
            alert_group_id: None,
            variables: [("  ".to_string(), "v".to_string())].into_iter().collect(),
        };
        assert!(blank.validate().is_err());
    }

    #[test]
    fn spec_roundtrips_through_json() {
        let mock = MockSpec {
            alert_group_id: Some(AlertGroupId::new()),
            variables: [("summary".to_string(), "s".to_string())]
                .into_iter()
                .collect(),
        };
        let text = serde_json::to_string(&mock).unwrap();
        let back: MockSpec = serde_json::from_str(&text).unwrap();
        assert_eq!(back, mock);
        // 缺省字段可省略
        let empty: MockSpec = serde_json::from_str("{}").unwrap();
        assert!(empty.is_empty());
    }
}
