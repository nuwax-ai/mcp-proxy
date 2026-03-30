//! 国际化模块
//!
//! 使用 rust-i18n 提供多语言支持，支持中文简体、中文繁体、英文三种语言。
//!
//! # 使用方法
//!
//! ```rust,ignore
//! use mcp_common::{t, set_locale, init_locale_from_env};
//!
//! // 初始化语言设置（通常在程序启动时调用）
//! init_locale_from_env();
//!
//! // 获取翻译
//! let msg = t!("errors.mcp_proxy.service_not_found", service = "my-service");
//!
//! // 手动设置语言
//! set_locale("zh-CN");
//! ```
//!
//! # 支持的语言
//!
//! - `en` - English
//! - `zh-CN` - 中文简体
//! - `zh-TW` - 中文繁体
//!
//! # 配置优先级
//!
//! 1. `DEFAULT_LOCALE` 环境变量（最高优先级）
//! 2. `LANG` 系统环境变量
//! 3. 默认使用英文
//!
//! # 线程安全
//!
//! `set_locale()` 和 `init_locale_from_env()` 应在程序启动时调用。
//! 语言设置是全局状态，不建议在运行时多线程环境中修改。

// 注意: rust-i18n 的 i18n! 宏需要在 lib.rs 中调用

/// 导出 t! 宏，用于获取翻译
pub use rust_i18n::t;

/// 设置当前语言
///
/// # 线程安全
///
/// 此函数应在程序启动时调用，不建议在运行时多线程环境中调用。
/// 语言设置是全局状态，并发调用可能导致不一致的翻译结果。
///
/// # 示例
///
/// ```rust
/// use mcp_common::set_locale;
///
/// set_locale("zh-CN");
/// set_locale("en");
/// set_locale("zh-TW");
/// ```
pub fn set_locale(locale: &str) {
    rust_i18n::set_locale(locale);
}

/// 获取当前语言设置
///
/// # 示例
///
/// ```rust
/// use mcp_common::current_locale;
///
/// let locale = current_locale();
/// println!("Current locale: {}", locale);
/// ```
pub fn current_locale() -> String {
    rust_i18n::locale().to_string()
}

/// 支持的语言列表
pub const AVAILABLE_LOCALES: &[&str] = &["en", "zh-CN", "zh-TW"];

/// 默认语言
pub const DEFAULT_LOCALE: &str = "en";

/// 从环境变量初始化语言设置
///
/// 按照以下优先级设置语言：
/// 1. `DEFAULT_LOCALE` 环境变量（最高优先级）
/// 2. `LANG` 系统环境变量（自动解析语言代码）
/// 3. 默认使用英文
///
/// # 示例
///
/// ```rust
/// use mcp_common::init_locale_from_env;
///
/// // 在程序启动时调用
/// init_locale_from_env();
/// ```
pub fn init_locale_from_env() {
    // 优先使用 DEFAULT_LOCALE 环境变量（最高优先级）
    if let Ok(lang) = std::env::var("DEFAULT_LOCALE") {
        let locale = normalize_locale(&lang);
        if AVAILABLE_LOCALES.contains(&locale.as_str()) {
            set_locale(&locale);
            return;
        } else {
            tracing::warn!(
                "Invalid locale '{}' from DEFAULT_LOCALE, falling back. Supported: {:?}",
                locale,
                AVAILABLE_LOCALES
            );
        }
    }

    // 其次尝试从 LANG 环境变量解析
    if let Ok(lang) = std::env::var("LANG") {
        let locale = parse_lang_env(&lang);
        if AVAILABLE_LOCALES.contains(&locale.as_str()) {
            set_locale(&locale);
            return;
        }
    }

    // 使用默认语言
    set_locale(DEFAULT_LOCALE);
}

