//! 飞书卡片 JSON 2.0 组件目录（本地卡片构建工具的唯一契约来源）。
//!
//! 目录承担三件事：
//! 1. 通过 `GET /api/v1/meta` 下发给管理后台，驱动组件面板与属性面板；
//! 2. 保存/校验模板时做**结构校验**（错误）与**可疑用法提示**（警告）；
//! 3. 提供每个组件的「新节点默认 JSON」，保证前端创建出来的节点就是 2.0 合法结构。
//!
//! 依据：`docs/research/feishu-api.md` §8（卡片 JSON 2.0 结构）与官方组件文档。
//! 其中 `note` / `action` 在 2.0 已废弃（官方「卡片 JSON 2.0 版本更新说明」明确
//! 「废弃备注组件 & 交互模块」），且 2.0 **传入不支持的属性会直接报错**——所以这里既要
//! 拦住废弃组件，也要提示未在目录中的属性。

use serde::Serialize;
use serde_json::Value;

/// 单张卡片最多 200 个元素/组件（官方硬限制）。
pub const MAX_CARD_ELEMENTS: usize = 200;

/// `element_id`：字母开头，仅字母/数字/下划线，≤20 字符。
pub const ELEMENT_ID_MAX_CHARS: usize = 20;

/// 属性类型：决定前端渲染哪种输入控件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardPropType {
    /// 单行文本。
    Text,
    /// 多行文本（Markdown 组件内容）。
    Markdown,
    /// 数字。
    Number,
    /// 布尔开关。
    Bool,
    /// 枚举下拉。
    Enum,
    /// 尺寸值（如 `"12px"` / `"4px 0"`）。
    Px,
    /// URL。
    Url,
}

/// 属性的读取/写入方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardPropAccessor {
    /// 按 `path`（点号路径）直接读写。
    Field,
    /// 按钮跳转：维护 `behaviors[{type:"open_url",default_url}]`。
    ButtonUrl,
}

/// 一个可编辑属性的元数据。
#[derive(Debug, Clone, Serialize)]
pub struct CardProp {
    /// UI/请求里的属性名。
    pub name: &'static str,
    pub label: &'static str,
    #[serde(rename = "type")]
    pub ty: CardPropType,
    /// JSON 定位路径（点号分隔）；缺省等于 `name`。
    pub path: &'static str,
    pub accessor: CardPropAccessor,
    /// `Enum` 的候选值。
    pub values: &'static [&'static str],
    /// 文本长度上限（字符）。
    pub max_chars: Option<usize>,
    pub placeholder: &'static str,
    pub hint: &'static str,
}

/// 一个飞书卡片组件的元数据。
#[derive(Debug, Clone, Serialize)]
pub struct CardComponent {
    pub tag: &'static str,
    pub label: &'static str,
    /// 分组，供面板分区：`container` | `display` | `interactive` | `element`。
    pub group: &'static str,
    pub description: &'static str,
    /// 子节点所在字段（`elements` / `columns`）；叶组件为 `None`。
    pub children_key: Option<&'static str>,
    /// 允许的子组件 tag；空数组表示「任意非容器专属组件」。
    pub children: &'static [&'static str],
    /// 允许的父组件 tag；空数组表示「任意位置」。
    pub child_of: &'static [&'static str],
    /// 是否出现在组件面板里（`column`、`plain_text` 由父组件自动创建）。
    pub palette: bool,
    /// 2.0 已废弃：非空表示废弃原因（校验时报错）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<&'static str>,
    /// 新建节点时使用的默认 JSON（保证结构合法）。
    pub default_json: &'static str,
    pub props: &'static [CardProp],
}

const fn prop(
    name: &'static str,
    label: &'static str,
    ty: CardPropType,
    values: &'static [&'static str],
    hint: &'static str,
) -> CardProp {
    CardProp {
        name,
        label,
        ty,
        path: name,
        accessor: CardPropAccessor::Field,
        values,
        max_chars: None,
        placeholder: "",
        hint,
    }
}

// --- 通用枚举 ---------------------------------------------------------------

