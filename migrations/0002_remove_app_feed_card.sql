-- 移除「应用消息流消息」（im/v2/app_feed_card）功能。
--
-- 背景：该形态只能投递给用户级渠道、无法与服务端的「一张卡片装一组告警」模型复用
-- 同一套模板（字段结构完全不同），维护两套渲染/发送/更新路径的收益不足以抵消复杂度。
--
-- 0001 仍会创建这些列（历史不可改，改它会破坏 sqlx 的迁移校验和），
-- 因此这里按顺序：先清理数据，再删列。
--
-- 数据处置：
--   * app_feed_card 模板行删除（其内容无法迁移成卡片 JSON）；
--   * 引用被删模板的渠道策略置空 card_template_id → 运行时回退到默认卡片模板（R12），
--     不会因为悬空引用而报错；
--   * message_type = 'app_feed_card' 的策略改为 card_message（渠道本身不变）；
--   * 历史投递行保留（投递记录是审计数据），只是不再有 app_feed_biz_id。

-- 1. 让引用 app_feed 模板的策略回退到默认模板
UPDATE notification_policies
SET card_template_id = NULL
WHERE card_template_id IN (
    SELECT id FROM card_templates WHERE kind = 'app_feed_card'
);

-- 2. 删除 app_feed 模板行
DELETE FROM card_templates WHERE kind = 'app_feed_card';

-- 3. 策略不再区分消息类型（只剩卡片消息）
UPDATE notification_policies
SET message_type = 'card_message'
WHERE message_type <> 'card_message';

-- 4. 删列。SQLite 的 DROP COLUMN 不允许删除被索引引用的列：
--    card_templates.kind 在 idx_templates_kind 中，必须先删索引。
DROP INDEX IF EXISTS idx_templates_kind;

ALTER TABLE card_templates DROP COLUMN app_feed_spec;
ALTER TABLE card_templates DROP COLUMN kind;

-- 默认模板改为全表唯一（原来按 kind 唯一）
CREATE INDEX IF NOT EXISTS idx_templates_default
    ON card_templates (is_default DESC);

ALTER TABLE notification_policies DROP COLUMN message_type;
ALTER TABLE notification_policies DROP COLUMN pin_unresolved;

ALTER TABLE deliveries DROP COLUMN message_type;
ALTER TABLE deliveries DROP COLUMN app_feed_biz_id;

ALTER TABLE app_settings DROP COLUMN default_app_feed_link;
