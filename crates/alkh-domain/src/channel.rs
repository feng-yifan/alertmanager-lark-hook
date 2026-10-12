use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::ids::ChannelId;
use crate::Timestamp;

/// 渠道形态：群聊 or 单聊用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Group,
    P2pUser,
}

/// 消息接收者 ID 类型，与飞书 `im/v1/messages` 的 `receive_id_type` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveIdType {
    ChatId,
    OpenId,
    UserId,
    Email,
}

impl ReceiveIdType {
    pub fn as_str(self) -> &'static str {
        match self {
            ReceiveIdType::ChatId => "chat_id",
            ReceiveIdType::OpenId => "open_id",
            ReceiveIdType::UserId => "user_id",
            ReceiveIdType::Email => "email",
        }
    }

    pub fn parse(s: &str) -> DomainResult<Self> {
        match s {
            "chat_id" => Ok(Self::ChatId),
            "open_id" => Ok(Self::OpenId),
            "user_id" => Ok(Self::UserId),
            "email" => Ok(Self::Email),
            other => Err(DomainError::validation(format!(
                "未知的 receive_id_type: {other}"
            ))),
        }
    }
}

/// 渠道来源：从飞书同步 or 手工创建。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSource {
    FeishuSync,
    Manual,
}

/// 消息渠道聚合根。一个渠道对应一个可接收消息的飞书会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub receive_id: String,
    pub receive_id_type: ReceiveIdType,
    pub kind: ChannelKind,
    pub avatar_url: Option<String>,
    /// 外部群（跨租户）。
    pub external: bool,
    /// 最近一次发送/探测结果：机器人是否仍可在此渠道发言。
    pub bot_can_send: bool,
    pub enabled: bool,
    pub source: ChannelSource,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Channel {
    pub const MAX_NAME_LEN: usize = 64;

    pub fn new(
        name: impl Into<String>,
        receive_id: impl Into<String>,
        receive_id_type: ReceiveIdType,
        kind: ChannelKind,
        source: ChannelSource,
    ) -> Self {
        let now = crate::now();
        Self {
            id: ChannelId::new(),
            name: name.into(),
            receive_id: receive_id.into(),
            receive_id_type,
            kind,
            avatar_url: None,
            external: false,
            bot_can_send: true,
            enabled: true,
            source,
            created_at: now,
            updated_at: now,
        }
    }

    /// 从飞书群列表条目构造。
    pub fn from_chat(
        receive_id: impl Into<String>,
        name: impl Into<String>,
        avatar_url: Option<String>,
        external: bool,
    ) -> Self {
        let mut c = Self::new(
            name,
            receive_id,
            ReceiveIdType::ChatId,
            ChannelKind::Group,
            ChannelSource::FeishuSync,
        );
        c.avatar_url = avatar_url;
        c.external = external;
        c
    }

    pub fn validate(&self) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::validation("渠道名称不能为空"));
        }
        if self.name.chars().count() > Self::MAX_NAME_LEN {
            return Err(DomainError::validation(format!(
                "渠道名称不能超过 {} 个字符",
                Self::MAX_NAME_LEN
            )));
        }
        if self.receive_id.trim().is_empty() {
            return Err(DomainError::validation("receive_id 不能为空"));
        }
        if self.receive_id_type == ReceiveIdType::ChatId && self.kind != ChannelKind::Group {
            return Err(DomainError::validation(
                "receive_id_type=chat_id 时 kind 必须为 group",
            ));
        }
        if self.receive_id_type != ReceiveIdType::ChatId && self.kind != ChannelKind::P2pUser {
            return Err(DomainError::validation(
                "用户级 receive_id_type（open_id/user_id/email）时 kind 必须为 p2p_user",
            ));
        }
        Ok(())
    }

    /// 该渠道是否可投递（渠道启用且机器人可发言）。
    pub fn can_deliver(&self) -> bool {
        self.enabled && self.bot_can_send
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_channel_validates() {
        let c = Channel::from_chat("oc_1", "运维群", None, false);
        assert!(c.validate().is_ok());
        assert!(c.can_deliver());
    }

    #[test]
    fn mismatched_kind_is_rejected() {
        let mut c = Channel::new(
            "u",
            "ou_1",
            ReceiveIdType::OpenId,
            ChannelKind::Group,
            ChannelSource::Manual,
        );
        assert!(c.validate().is_err());
        c.kind = ChannelKind::P2pUser;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn blank_name_rejected() {
        let c = Channel::from_chat("oc_1", "   ", None, false);
        assert!(c.validate().is_err());
    }
}