const TEXT_ALIGN: &[&str] = &["left", "center", "right"];
const TEXT_SIZE: &[&str] = &[
    "heading-0",
    "heading-1",
    "heading-2",
    "heading-3",
    "heading-4",
    "heading",
    "normal",
    "notation",
    "large",
    "medium",
    "small",
];
const SPACING: &[&str] = &["small", "medium", "large", "extra_large"];
const BUTTON_TYPE: &[&str] = &[
    "default",
    "primary",
    "danger",
    "text",
    "primary_text",
    "danger_text",
    "primary_filled",
    "danger_filled",
    "laser",
];
const BUTTON_SIZE: &[&str] = &["tiny", "small", "medium", "large"];
const BUTTON_WIDTH: &[&str] = &["default", "fill"];
const FLEX_MODE: &[&str] = &["none", "stretch", "flow", "bisect", "trisect"];
const COLUMN_WIDTH: &[&str] = &["auto", "weighted"];
const VERTICAL_ALIGN: &[&str] = &["top", "center", "bottom"];
const BACKGROUND_STYLE: &[&str] = &["default", "grey"];
const DIRECTION: &[&str] = &["vertical", "horizontal"];

const MARKDOWN_PROPS: &[CardProp] = &[
    CardProp {
        name: "content",
        label: "内容（Markdown）",
        ty: CardPropType::Markdown,
        path: "content",
        accessor: CardPropAccessor::Field,
        values: &[],
        max_chars: None,
        placeholder: "${alerts_list}",
        hint: "支持 ${变量}；服务端已把告警列表拍平成 markdown，直接引用 ${alerts_list} 即可",
    },
    prop("text_size", "字号", CardPropType::Enum, TEXT_SIZE, ""),
    prop("text_align", "对齐", CardPropType::Enum, TEXT_ALIGN, ""),
    prop(
        "margin",
        "外边距",
        CardPropType::Px,
        &[],
        "如 4px / 4px 0 / 4px 0 4px 0",
    ),
];

const PLAIN_TEXT_PROPS: &[CardProp] = &[
    CardProp {
        name: "content",
        label: "内容",
        ty: CardPropType::Text,
        path: "text.content",
        accessor: CardPropAccessor::Field,
        values: &[],
        max_chars: None,
        placeholder: "${status_text}",
        hint: "支持 ${变量}",
    },
    prop(
        "text_size",
        "字号",
        CardPropType::Enum,
        TEXT_SIZE,
        "notation = 辅助信息（12px），做脚注最合适",
    ),
    prop("text_align", "对齐", CardPropType::Enum, TEXT_ALIGN, ""),
    prop(
        "text_color",
        "颜色",
        CardPropType::Enum,
        &["default", "grey", "blue", "green", "orange", "red"],
        "",
    ),
    prop("margin", "外边距", CardPropType::Px, &[], ""),
];

const BUTTON_PROPS: &[CardProp] = &[
    CardProp {
        name: "text",
        label: "按钮文案",
        ty: CardPropType::Text,
        path: "text.content",
        accessor: CardPropAccessor::Field,
        values: &[],
        max_chars: Some(100),
        placeholder: "查看详情",
        hint: "最多 100 字符",
    },
    prop("type", "样式", CardPropType::Enum, BUTTON_TYPE, ""),
    prop("size", "尺寸", CardPropType::Enum, BUTTON_SIZE, ""),
    prop("width", "宽度", CardPropType::Enum, BUTTON_WIDTH, ""),
    CardProp {
        name: "url",
        label: "跳转链接",
        ty: CardPropType::Url,
        path: "behaviors",
        accessor: CardPropAccessor::ButtonUrl,
        values: &[],
        max_chars: None,
        placeholder: "${external_url}",
        hint: "写入 behaviors[0].{type:open_url,default_url}",
    },
    prop("disabled", "禁用", CardPropType::Bool, &[], ""),
    prop("margin", "外边距", CardPropType::Px, &[], ""),
];

const COLUMN_SET_PROPS: &[CardProp] = &[
    prop(
        "flex_mode",
        "窄屏自适应",
        CardPropType::Enum,
        FLEX_MODE,
        "bisect = 两列等分；flow = 自动换行",
    ),
    prop(
        "horizontal_spacing",
        "列间距",
        CardPropType::Enum,
        SPACING,
        "",
    ),
    prop(
        "horizontal_align",
        "水平对齐",
        CardPropType::Enum,
        TEXT_ALIGN,
        "",
    ),
    prop(
        "background_style",
        "背景色",
        CardPropType::Enum,
        BACKGROUND_STYLE,
        "",
    ),
    prop("margin", "外边距", CardPropType::Px, &[], ""),
];

