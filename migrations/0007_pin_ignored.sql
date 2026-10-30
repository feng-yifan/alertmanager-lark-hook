-- 「忽略并 unpin」（R4c）：分组级持久标志 + 卡片回调验签配置。
--
-- 告警卡片新增「忽略并 unpin」按钮：点击后撤销该分组当前所有 Pin，并在本次告警
-- 事件周期内（重复/恢复/更新/静默/抑制/重试）不再 Pin；该告警彻底恢复后重开
-- （Resolved→Firing）时自动恢复 Pin 能力。
--   1. alert_groups.pin_ignored：分组级标志，由唯一的 Pin 对账函数 reconcile_pins
--      统一压住（置位后期望 Pin 集合视为空）。
--   2. app_settings 增加卡片回调（card.action.trigger）验签用的 Verification Token
--      （加密存储，永不回显明文）。回调端点 POST /api/v1/lark/card-callback 用它校验来源。
--      注意：本实现采用 Verification Token 校验方式，不启用飞书 Encrypt Key 加密策略
--      （启用后事件 body 会被加密，需 AES 解密；本服务不引入该复杂度）。

ALTER TABLE alert_groups ADD COLUMN pin_ignored INTEGER NOT NULL DEFAULT 0;

ALTER TABLE app_settings ADD COLUMN lark_verification_token_nonce TEXT;
ALTER TABLE app_settings ADD COLUMN lark_verification_token_ciphertext TEXT;