/// 标准化语言代码
///
/// 支持的输入格式：
/// - `en`, `EN`, `En`, `en_US`, `en_US.UTF-8` -> `en`
/// - `zh-CN`, `zh-cn`, `ZH-CN` -> `zh-CN`
/// - `zh_TW`, `zh-TW` -> `zh-TW`
/// - `zh`, `ZH` -> `zh-CN` (默认简体中文)
fn normalize_locale(input: &str) -> String {
    let input = input.trim();
    // 支持解析带编码/修饰符的值（例如 en_US.UTF-8、zh_CN@cjk）
    let input = input.split('.').next().unwrap_or(input);
    let input = input.split('@').next().unwrap_or(input);

    // 直接匹配
    match input.to_lowercase().as_str() {
        "en" | "en_us" | "en-us" | "en_gb" | "en-gb" => return "en".to_string(),
        "zh-cn" | "zh_cn" | "zh-hans" => return "zh-CN".to_string(),
        "zh-tw" | "zh_tw" | "zh-hant" => return "zh-TW".to_string(),
        "zh" => return "zh-CN".to_string(), // 默认简体中文
        _ => {}
    }

    // 尝试解析语言-地区格式
    let parts: Vec<&str> = input.split(|c| c == '-' || c == '_').collect();
    if parts.len() >= 2 {
        let lang = parts[0].to_lowercase();
        let region = parts[1].to_uppercase();
        // 英文变体统一映射到 en
        if lang == "en" {
            return "en".to_string();
        }
        return format!("{}-{}", lang, region);
    }

    input.to_string()
}