const COLUMN_PROPS: &[CardProp] = &[
    prop(
        "width",
        "列宽",
        CardPropType::Enum,
        COLUMN_WIDTH,
        "flex_mode=none 时生效",
    ),
    prop(
        "weight",
        "权重",
        CardPropType::Number,
        &[],
        "width=weighted 时生效，1–5",
    ),
    prop(
        "vertical_align",
        "垂直对齐",
        CardPropType::Enum,
        VERTICAL_ALIGN,
        "",
    ),
    prop(
        "vertical_spacing",
        "纵向间距",
        CardPropType::Enum,
        SPACING,
        "",
    ),
    prop(
        "background_style",
        "背景色",
        CardPropType::Enum,
        BACKGROUND_STYLE,
        "",
    ),
    prop("padding", "内边距", CardPropType::Px, &[], ""),
    prop("direction", "排列方向", CardPropType::Enum, DIRECTION, ""),
    prop("margin", "外边距", CardPropType::Px, &[], ""),
];

const COLLAPSIBLE_PROPS: &[CardProp] = &[
    CardProp {
        name: "title",
        label: "面板标题",
        ty: CardPropType::Text,
        path: "header.title.content",
        accessor: CardPropAccessor::Field,
        values: &[],
        max_chars: None,
        placeholder: "详情",
        hint: "支持 ${变量}",
    },
    prop("expanded", "默认展开", CardPropType::Bool, &[], ""),
    prop("margin", "外边距", CardPropType::Px, &[], ""),
];

const HR_PROPS: &[CardProp] = &[prop("margin", "外边距", CardPropType::Px, &[], "")];

const PLAIN_TEXT_ELEMENT_PROPS: &[CardProp] = &[
    CardProp {
        name: "content",
        label: "内容",
        ty: CardPropType::Text,
        path: "content",
        accessor: CardPropAccessor::Field,
        values: &[],
        max_chars: None,
        placeholder: "文本",
        hint: "支持 ${变量}",
    },
    prop("text_size", "字号", CardPropType::Enum, TEXT_SIZE, ""),
    prop("text_align", "对齐", CardPropType::Enum, TEXT_ALIGN, ""),
    prop(
        "text_color",
        "颜色",
        CardPropType::Enum,
        &["default", "grey", "blue", "green", "orange", "red"],
        "",
    ),
];

