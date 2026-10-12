//! 启动期引导：确保存在默认卡片模板与默认应用消息流模板。

use alkh_domain::template::{AppFeedCardSpec, CardMessageSpec, CardTemplate, TemplateKind};

use crate::deps::Deps;
use crate::error::AppResult;

/// 幂等：仅在缺失时补齐默认模板。
pub async fn ensure_default_templates(deps: &Deps) -> AppResult<()> {
    let existing = deps.templates.list(None).await?;
    let has_card = existing.iter().any(|t| t.kind == TemplateKind::CardMessage);
    let has_feed = existing.iter().any(|t| t.kind == TemplateKind::AppFeedCard);

    if !has_card {
        let mut t = CardTemplate::new_card_message(
            crate::defaults::DEFAULT_CARD_TEMPLATE_NAME,
            CardMessageSpec::builder_json(crate::defaults::default_card_json()),
        );
        t.is_default = true;
        t.validate()?;
        deps.templates.insert(&t).await?;
        tracing::info!(template = %t.name, "已创建默认卡片消息模板");
    }

    if !has_feed {
        let mut t = CardTemplate::new_app_feed_card(
            crate::defaults::DEFAULT_APP_FEED_TEMPLATE_NAME,
            AppFeedCardSpec::default(),
        );
        t.is_default = true;
        t.validate()?;
        deps.templates.insert(&t).await?;
        tracing::info!(template = %t.name, "已创建默认应用消息流模板");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_card_template_is_valid() {
        let mut t = CardTemplate::new_card_message(
            crate::defaults::DEFAULT_CARD_TEMPLATE_NAME,
            CardMessageSpec::builder_json(crate::defaults::default_card_json()),
        );
        t.validate().expect("默认卡片模板必须合法");
        assert!(!t.detected_variables().is_empty());
    }

    #[test]
    fn default_app_feed_template_is_valid() {
        let mut t = CardTemplate::new_app_feed_card(
            crate::defaults::DEFAULT_APP_FEED_TEMPLATE_NAME,
            AppFeedCardSpec::default(),
        );
        t.validate().expect("默认应用消息流模板必须合法");
        assert!(!t.detected_variables().is_empty());
    }
}
