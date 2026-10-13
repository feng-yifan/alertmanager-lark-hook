//! 启动期引导：确保存在默认卡片模板。

use alkh_domain::template::{CardMessageSpec, CardTemplate};

use crate::deps::Deps;
use crate::error::AppResult;

/// 幂等：仅在没有任何模板时补齐默认卡片模板。
pub async fn ensure_default_templates(deps: &Deps) -> AppResult<()> {
    let existing = deps.templates.list().await?;

    if existing.is_empty() {
        let mut t = CardTemplate::new_card_message(
            crate::defaults::DEFAULT_CARD_TEMPLATE_NAME,
            CardMessageSpec::builder_json(crate::defaults::default_card_json()),
        );
        t.is_default = true;
        t.validate()?;
        deps.templates.insert(&t).await?;
        tracing::info!(template = %t.name, "已创建默认卡片消息模板");
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
}