/// 全部组件（顺序即面板顺序）。
pub fn card_components() -> &'static [CardComponent] {
    &[
        CardComponent {
            tag: "markdown",
            label: "富文本",
            group: "display",
            description: "Markdown 文本；告警列表就用它（${alerts_list}）",
            children_key: None,
            children: &[],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"markdown","content":""}"#,
            props: MARKDOWN_PROPS,
        },
        CardComponent {
            tag: "div",
            label: "普通文本",
            group: "display",
            description: "单段文本；用 notation + grey 可替代已废弃的 note",
            children_key: None,
            children: &[],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"div","text":{"tag":"plain_text","content":""}}"#,
            props: PLAIN_TEXT_PROPS,
        },
        CardComponent {
            tag: "hr",
            label: "分割线",
            group: "display",
            description: "一条水平分割线",
            children_key: None,
            children: &[],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"hr"}"#,
            props: HR_PROPS,
        },
        CardComponent {
            tag: "button",
            label: "按钮",
            group: "interactive",
            description: "跳转按钮，常用于「查看详情」",
            children_key: None,
            children: &[],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"button","text":{"tag":"plain_text","content":"查看详情"},"type":"default","size":"medium"}"#,
            props: BUTTON_PROPS,
        },
        CardComponent {
            tag: "column_set",
            label: "分栏",
            group: "container",
            description: "横向排布多列（每列内可放任意组件）",
            children_key: Some("columns"),
            children: &["column"],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"column_set","flex_mode":"none","columns":[{"tag":"column","width":"weighted","weight":1,"elements":[]},{"tag":"column","width":"weighted","weight":1,"elements":[]}]}"#,
            props: COLUMN_SET_PROPS,
        },
        CardComponent {
            tag: "column",
            label: "列",
            group: "container",
            description: "分栏中的一列；由「分栏」自动创建",
            children_key: Some("elements"),
            children: &[],
            child_of: &["column_set"],
            palette: false,
            deprecated: None,
            default_json: r#"{"tag":"column","width":"weighted","weight":1,"elements":[]}"#,
            props: COLUMN_PROPS,
        },
        CardComponent {
            tag: "collapsible_panel",
            label: "折叠面板",
            group: "container",
            description: "折叠次要信息（注意：搭建工具暂不支持，JSON 可用）",
            children_key: Some("elements"),
            children: &[],
            child_of: &[],
            palette: true,
            deprecated: None,
            default_json: r#"{"tag":"collapsible_panel","expanded":true,"header":{"title":{"tag":"plain_text","content":"详情"}},"elements":[]}"#,
            props: COLLAPSIBLE_PROPS,
        },
        CardComponent {
            tag: "plain_text",
            label: "纯文本元素",
            group: "element",
            description: "文本元素；一般作为 div/button 的内嵌文本",
            children_key: None,
            children: &[],
            child_of: &[],
            palette: false,
            deprecated: None,
            default_json: r#"{"tag":"plain_text","content":"文本"}"#,
            props: PLAIN_TEXT_ELEMENT_PROPS,
        },
        // --- 2.0 已废弃：保留目录项只为给出**可操作的**报错 ---
        CardComponent {
            tag: "note",
            label: "备注（已废弃）",
            group: "deprecated",
            description: "JSON 2.0 不再支持",
            children_key: Some("elements"),
            children: &[],
            child_of: &[],
            palette: false,
            deprecated: Some(
                "JSON 2.0 已废弃 note：请改用「普通文本」组件并把字号设为 notation、颜色设为 grey",
            ),
            default_json: r#"{"tag":"note"}"#,
            props: &[],
        },
        CardComponent {
            tag: "action",
            label: "交互模块（已废弃）",
            group: "deprecated",
            description: "JSON 2.0 不再支持",
            children_key: Some("elements"),
            children: &[],
            child_of: &[],
            palette: false,
            deprecated: Some(
                "JSON 2.0 已废弃 action：请把按钮直接放进 elements，并用容器间距控制排版",
            ),
            default_json: r#"{"tag":"action"}"#,
            props: &[],
        },
    ]
}

/// 面向告警场景的组件预设（「积木」）。
///
/// 放在服务端而不是前端，是为了让「积木只使用真实存在的变量」这件事**可以被测试**
/// （见 `alkh-app::meta` 的 `alert_presets_use_only_builtin_variables`），
/// 也避免前端另抄一份会发生漂移的模板。
#[derive(Debug, Clone, Serialize)]
pub struct CardPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// 插入画布的节点数组（JSON 文本；可为 1 个或多个节点）。
    pub nodes_json: &'static str,
}

