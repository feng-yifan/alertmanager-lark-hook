use std::fmt;

/// 生成字符串型强类型 ID（UUID v4）。
macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
            pub fn from_string(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn into_string(self) -> String {
                self.0
            }
            /// 仅用于展示的短 ID。
            pub fn short(&self) -> &str {
                &self.0[..self.0.len().min(8)]
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

id_type!(/// 消息渠道（群聊 / 单聊用户）。
ChannelId);
id_type!(/// 渠道下的一个 AlertManager 接入端点。
HookId);
id_type!(/// 卡片模板。
CardTemplateId);
id_type!(/// 以 AlertManager `groupKey` 为自然键的告警分组。
AlertGroupId);
id_type!(/// 一次具体的消息投递。
DeliveryId);
id_type!(/// 管理后台用户。
AdminUserId);
id_type!(/// 管理后台会话。
SessionId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_short_is_safe() {
        let a = ChannelId::new();
        let b = ChannelId::new();
        assert_ne!(a, b);
        assert_eq!(a.short().len(), 8);
        assert_eq!(HookId::from_string("abc").short(), "abc");
    }
}
