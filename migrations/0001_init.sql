-- alertmanager-lark-hook 初始 schema
-- 约定：所有时间列为 RFC3339 UTC 文本；JSON 列为序列化后的 TEXT。

CREATE TABLE IF NOT EXISTS channels (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    receive_id       TEXT NOT NULL,
    receive_id_type  TEXT NOT NULL,
    kind             TEXT NOT NULL,
    avatar_url       TEXT,
    external         INTEGER NOT NULL DEFAULT 0,
    bot_can_send     INTEGER NOT NULL DEFAULT 1,
    enabled          INTEGER NOT NULL DEFAULT 1,
    source           TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_channels_receive
    ON channels (receive_id_type, receive_id);
CREATE INDEX IF NOT EXISTS idx_channels_enabled ON channels (enabled);

CREATE TABLE IF NOT EXISTS hooks (
    id              TEXT PRIMARY KEY,
    channel_id      TEXT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    token_hash      TEXT NOT NULL UNIQUE,
    token_prefix    TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_called_at  TEXT,
    call_count      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    rotated_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_hooks_channel ON hooks (channel_id);

CREATE TABLE IF NOT EXISTS card_templates (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    name          TEXT NOT NULL,
    card_message  TEXT,
    app_feed_spec TEXT,
    is_default    INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_templates_kind ON card_templates (kind, is_default DESC);

CREATE TABLE IF NOT EXISTS notification_policies (
    channel_id               TEXT PRIMARY KEY REFERENCES channels (id) ON DELETE CASCADE,
    message_type             TEXT NOT NULL,
    card_template_id         TEXT,
    pin_unresolved           INTEGER NOT NULL DEFAULT 0,
    repeat_policy            TEXT NOT NULL,
    resolved_policy          TEXT NOT NULL,
    group_update_policy      TEXT NOT NULL,
    label_matchers           TEXT NOT NULL DEFAULT '[]',
    aggregation_window_secs  INTEGER NOT NULL DEFAULT 0,
    min_send_interval_secs   INTEGER NOT NULL DEFAULT 0,
    max_alerts_per_message   INTEGER NOT NULL DEFAULT 20,
    mute_until               TEXT,
    updated_at               TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_policies_template ON notification_policies (card_template_id);

CREATE TABLE IF NOT EXISTS alert_groups (
    id                   TEXT PRIMARY KEY,
    hook_id              TEXT NOT NULL,
    channel_id           TEXT NOT NULL,
    group_key            TEXT NOT NULL,
    receiver             TEXT NOT NULL DEFAULT '',
    status               TEXT NOT NULL,
    alertname            TEXT,
    severity             TEXT,
    group_labels         TEXT NOT NULL DEFAULT '{}',
    common_labels        TEXT NOT NULL DEFAULT '{}',
    common_annotations   TEXT NOT NULL DEFAULT '{}',
    alerts               TEXT NOT NULL DEFAULT '[]',
    firing_count         INTEGER NOT NULL DEFAULT 0,
    resolved_count       INTEGER NOT NULL DEFAULT 0,
    truncated_alerts     INTEGER NOT NULL DEFAULT 0,
    event_count          INTEGER NOT NULL DEFAULT 0,
    last_event_hash      TEXT,
    external_url         TEXT,
    first_seen_at        TEXT NOT NULL,
    last_seen_at         TEXT NOT NULL,
    last_notified_at     TEXT,
    resolved_at          TEXT,
    active_delivery_id   TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_alert_groups_natural_key
    ON alert_groups (hook_id, group_key);
CREATE INDEX IF NOT EXISTS idx_alert_groups_status
    ON alert_groups (status, last_seen_at DESC);
CREATE INDEX IF NOT EXISTS idx_alert_groups_channel
    ON alert_groups (channel_id, last_seen_at DESC);

CREATE TABLE IF NOT EXISTS deliveries (
    id                TEXT PRIMARY KEY,
    alert_group_id    TEXT NOT NULL,
    channel_id        TEXT NOT NULL,
    kind              TEXT NOT NULL,
    message_type      TEXT NOT NULL,
    state             TEXT NOT NULL,
    card_template_id  TEXT,
    lark_message_id   TEXT,
    card_entity_id    TEXT,
    app_feed_biz_id   TEXT,
    card_sequence     INTEGER NOT NULL DEFAULT 0,
    rendered_digest   TEXT NOT NULL DEFAULT '',
    attempt           INTEGER NOT NULL DEFAULT 0,
    last_error        TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_deliveries_group
    ON deliveries (alert_group_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_deliveries_state
    ON deliveries (state, updated_at);
CREATE INDEX IF NOT EXISTS idx_deliveries_channel
    ON deliveries (channel_id, created_at DESC);

CREATE TABLE IF NOT EXISTS app_settings (
    id                          INTEGER PRIMARY KEY CHECK (id = 1),
    lark_app_id                 TEXT,
    lark_app_secret_nonce       TEXT,
    lark_app_secret_ciphertext  TEXT,
    lark_base_url               TEXT NOT NULL DEFAULT 'https://open.feishu.cn',
    public_base_url             TEXT,
    default_app_feed_link       TEXT,
    retention_alert_group_days  INTEGER NOT NULL DEFAULT 30,
    retention_delivery_days     INTEGER NOT NULL DEFAULT 30,
    retention_audit_days        INTEGER NOT NULL DEFAULT 14,
    updated_at                  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS admin_users (
    id                    TEXT PRIMARY KEY,
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,
    must_change_password  INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL,
    last_login_at         TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    user_agent  TEXT,
    ip          TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_expiry ON sessions (expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    at       TEXT NOT NULL,
    actor    TEXT NOT NULL,
    action   TEXT NOT NULL,
    target   TEXT,
    outcome  TEXT NOT NULL,
    detail   TEXT NOT NULL DEFAULT 'null'
);
CREATE INDEX IF NOT EXISTS idx_audit_at ON audit_logs (at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_logs (action, at DESC);

CREATE TABLE IF NOT EXISTS event_dedup (
    hook_id     TEXT NOT NULL,
    event_hash  TEXT NOT NULL,
    seen_at     TEXT NOT NULL,
    PRIMARY KEY (hook_id, event_hash)
);
CREATE INDEX IF NOT EXISTS idx_dedup_seen ON event_dedup (seen_at);