/// 告警积木：覆盖「一张告警卡需要什么」的全部高频块。
pub fn alert_presets() -> &'static [CardPreset] {
    &[
        CardPreset {
            id: "summary",
            label: "汇总行",
            description: "🔥 3 条未解决 · ✅ 1 条已恢复",
            nodes_json: r#"[{"tag":"markdown","element_id":"summary","text_size":"normal","content":"${alerts_summary}"}]"#,
        },
        CardPreset {
            id: "alerts_list",
            label: "告警列表",
            description: "服务端已拍平好的多行列表（${alerts_list}）",
            nodes_json: r#"[{"tag":"markdown","element_id":"alerts","content":"${alerts_list}"}]"#,
        },
        CardPreset {
            id: "alerts_table",
            label: "告警表格",
            description: "Markdown 表格形式（${alerts_table}）",
            nodes_json: r#"[{"tag":"markdown","element_id":"alerts_table","content":"${alerts_table}"}]"#,
        },
        CardPreset {
            id: "meta",
            label: "元信息块",
            description: "接收器 / 分组 / 时间 / 数量",
            nodes_json: r#"[{"tag":"markdown","element_id":"meta","content":"**接收器**：${receiver}\n**分组**：${group_key}\n**首次出现**：${first_seen_at}\n**最后更新**：${last_seen_at}\n**告警总数**：${alert_count}"}]"#,
        },
        CardPreset {
            id: "severity_row",
            label: "级别 + 状态",
            description: "严重级别与恢复状态两列",
            nodes_json: r#"[{"tag":"column_set","element_id":"severity_row","flex_mode":"bisect","columns":[{"tag":"column","width":"weighted","weight":1,"elements":[{"tag":"div","text":{"tag":"plain_text","content":"级别：${severity_text}","text_size":"normal"}}]},{"tag":"column","width":"weighted","weight":1,"elements":[{"tag":"div","text":{"tag":"plain_text","content":"状态：${status_text}","text_size":"normal","text_align":"right"}}]}]}]"#,
        },
        CardPreset {
            id: "counters",
            label: "数量统计",
            description: "未解决 / 已恢复 两列数字",
            nodes_json: r#"[{"tag":"column_set","element_id":"counters","flex_mode":"bisect","columns":[{"tag":"column","width":"weighted","weight":1,"background_style":"grey","padding":"8px","elements":[{"tag":"div","text":{"tag":"plain_text","content":"${firing_count}","text_size":"heading","text_align":"center"}},{"tag":"div","text":{"tag":"plain_text","content":"未解决","text_size":"notation","text_color":"grey","text_align":"center"}}]},{"tag":"column","width":"weighted","weight":1,"background_style":"grey","padding":"8px","elements":[{"tag":"div","text":{"tag":"plain_text","content":"${resolved_count}","text_size":"heading","text_align":"center"}},{"tag":"div","text":{"tag":"plain_text","content":"已恢复","text_size":"notation","text_color":"grey","text_align":"center"}}]}]}]"#,
        },
        CardPreset {
            id: "detail_button",
            label: "详情按钮",
            description: "跳转 AlertManager（${external_url}）",
            nodes_json: r#"[{"tag":"button","element_id":"detail_btn","text":{"tag":"plain_text","content":"查看详情"},"type":"primary","size":"medium","behaviors":[{"type":"open_url","default_url":"${external_url}","pc_url":"","ios_url":"","android_url":""}]}]"#,
        },
        CardPreset {
            id: "collapsible_detail",
            label: "折叠详情",
            description: "默认收起，点开看完整列表",
            nodes_json: r#"[{"tag":"collapsible_panel","element_id":"detail_panel","expanded":false,"header":{"title":{"tag":"plain_text","content":"详情"}},"elements":[{"tag":"markdown","content":"${alerts_list}"}]}]"#,
        },
        CardPreset {
            id: "footer",
            label: "脚注",
            description: "notation + grey 的灰色小字（替代已废弃的 note）",
            nodes_json: r#"[{"tag":"div","element_id":"footer","text":{"tag":"plain_text","content":"由 alertmanager-lark-hook 投递","text_size":"notation","text_color":"grey"}}]"#,
        },
        CardPreset {
            id: "divider",
            label: "分割线",
            description: "分隔内容区块",
            nodes_json: r#"[{"tag":"hr"}]"#,
        },
    ]
}

/// 按 tag 查目录。
pub fn find_component(tag: &str) -> Option<&'static CardComponent> {
    card_components().iter().find(|c| c.tag == tag)
}

/// 卡片 JSON 的结构校验结果。
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct CardValidation {
    /// 阻断保存：飞书必报错或必然渲染异常。
    pub errors: Vec<String>,
    /// 提示：未知组件/属性、可疑嵌套等，不阻断保存。
    pub warnings: Vec<String>,
}

impl CardValidation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 校验整张卡片 JSON 的 2.0 结构。
pub fn validate_card_json(card: &Value) -> CardValidation {
    let mut out = CardValidation::default();
    let Some(obj) = card.as_object() else {
        out.errors.push("卡片必须是 JSON 对象".into());
        return out;
    };

    // 1.0 结构遗留字段
    if obj.contains_key("elements") {
        out.errors
            .push("顶层出现 elements：JSON 2.0 的组件必须放在 body.elements 下".into());
    }
    if obj.contains_key("i18n_elements") {
        out.errors
            .push("JSON 2.0 不支持 i18n_elements，请改用组件级 i18n_* 字段".into());
    }
    if obj.contains_key("fallback") {
        out.warnings
            .push("JSON 2.0 暂不支持 fallback（会自动忽略）".into());
    }

    // body
    match obj.get("body") {
        None => out
            .warnings
            .push("卡片没有 body：除标题外不会有任何内容".into()),
        Some(body) => match body.as_object() {
            None => out.errors.push("body 必须是 JSON 对象".into()),
            Some(body) => match body.get("elements") {
                None => out.errors.push("body.elements 缺失".into()),
                Some(Value::Array(items)) => {
                    let mut ids: Vec<(String, String)> = Vec::new();
                    for (index, item) in items.iter().enumerate() {
                        walk(
                            item,
                            None,
                            &format!("body.elements[{index}]"),
                            &mut out,
                            &mut ids,
                        );
                    }
                    report_duplicate_ids(&ids, &mut out);
                }
                Some(_) => out.errors.push("body.elements 必须是数组".into()),
            },
        },
    }

    // header.template 枚举（渲染后的值不检查）
    if let Some(template) = obj
        .get("header")
        .and_then(|h| h.get("template"))
        .and_then(|t| t.as_str())
    {
        if !template.contains("${") && !HEADER_TEMPLATES.contains(&template) {
            out.warnings.push(format!(
                "header.template = `{template}` 不在飞书枚举内，客户端会按默认样式展示"
            ));
        }
    }

    out
}

