-- 卡片内时间的展示时区（IANA 名，默认 UTC）。
--
-- 背景：
--   * AlertManager 推来的时间戳（`startsAt` / `endsAt`）本身是 RFC 3339 UTC，
--     这不是本项目的取舍、也没有可配的开关；
--   * 但卡片里给人看的时间此前固定按 `%Y-%m-%d %H:%M:%S UTC` 渲染，
--     非 UTC 时区的值班人得在脑子里做换算（例：北京同事看到的是 03:04:19 UTC）；
--   * 现把「展示时区」做成后台「设置」项，默认 `UTC`（保持既有行为），
--     例如设为 `Asia/Shanghai` 后，卡片内所有时间变量直接按东八区渲染
--     （`2026-10-16 11:04:19 +08:00`）。
--
-- 渲染口径：只在**展示字符串**上换算；库内存储、HTTP API、AlertManager 侧
-- 一律保持 RFC 3339 UTC，历史卡片不重渲染（下次更新时才用新时区）。
--
-- 纯增量加列（与 0003 / 0004 同款）；历史行取默认值 UTC。

ALTER TABLE app_settings ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
