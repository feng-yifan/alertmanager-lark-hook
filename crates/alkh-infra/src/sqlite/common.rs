//! SQLite 仓储共用的编解码助手。
//!
//! 约定：
//! - 时间列以 RFC3339 UTC 文本存储
//! - JSON 列（labels / alerts / matchers / actor / outcome / detail）以 TEXT 存储
//! - 枚举列以 serde `snake_case` 字符串存储（`MatchOp` 存 `=` / `!=` / `=~` / `!~`）

use alkh_domain::error::{PortError, PortResult};
use alkh_domain::Timestamp;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

/// sqlx 错误 → 端口错误。
pub fn map_sql(e: sqlx::Error) -> PortError {
    match &e {
        sqlx::Error::RowNotFound => PortError::NotFound("记录不存在".into()),
        sqlx::Error::Database(db) => {
            let msg = db.message().to_string();
            if db.is_unique_violation() {
                PortError::Conflict(msg)
            } else if db.is_foreign_key_violation() {
                PortError::Conflict(format!("外键约束失败: {msg}"))
            } else {
                PortError::Internal(msg)
            }
        }
        other => PortError::Internal(other.to_string()),
    }
}

// --- JSON ---

pub fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|e| {
        tracing::error!(error = %e, "JSON 序列化失败，写入 null");
        "null".to_string()
    })
}

pub fn from_json<T: DeserializeOwned>(s: &str) -> PortResult<T> {
    serde_json::from_str(s).map_err(|e| PortError::Internal(format!("JSON 反序列化失败: {e}")))
}

pub fn from_json_or_default<T: DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

// --- 枚举（以 serde 表示存储为字符串）---

pub fn enum_to_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(e) => {
            tracing::error!(error = %e, "枚举序列化失败");
            String::new()
        }
    }
}

pub fn enum_from_str<T: DeserializeOwned>(s: &str) -> PortResult<T> {
    serde_json::from_value(Value::String(s.to_string()))
        .map_err(|e| PortError::Internal(format!("枚举值 `{s}` 无法解析: {e}")))
}

// --- 时间 ---

pub fn ts(t: Timestamp) -> String {
    t.to_rfc3339()
}

pub fn ts_opt(t: Option<Timestamp>) -> Option<String> {
    t.map(ts)
}

pub fn parse_ts(s: &str) -> PortResult<Timestamp> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| PortError::Internal(format!("时间戳 `{s}` 解析失败: {e}")))
}

pub fn parse_ts_or_now(s: &str) -> Timestamp {
    parse_ts(s).unwrap_or_else(|_| Utc::now())
}

pub fn opt_ts(s: Option<String>) -> Option<Timestamp> {
    s.as_deref().and_then(|v| parse_ts(v).ok())
}

// --- 布尔 ---

pub fn b(v: bool) -> i64 {
    if v {
        1
    } else {
        0
    }
}

pub fn unb(v: i64) -> bool {
    v != 0
}

/// 行读取助手，减少 `try_get` 样板。
pub trait RowExt {
    fn str_col(&self, col: &str) -> PortResult<String>;
    fn opt_str_col(&self, col: &str) -> PortResult<Option<String>>;
    fn i64_col(&self, col: &str) -> PortResult<i64>;
    fn u32_col(&self, col: &str) -> PortResult<u32>;
    fn u64_col(&self, col: &str) -> PortResult<u64>;
    fn bool_col(&self, col: &str) -> PortResult<bool>;
    fn ts_col(&self, col: &str) -> PortResult<Timestamp>;
    fn opt_ts_col(&self, col: &str) -> PortResult<Option<Timestamp>>;
    /// JSON 列 → 类型；空/非法时回退到 `Default`。
    fn json_col<T: DeserializeOwned + Default>(&self, col: &str) -> T;
    /// JSON 列 → `Option<T>`；`NULL` 或非法时为 `None`。
    fn json_opt_col<T: DeserializeOwned>(&self, col: &str) -> Option<T>;
    /// 枚举列。
    fn enum_col<T: DeserializeOwned>(&self, col: &str) -> PortResult<T>;
}

impl RowExt for SqliteRow {
    fn str_col(&self, col: &str) -> PortResult<String> {
        self.try_get::<String, _>(col).map_err(map_sql)
    }

    fn opt_str_col(&self, col: &str) -> PortResult<Option<String>> {
        self.try_get::<Option<String>, _>(col).map_err(map_sql)
    }

    fn i64_col(&self, col: &str) -> PortResult<i64> {
        self.try_get::<i64, _>(col).map_err(map_sql)
    }

    fn u32_col(&self, col: &str) -> PortResult<u32> {
        Ok(self.i64_col(col)?.max(0) as u32)
    }

    fn u64_col(&self, col: &str) -> PortResult<u64> {
        Ok(self.i64_col(col)?.max(0) as u64)
    }

    fn bool_col(&self, col: &str) -> PortResult<bool> {
        Ok(unb(self.i64_col(col)?))
    }

    fn ts_col(&self, col: &str) -> PortResult<Timestamp> {
        Ok(parse_ts_or_now(&self.str_col(col)?))
    }

    fn opt_ts_col(&self, col: &str) -> PortResult<Option<Timestamp>> {
        Ok(opt_ts(self.opt_str_col(col)?))
    }

    fn json_col<T: DeserializeOwned + Default>(&self, col: &str) -> T {
        match self.opt_str_col(col) {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::error!(column = col, error = %e, "JSON 列解析失败，使用默认值");
                T::default()
            }),
            _ => T::default(),
        }
    }

    fn json_opt_col<T: DeserializeOwned>(&self, col: &str) -> Option<T> {
        match self.opt_str_col(col) {
            Ok(Some(s)) if !s.trim().is_empty() => serde_json::from_str(&s).ok(),
            Ok(Some(s)) if s.trim() == "null" => None,
            _ => None,
        }
    }

    fn enum_col<T: DeserializeOwned>(&self, col: &str) -> PortResult<T> {
        enum_from_str(&self.str_col(col)?)
    }
}

/// 构造一个空分页（查询出错或提前返回时使用）。
pub fn empty_page<T>(limit: u32, offset: u32) -> alkh_domain::Page<T> {
    alkh_domain::Page::new(Vec::new(), 0, limit, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn enum_roundtrip() {
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum E {
            A,
            B,
        }
        assert_eq!(enum_to_str(&E::A), "a");
        assert_eq!(enum_from_str::<E>("b").unwrap(), E::B);
        assert!(enum_from_str::<E>("nope").is_err());
    }

    #[test]
    fn json_roundtrip_and_default() {
        let mut m: BTreeMap<String, String> = BTreeMap::new();
        m.insert("k".into(), "v".into());
        let s = to_json(&m);
        let back: BTreeMap<String, String> = from_json(&s).unwrap();
        assert_eq!(back, m);
        let d: BTreeMap<String, String> = from_json_or_default("not json");
        assert!(d.is_empty());
    }

    #[test]
    fn time_helpers() {
        let now = Utc::now();
        let s = ts(now);
        assert!(parse_ts(&s).is_ok());
        assert_eq!(opt_ts(Some(s)), Some(now));
        assert_eq!(opt_ts(None), None);
        assert_eq!(opt_ts(Some("garbage".into())), None);
    }

    #[test]
    fn bool_helpers() {
        assert_eq!(b(true), 1);
        assert!(unb(1));
        assert!(!unb(0));
    }
}