const HEADER_TEMPLATES: &[&str] = &[
    "blue",
    "wathet",
    "turquoise",
    "green",
    "yellow",
    "orange",
    "red",
    "carmine",
    "violet",
    "purple",
    "indigo",
    "grey",
    "default",
];

fn walk(
    node: &Value,
    parent_tag: Option<&str>,
    path: &str,
    out: &mut CardValidation,
    ids: &mut Vec<(String, String)>,
) {
    let Some(obj) = node.as_object() else {
        out.errors.push(format!("{path} 不是 JSON 对象"));
        return;
    };
    let Some(tag) = obj.get("tag").and_then(|t| t.as_str()) else {
        out.errors.push(format!("{path} 缺少 tag"));
        return;
    };

    let component = find_component(tag);
    if let Some(component) = component {
        if let Some(reason) = component.deprecated {
            out.errors.push(format!("{path}（tag={tag}）：{reason}"));
        }
        if !component.child_of.is_empty()
            && !parent_tag.is_some_and(|p| component.child_of.contains(&p))
        {
            out.errors.push(format!(
                "{path}：{} 只能放在 {} 内",
                tag,
                component.child_of.join(" / ")
            ));
        }
        if let Some(key) = component.children_key {
            match obj.get(key) {
                Some(Value::Array(_)) => {}
                Some(_) => out.errors.push(format!("{path}.{key} 必须是数组")),
                None if !component.children.is_empty() => out.errors.push(format!(
                    "{path} 缺少 {key}（{tag} 至少需要一个 {}）",
                    component.children.join("/")
                )),
                None => {}
            }
        }

        // 未在目录中的属性：2.0 会直接报错，给提示而不是阻断。
        let known: Vec<&str> = component
            .props
            .iter()
            .map(|p| p.path.split('.').next().unwrap_or(p.path))
            .collect();
        for key in obj.keys() {
            if key == "tag" || key == "element_id" {
                continue;
            }
            if Some(key.as_str()) == component.children_key {
                continue;
            }
            if !known.contains(&key.as_str()) && component.deprecated.is_none() {
                out.warnings.push(format!(
                    "{path}：组件 {tag} 上的属性 `{key}` 不在目录中（JSON 2.0 会对不支持的属性报错）"
                ));
            }
        }
    } else {
        out.warnings.push(format!(
            "{path}：未知组件 tag=`{tag}`（本工具不支持可视化编辑，保存后仍会原样发送）"
        ));
    }

    if let Some(id) = obj.get("element_id").and_then(|v| v.as_str()) {
        if !valid_element_id(id) {
            out.errors.push(format!(
                "{path}：element_id `{id}` 非法（字母开头，仅字母/数字/下划线，≤{ELEMENT_ID_MAX_CHARS} 字符）"
            ));
        }
        ids.push((id.to_string(), path.to_string()));
    }

    // 递归子节点
    if let Some(key) = component.and_then(|c| c.children_key) {
        if let Some(children) = obj.get(key).and_then(|v| v.as_array()) {
            for (index, child) in children.iter().enumerate() {
                let child_path = format!("{path}.{key}[{index}]");
                if let Some(allowed) = component.map(|c| c.children) {
                    if !allowed.is_empty() {
                        let child_tag = child.get("tag").and_then(|t| t.as_str()).unwrap_or("");
                        if !allowed.contains(&child_tag) {
                            out.errors.push(format!(
                                "{child_path}：{tag}.{key} 只允许 {}",
                                allowed.join(" / ")
                            ));
                        }
                    }
                }
                walk(child, Some(tag), &child_path, out, ids);
            }
        }
    }
}

