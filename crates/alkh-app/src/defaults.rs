use std::sync::OnceLock;

use serde_json::Value;

/// 内置默认「卡片消息」模板（卡片 JSON 2.0）。
///
/// 变量来自 `alkh_app::render::build_context`，详见 `docs/business-rules.md` §5。
const DEFAULT_CARD_JSON: &str = r#"{
  "schema": "2.0",
  "config": {
    "update_multi": true,
    "summary": { "content": "${status_text} · ${alertname}" }
  },
  "header": {
    "template": "${severity_color}",
    "title": { "tag": "plain_text", "content": "【${status_text}】${alertname}" },
    "subtitle": { "tag": "plain_text", "content": "渠道：${channel_name}" }
  },
  "body": {
    "direction": "vertical",
    "padding": "12px",
    "elements": [
      { "tag": "markdown", "element_id": "summary", "content": "${alerts_summary}" },
      { "tag": "markdown", "element_id": "alerts", "content": "${alerts_list}" },
      { "tag": "hr" },
      {
        "tag": "markdown",
        "element_id": "meta",
        "content": "**接收器**：${receiver}\n**分组**：${group_key}\n**首次出现**：${first_seen_at}\n**最后更新**：${last_seen_at}\n**告警总数**：${alert_count}"
      },
      {
        "tag": "div",
        "element_id": "footer",
        "text": {
          "tag": "plain_text",
          "content": "由 alertmanager-lark-hook 投递",
          "text_size": "notation",
          "text_color": "grey"
        }
      }
    ]
  }
}"#;

static PARSED: OnceLock<Value> = OnceLock::new();

/// 读取内置默认卡片 JSON（同一进程内只解析一次）。
pub fn default_card_json() -> Value {
    PARSED
        .get_or_init(|| {
            serde_json::from_str(DEFAULT_CARD_JSON).expect("内置默认卡片 JSON 必须合法")
        })
        .clone()
}

/// 默认模板名。
pub const DEFAULT_CARD_TEMPLATE_NAME: &str = "默认告警卡片";

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::template::{detect_variables_json, CardMessageSpec};

    #[test]
    fn default_card_is_valid_json_2() {
        let mut spec = CardMessageSpec::builder_json(default_card_json());
        spec.normalize_and_validate()
            .expect("内置默认卡片必须通过校验");
    }

    /// 回归：默认卡片曾用 `note`（JSON 2.0 已废弃的 1.0 组件），
    /// 在飞书侧会直接报错。这里锁死「组件目录零错误零警告」。
    #[test]
    fn default_card_uses_no_deprecated_component() {
        let card = default_card_json();
        let report = alkh_domain::card_catalog::validate_card_json(&card);
        assert!(report.is_valid(), "默认卡片结构错误: {:?}", report.errors);
        assert!(
            report.warnings.is_empty(),
            "默认卡片出现可疑用法: {:?}",
            report.warnings
        );
        assert_eq!(
            alkh_domain::card_catalog::collect_tags(&card),
            vec!["markdown", "hr", "div"]
        );
    }

    #[test]
    fn default_card_only_uses_documented_variables() {
        let vars = detect_variables_json(&default_card_json());
        let known = crate::meta::render_variable_names();
        for v in &vars {
            assert!(known.contains(v), "默认卡片引用了未定义变量: {v}");
        }
        assert!(!vars.is_empty());
    }
}
