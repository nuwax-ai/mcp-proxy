const AVAILABLE_LOCALES: &[&str] = &["en", "zh-CN", "zh-TW"];
const DEFAULT_LOCALE: &str = "en";

pub fn init_locale_from_env() {
    for env_key in ["DEFAULT_LOCALE", "LANG"] {
        let Ok(raw_locale) = std::env::var(env_key) else {
            continue;
        };

        let normalized = if env_key == "LANG" {
            parse_lang_env(&raw_locale)
        } else {
            normalize_locale(&raw_locale)
        };

        if AVAILABLE_LOCALES.contains(&normalized.as_str()) {
            rust_i18n::set_locale(&normalized);
            return;
        }
    }

    rust_i18n::set_locale(DEFAULT_LOCALE);
}

fn parse_lang_env(lang: &str) -> String {
    let lang = lang.split('.').next().unwrap_or(lang);
    let lang = lang.split('@').next().unwrap_or(lang);
    normalize_locale(lang)
}

fn normalize_locale(input: &str) -> String {
    let input = input.trim();
    let input = input.split('.').next().unwrap_or(input);
    let input = input.split('@').next().unwrap_or(input);

    match input.to_lowercase().as_str() {
        "en" | "en_us" | "en-us" | "en_gb" | "en-gb" => "en".to_string(),
        "zh-cn" | "zh_cn" | "zh-hans" | "zh" => "zh-CN".to_string(),
        "zh-tw" | "zh_tw" | "zh-hant" => "zh-TW".to_string(),
        _ => input.to_string(),
    }
}