/// 解析 LANG 环境变量
///
/// 支持的格式：
/// - `en_US.UTF-8` -> `en`
/// - `zh_CN.UTF-8` -> `zh-CN`
/// - `zh_TW.UTF-8` -> `zh-TW`
/// - `zh_CN` -> `zh-CN`
fn parse_lang_env(lang: &str) -> String {
    // 移除编码部分 (如 .UTF-8)
    let lang = lang.split('.').next().unwrap_or(lang);

    // 移除修饰部分 (如 @cjk)
    let lang = lang.split('@').next().unwrap_or(lang);

    // 标准化格式
    normalize_locale(lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvRestore {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(v) => unsafe { std::env::set_var(key, v) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    fn prepare_env(overrides: &[(&'static str, Option<&str>)]) -> EnvRestore {
        let tracked_keys = ["DEFAULT_LOCALE", "LANG", "MCP_PROXY_LANG", "APP_LANG"];
        let mut saved = Vec::with_capacity(tracked_keys.len());

        for key in tracked_keys {
            saved.push((key, std::env::var(key).ok()));
            unsafe { std::env::remove_var(key) };
        }

        for (key, value) in overrides {
            match value {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        EnvRestore { saved }
    }

    #[test]
    fn test_normalize_locale() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        assert_eq!(normalize_locale("en"), "en");
        assert_eq!(normalize_locale("EN"), "en");
        assert_eq!(normalize_locale("zh-CN"), "zh-CN");
        assert_eq!(normalize_locale("zh-cn"), "zh-CN");
        assert_eq!(normalize_locale("zh_CN"), "zh-CN");
        assert_eq!(normalize_locale("zh-TW"), "zh-TW");
        assert_eq!(normalize_locale("zh_tw"), "zh-TW");
        assert_eq!(normalize_locale("zh"), "zh-CN");
        assert_eq!(normalize_locale("en_US.UTF-8"), "en");
        assert_eq!(normalize_locale("zh_CN@cjk"), "zh-CN");
    }

    #[test]
    fn test_parse_lang_env() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        assert_eq!(parse_lang_env("en_US.UTF-8"), "en");
        assert_eq!(parse_lang_env("zh_CN.UTF-8"), "zh-CN");
        assert_eq!(parse_lang_env("zh_TW.UTF-8"), "zh-TW");
        assert_eq!(parse_lang_env("zh_CN"), "zh-CN");
        assert_eq!(parse_lang_env("en_US@cjk"), "en");
    }

    #[test]
    fn test_set_and_get_locale() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        set_locale("zh-CN");
        assert_eq!(current_locale(), "zh-CN");

        set_locale("en");
        assert_eq!(current_locale(), "en");

        set_locale("zh-TW");
        assert_eq!(current_locale(), "zh-TW");
    }

    /// 关键翻译键在所有语言中都存在的测试
    #[test]
    fn test_translation_completeness() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        set_locale("en");
        let test_msg = t!("common.success").to_string();
        assert_ne!(
            test_msg, "common.success",
            "Translations are not loaded; expected crate-local locales to be available"
        );

        // 测试关键错误消息键
        let critical_keys = [
            "errors.mcp_proxy.service_not_found",
            "errors.mcp_proxy.service_startup_failed",
            "errors.document_parser.config",
            "errors.document_parser.parse",
            "errors.oss.config",
            "errors.oss.network",
            "errors.voice.config",
            "errors.voice.transcription",
            "cli.startup.service_starting",
            "cli.startup.success",
            "common.error",
            "common.success",
        ];

        for locale in AVAILABLE_LOCALES {
            set_locale(locale);
            for key in &critical_keys {
                let msg = match *key {
                    "errors.mcp_proxy.service_not_found" => {
                        t!("errors.mcp_proxy.service_not_found", service = "test").to_string()
                    }
                    "errors.mcp_proxy.service_startup_failed" => t!(
                        "errors.mcp_proxy.service_startup_failed",
                        mcp_id = "test",
                        reason = "test"
                    )
                    .to_string(),
                    _ => t!(*key).to_string(),
                };
                // 翻译不应该返回 key 本身（表示翻译缺失）
                assert_ne!(
                    msg, *key,
                    "Missing translation for '{}' in locale '{}'",
                    key, locale
                );
            }
        }
    }

    /// 测试所有支持的语言都能正确切换
    ///
    /// 注意：此测试依赖于 rust-i18n 的全局状态，在测试环境中可能不稳定。
    /// 但在实际运行时，语言切换功能是正常的。
    #[test]
    fn test_all_locales_available() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        // 测试每个语言代码都是有效的
        for locale in AVAILABLE_LOCALES {
            // 验证语言代码格式正确
            assert!(
                locale.contains('-') || *locale == "en",
                "Locale '{}' should follow language-region format",
                locale
            );
            // 尝试设置（在翻译文件不可用时可能不生效，但不应该崩溃）
            set_locale(locale);
        }

        // 重置为默认语言
        set_locale(DEFAULT_LOCALE);
    }

    #[test]
    fn test_init_locale_from_env_prefers_default_locale() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        let _env = prepare_env(&[
            ("DEFAULT_LOCALE", Some("zh-TW")),
            ("LANG", Some("en_US.UTF-8")),
            ("MCP_PROXY_LANG", Some("zh-CN")),
            ("APP_LANG", Some("zh-CN")),
        ]);

        init_locale_from_env();
        assert_eq!(current_locale(), "zh-TW");
        set_locale(DEFAULT_LOCALE);
    }

    #[test]
    fn test_init_locale_from_env_falls_back_to_lang() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        let _env = prepare_env(&[
            ("DEFAULT_LOCALE", Some("unsupported")),
            ("LANG", Some("zh_CN.UTF-8")),
        ]);

        init_locale_from_env();
        assert_eq!(current_locale(), "zh-CN");
        set_locale(DEFAULT_LOCALE);
    }

    #[test]
    fn test_init_locale_from_env_falls_back_to_english() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        let _env = prepare_env(&[
            ("DEFAULT_LOCALE", Some("unsupported")),
            ("LANG", Some("ja_JP.UTF-8")),
        ]);

        init_locale_from_env();
        assert_eq!(current_locale(), "en");
        set_locale(DEFAULT_LOCALE);
    }

    #[test]
    fn test_init_locale_from_env_ignores_removed_env_vars() {
        let _guard = test_lock().lock().expect("locale test lock poisoned");
        let _env = prepare_env(&[
            ("MCP_PROXY_LANG", Some("zh-TW")),
            ("APP_LANG", Some("zh-CN")),
        ]);

        init_locale_from_env();
        assert_eq!(current_locale(), "en");
        set_locale(DEFAULT_LOCALE);
    }
}
