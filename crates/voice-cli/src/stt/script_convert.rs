//! STT 文字脚本转换：繁→简（OpenCC，英文/非中文透传）。
//!
//! Whisper 中文默认输出繁体（训练语料偏好，无简繁参数），本模块在 STT 结果返回客户端前
//! 做繁→简规范化。OpenCC 只作用于中文字符，英文/数字/拉丁/日韩文原样透传（无损，非翻译）。
//!
//! 单例 [`OPENCC_T2S`] 启动时 init 一次（字典编译期内嵌进二进制，无运行时文件依赖）；
//! init 失败优雅退化为透传（Fail-Soft，不阻塞 STT）。

use std::sync::LazyLock;

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use tracing::warn;

use crate::models::config::OutputScript;

/// 进程级 OpenCC 繁→简转换器（T2s 配置）
static OPENCC_T2S: LazyLock<Option<OpenCC>> =
    LazyLock::new(|| match OpenCC::from_config(BuiltinConfig::T2s) {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("opencc T2s init failed, fallback passthrough: {e}");
            None
        }
    });

/// 繁→简转换；OPENCC 未就绪时透传（Fail-Soft）
pub fn to_simplified(text: &str) -> String {
    OPENCC_T2S
        .as_ref()
        .map(|c| c.convert(text))
        .unwrap_or_else(|| text.to_string())
}

/// 按 `output_script` 配置决定是否转换（Simplified=转，Original=原样）
pub fn convert_if_needed(text: &str, script: OutputScript) -> String {
    match script {
        OutputScript::Simplified => to_simplified(text),
        OutputScript::Original => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t2s_chinese() {
        assert_eq!(
            to_simplified("你好嗎？繁體轉簡體測試。"),
            "你好吗？繁体转简体测试。"
        );
    }

    #[test]
    fn english_passthrough() {
        assert_eq!(
            to_simplified("Hello World, the quick brown fox. 12345"),
            "Hello World, the quick brown fox. 12345"
        );
    }

    #[test]
    fn mixed_chinese_english() {
        assert_eq!(
            to_simplified("我用 iPhone 打電話，call 你 later。"),
            "我用 iPhone 打电话，call 你 later。"
        );
    }

    #[test]
    fn original_mode_no_convert() {
        assert_eq!(
            convert_if_needed("你好嗎？繁體", OutputScript::Original),
            "你好嗎？繁體"
        );
    }

    #[test]
    fn simplified_mode_convert() {
        assert_eq!(
            convert_if_needed("你好嗎？繁體", OutputScript::Simplified),
            "你好吗？繁体"
        );
    }
}
