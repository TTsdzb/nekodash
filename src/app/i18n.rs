use serde_json::Value;
use std::collections::BTreeMap;

pub struct Translations(BTreeMap<&'static str, Value>);
impl Translations {
    pub fn load() -> Result<Self, serde_json::Error> {
        let mut all = BTreeMap::new();
        for (lang, raw) in [
            ("zh", include_str!("../../assets/i18n/zh.json")),
            ("en", include_str!("../../assets/i18n/en.json")),
            ("ru", include_str!("../../assets/i18n/ru.json")),
            ("ko", include_str!("../../assets/i18n/ko.json")),
            ("fr", include_str!("../../assets/i18n/fr.json")),
            ("ja", include_str!("../../assets/i18n/ja.json")),
            ("fa", include_str!("../../assets/i18n/fa.json")),
        ] {
            all.insert(lang, serde_json::from_str(raw)?);
        }
        Ok(Self(all))
    }
    pub fn text(&self, key: &str, lang: &str) -> String {
        let alias = match key {
            "testAll" => "testAll",
            "settings" => "setup",
            _ => key,
        };
        let get = |lang| {
            self.0
                .get(lang)
                .and_then(|v| alias.split('.').try_fold(v, |v, p| v.get(p)))
                .and_then(Value::as_str)
        };
        if let Some(text) = get(lang).or_else(|| get("en")) {
            return text.into();
        }
        let local = match alias {
            "size" => ("数量", "Size"),
            "behavior" => ("行为", "Behavior"),
            "format" => ("格式", "Format"),
            "networkType" => ("网络类型", "Network types"),
            "topProxies" => ("代理用量", "Top proxies"),
            "testAll" => ("全部测速", "Test all"),
            "latencyTest" => ("测速", "Test latency"),
            "testConcurrency" => ("并发测速数", "Concurrent tests"),
            "connectionHistoryLimit" => ("连接历史保留数量", "Connection history limit"),
            "retentionDays" => ("保留天数（0 为永久）", "Retention days (0 for unlimited)"),
            "connect" => ("连接", "Connect"),
            "connecting" => ("正在连接…", "Connecting…"),
            "connected" => ("已连接", "Connected"),
            "disconnected" => ("未连接", "Disconnected"),
            "reconnecting" => ("正在恢复连接…", "Reconnecting…"),
            "saved" => ("已保存", "Saved"),
            "success" => ("操作完成", "Operation completed"),
            "back" => ("返回", "Back"),
            "actions" => ("操作", "Actions"),
            "noData" => ("暂无数据", "No data"),
            "more" => ("更多", "More"),
            "details" => ("详情", "Details"),
            "refresh" => ("刷新", "Refresh"),
            "switchTheme" => ("切换主题", "Switch theme"),
            "edit" => ("编辑", "Edit"),
            "delete" => ("删除", "Delete"),
            "copy" => ("复制", "Copy"),
            "confirm" => ("确认", "Confirm"),
            "cancel" => ("取消", "Cancel"),
            "active" => ("活动", "Active"),
            "closed" => ("已关闭", "Closed"),
            "all" => ("全部", "All"),
            "updateAll" => ("全部更新", "Update all"),
            "closeAll" => ("全部关闭", "Close all"),
            "enabled" => ("已启用", "Enabled"),
            "disabled" => ("已禁用", "Disabled"),
            "status" => ("状态", "Status"),
            "time" => ("时间", "Time"),
            "payload" => ("内容", "Content"),
            "hitCount" => ("命中次数", "Matches"),
            "lastMatchedAt" => ("最近命中", "Last match"),
            "total" => ("总量", "Total"),
            "grouping" => ("分组", "Group by"),
            "exportPath" => ("保存文件路径", "Save file path"),
            "importPath" => ("导入文件路径", "Import file path"),
            "operationConfirm" => (
                "此操作将作用于当前后端，请确认后执行。",
                "Confirm this operation on the current endpoint.",
            ),
            "unauthorized" => (
                "鉴权失败，请检查 Secret。",
                "Authentication failed. Check the secret.",
            ),
            "unavailable" => (
                "无法连接，请检查地址、网络和证书。",
                "Connection failed. Check the address, network and certificate.",
            ),
            "timeout" => (
                "请求超时，可稍后重试。",
                "The request timed out. Try again later.",
            ),
            "unsupported" => (
                "当前核心版本未提供此接口。",
                "This core version does not provide this API.",
            ),
            "invalidInput" => (
                "输入无效，请检查填写内容。",
                "Invalid input. Check the entered values.",
            ),
            "interrupted" => (
                "操作结果未能确认，已重新获取核心状态。",
                "The operation result is uncertain. Core state has been refreshed.",
            ),
            _ => return key.into(),
        };
        if lang == "zh" {
            local.0.into()
        } else {
            local.1.into()
        }
    }
}
