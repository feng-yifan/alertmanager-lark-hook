-- 告警 Pin 拆成两组、并支持多条 Pin。
--
-- 背景：原先「未解决告警 Pin」每组只记一条 pinned_message_id，且全部恢复时无条件取消。
-- 但 `repeat_policy = send` 时每次重复通知都会新发一张卡片，只有一条记录会把历史卡片顶掉；
-- 同时「恢复通知」既可能原地更新、也可能单独发送或发送并删除旧卡，需要更细的开关：
--   1. 未解决告警 Pin：`pin_unresolved` + `unpin_on_resolve`（恢复后是否自动取消，默认开启）；
--   2. 已解决告警 Pin：`pin_resolved`（恢复时是否把恢复卡片也 Pin，默认关闭）。
--
-- 因此分组侧由单值 `pinned_message_id` 升级为 `pinned_messages`（JSON 数组：
-- `[{"message_id":"om_...","kind":"unresolved"|"resolved"}]`），支持同一分组累积多条 Pin。
-- 迁移时把旧值转成一条 `unresolved` 记录，避免丢历史 Pin。

ALTER TABLE notification_policies ADD COLUMN unpin_on_resolve INTEGER NOT NULL DEFAULT 1;
ALTER TABLE notification_policies ADD COLUMN pin_resolved INTEGER NOT NULL DEFAULT 0;

ALTER TABLE alert_groups ADD COLUMN pinned_messages TEXT NOT NULL DEFAULT '[]';

UPDATE alert_groups
SET pinned_messages = '[{"message_id":"' || pinned_message_id || '","kind":"unresolved"}]'
WHERE pinned_message_id IS NOT NULL AND pinned_message_id <> '';

ALTER TABLE alert_groups DROP COLUMN pinned_message_id;