fn report_duplicate_ids(ids: &[(String, String)], out: &mut CardValidation) {
    for (index, (id, path)) in ids.iter().enumerate() {
        if let Some((_, first)) = ids[..index].iter().find(|(other, _)| other == id) {
            out.errors
                .push(format!("element_id `{id}` 重复（{first} 与 {path}）"));
        }
    }
}

/// `element_id`：字母开头，仅字母/数字/下划线，≤20 字符。
pub fn valid_element_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    id.chars().count() <= ELEMENT_ID_MAX_CHARS
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 统计卡片中的元素数量（含嵌套）。
pub fn count_elements(card: &Value) -> usize {
    fn count(node: &Value) -> usize {
        let Some(obj) = node.as_object() else {
            return 0;
        };
        let tag = obj.get("tag").and_then(|t| t.as_str());
        let key = tag.and_then(find_component).and_then(|c| c.children_key);
        match key.and_then(|k| obj.get(k)).and_then(|v| v.as_array()) {
            Some(children) => 1 + children.iter().map(count).sum::<usize>(),
            None => 1,
        }
    }
    card.get("body")
        .and_then(|b| b.get("elements"))
        .and_then(|e| e.as_array())
        .map(|items| items.iter().map(count).sum())
        .unwrap_or(0)
}

