-- 恢复「未解决告警置顶」能力（原 R4），改用官方的 Pin 消息接口实现。
--
-- 背景：
--   * 0001 的 `notification_policies.pin_unresolved` 依赖应用消息流卡片的
--     `time_sensitive`（只能在消息列表里对**用户**临时置顶）；
--   * 0002 整体移除应用消息流消息形态时，该列一并被删掉，R4 记为「废弃」；
--   * 但飞书对普通卡片消息（`im/v1/messages`）另有官方的 Pin 接口
--     （`POST /open-apis/im/v1/pins`、`DELETE /open-apis/im/v1/pins/:message_id`），
--     机器人对自己发出的群消息即可置顶 / 取消置顶，因此该能力可以按原语义恢复：
--     「分组仍有未解决告警 ⇒ 置顶当前卡片；全部恢复 ⇒ 取消置顶」。
--
-- 两处改动：
--   1. 策略上的开关（opt-in，默认关闭），仅对开启的渠道生效；
--   2. 分组上记录「当前被置顶的消息」，使恢复 / 关闭开关 / 消息被替换时能精确取消，
--      也避免每次事件都重复调用 Pin 接口。
--
-- 说明：0002 已 DROP 掉 notification_policies.pin_unresolved，这里重新 ADD 回来
-- （不是恢复旧列的数据：0002 之前的取值本就是不稳定的历史配置，重来时默认关闭）。

ALTER TABLE notification_policies ADD COLUMN pin_unresolved INTEGER NOT NULL DEFAULT 0;

ALTER TABLE alert_groups ADD COLUMN pinned_message_id TEXT;