/// 在卡片里查找所有出现的 tag（去重、稳定顺序），供 UI 提示。
pub fn collect_tags(card: &Value) -> Vec<String> {
    fn collect(node: &Value, out: &mut Vec<String>) {
        let Some(obj) = node.as_object() else { return };
        if let Some(tag) = obj.get("tag").and_then(|t| t.as_str()) {
            if !out.iter().any(|t| t == tag) {
                out.push(tag.to_string());
            }
            if let Some(key) = find_component(tag).and_then(|c| c.children_key) {
                if let Some(children) = obj.get(key).and_then(|v| v.as_array()) {
                    for child in children {
                        collect(child, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Some(items) = card
        .get("body")
        .and_then(|b| b.get("elements"))
        .and_then(|e| e.as_array())
    {
        for item in items {
            collect(item, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_tags_are_unique() {
        let mut tags: Vec<&str> = card_components().iter().map(|c| c.tag).collect();
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total, "组件 tag 必须唯一");
    }

    #[test]
    fn default_json_of_every_palette_component_is_valid() {
        for component in card_components() {
            if component.deprecated.is_some() || !component.palette {
                continue;
            }
            let node: Value = serde_json::from_str(component.default_json)
                .unwrap_or_else(|e| panic!("{} 的 default_json 非法: {e}", component.tag));
            let card =
                json!({"schema":"2.0","config":{"update_multi":true},"body":{"elements":[node]}});
            let report = validate_card_json(&card);
            assert!(
                report.is_valid(),
                "{} 的默认节点未通过校验: {:?}",
                component.tag,
                report.errors
            );
            assert!(
                report.warnings.is_empty(),
                "{} 的默认节点产生了警告: {:?}",
                component.tag,
                report.warnings
            );
        }
    }

    #[test]
    fn column_default_json_is_valid_inside_column_set() {
        let column: Value = serde_json::from_str(
            card_components()
                .iter()
                .find(|c| c.tag == "column")
                .unwrap()
                .default_json,
        )
        .unwrap();
        let card = json!({"schema":"2.0","body":{"elements":[
            {"tag":"column_set","flex_mode":"bisect","columns":[column, column]}
        ]}});
        let report = validate_card_json(&card);
        assert!(report.is_valid(), "{:?}", report.errors);
    }

    #[test]
    fn alert_presets_are_valid_cards() {
        for preset in alert_presets() {
            let nodes: Value = serde_json::from_str(preset.nodes_json)
                .unwrap_or_else(|e| panic!("积木「{}」不是合法 JSON: {e}", preset.label));
            let card =
                json!({"schema":"2.0","config":{"update_multi":true},"body":{"elements":nodes}});
            let report = validate_card_json(&card);
            assert!(
                report.is_valid(),
                "积木「{}」结构不合法: {:?}",
                preset.label,
                report.errors
            );
            assert!(
                report.warnings.is_empty(),
                "积木「{}」产生警告: {:?}",
                preset.label,
                report.warnings
            );
        }
    }

    #[test]
    fn preset_ids_are_unique() {
        let mut ids: Vec<&str> = alert_presets().iter().map(|p| p.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "积木 id 必须唯一");
    }

    #[test]
    fn deprecated_note_and_action_are_errors() {
        let card = json!({
            "schema": "2.0",
            "body": {"elements": [{"tag": "note", "elements": [{"tag": "plain_text", "content": "x"}]}]}
        });
        let report = validate_card_json(&card);
        assert!(!report.is_valid());
        assert!(report.errors[0].contains("note"), "{:?}", report.errors);
    }

    #[test]
    fn schema_one_shapes_are_rejected() {
        let report = validate_card_json(&json!({"schema": "2.0", "elements": []}));
        assert!(!report.is_valid());
        assert!(report.errors.iter().any(|e| e.contains("body.elements")));

        let report = validate_card_json(&json!({"schema": "2.0", "body": {}}));
        assert!(!report.is_valid());

        let report = validate_card_json(&json!({"schema": "2.0", "body": {"elements": {}}}));
        assert!(!report.is_valid());
    }

    #[test]
    fn column_must_live_in_column_set() {
        let card = json!({"schema":"2.0","body":{"elements":[{"tag":"column","elements":[]}]}});
        let report = validate_card_json(&card);
        assert!(!report.is_valid());
        assert!(
            report.errors[0].contains("column_set"),
            "{:?}",
            report.errors
        );
    }

    #[test]
    fn column_set_requires_columns_array() {
        let card = json!({"schema":"2.0","body":{"elements":[{"tag":"column_set"}]}});
        let report = validate_card_json(&card);
        assert!(!report.is_valid());

        let card = json!({"schema":"2.0","body":{"elements":[
            {"tag":"column_set","columns":[{"tag":"column","elements":[]}]}
        ]}});
        assert!(validate_card_json(&card).is_valid());
    }

    #[test]
    fn unknown_tag_is_a_warning_not_an_error() {
        let card = json!({"schema":"2.0","body":{"elements":[{"tag":"chart","chart_spec":{}}]}});
        let report = validate_card_json(&card);
        assert!(report.is_valid(), "{:?}", report.errors);
        assert!(report.warnings.iter().any(|w| w.contains("chart")));
    }

    #[test]
    fn unknown_prop_is_a_warning() {
        let card =
            json!({"schema":"2.0","body":{"elements":[{"tag":"markdown","content":"x","nope":1}]}});
        let report = validate_card_json(&card);
        assert!(report.is_valid());
        assert!(report.warnings.iter().any(|w| w.contains("nope")));
    }

    #[test]
    fn element_id_rules() {
        assert!(valid_element_id("markdown_1"));
        assert!(!valid_element_id("1markdown"));
        assert!(!valid_element_id("markdown-1"));
        assert!(!valid_element_id(&"a".repeat(21)));

        let card = json!({"schema":"2.0","body":{"elements":[
            {"tag":"markdown","content":"a","element_id":"dup"},
            {"tag":"markdown","content":"b","element_id":"dup"}
        ]}});
        let report = validate_card_json(&card);
        assert!(!report.is_valid());
        assert!(report.errors[0].contains("重复"), "{:?}", report.errors);
    }

    #[test]
    fn nested_containers_are_walked() {
        let card = json!({"schema":"2.0","body":{"elements":[
            {"tag":"column_set","columns":[
                {"tag":"column","elements":[
                    {"tag":"collapsible_panel","expanded":true,
                     "header":{"title":{"tag":"plain_text","content":"详情"}},
                     "elements":[{"tag":"markdown","content":"x"}]}
                ]}
            ]}
        ]}});
        let report = validate_card_json(&card);
        assert!(report.is_valid(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(count_elements(&card), 4);
        assert_eq!(
            collect_tags(&card),
            vec![
                "column_set".to_string(),
                "column".to_string(),
                "collapsible_panel".to_string(),
                "markdown".to_string()
            ]
        );
    }

    #[test]
    fn header_template_enum_is_checked_only_for_literals() {
        let ok = json!({"schema":"2.0","header":{"template":"${severity_color}"},"body":{"elements":[]}});
        assert!(validate_card_json(&ok).warnings.is_empty());

        let bad = json!({"schema":"2.0","header":{"template":"purple-ish"},"body":{"elements":[]}});
        assert!(validate_card_json(&bad)
            .warnings
            .iter()
            .any(|w| w.contains("header.template")));
    }
}
