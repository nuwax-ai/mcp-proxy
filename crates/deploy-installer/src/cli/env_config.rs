//! `.document-parser.env` 读写与上传后端配置判定（OSS / 自定义上传二选一）。
//!
//! 解析语义全程与运行时 dotenvy 对齐：first-wins、引号剥壳、注释跳过。

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// 上传后端相关的环境变量键：OSS 凭证 + 自定义上传后端（nuwax 风格）配置。
///
/// 语义与 document-parser 侧 `load_custom_upload_config_from_env` 对齐：
/// `DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL` trim 后非空即启用自定义后端，
/// api_key 允许为空（无鉴权部署），path 兜底 `/api/v1/file/upload`。
///
/// `ALIYUN_OSS_*_BUCKET` 与运行时 `load_oss_config_from_env` 的覆盖键同名
/// （app_config.rs）——config.yml 模板的 bucket 占位符可经这两个键注入，
/// 不落盘则用户须手改 config.yml（安装期有占位符 fail-fast 校验）。
pub const UPLOAD_ENV_KEYS: &[&str] = &[
    "OSS_ACCESS_KEY_ID",
    "OSS_ACCESS_KEY_SECRET",
    "ALIYUN_OSS_PUBLIC_BUCKET",
    "ALIYUN_OSS_PRIVATE_BUCKET",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY",
    "DOCUMENT_PARSER_CUSTOM_UPLOAD_PATH",
];

/// config.yml 模板里 OSS bucket 的占位符值——OSS 密钥已配置而 bucket 仍是
/// 这些值时，异步上传会在运行期才炸 E010（2026-09-10 三机深测实测），
/// 安装期与运行期校验都拒绝。
pub const OSS_BUCKET_PLACEHOLDERS: &[&str] = &["your-public-bucket", "your-private-bucket"];

/// 把上传后端配置（OSS 密钥与/或自定义上传后端变量）从环境落盘到 `.env`。
///
/// 各键**独立** upsert（环境里非空即写），不要求 OSS 成对出现——
/// "是否配置完成"的判定交给 [`upload_backend_configured`]。
/// 仅应在 [`upload_backend_configured`] 为 false（尚未配置任何后端）时调用：
/// 已配置的 `.env` 不应被 shell 环境残留值覆盖。
/// 读失败（权限/编码错误）会中止而非当作空文件——避免整份凭证被静默重写。
pub fn apply_upload_config_from_env(env_path: &Path) -> Result<()> {
    let mut lines = read_env_lines(env_path)?;
    let mut changed = false;
    for key in UPLOAD_ENV_KEYS {
        if let Some(value) = std::env::var(key).ok().filter(|s| !s.trim().is_empty()) {
            upsert_env_line(&mut lines, key, &value);
            changed = true;
        }
    }
    if changed {
        write_env_lines(env_path, &lines)?;
    }
    Ok(())
}

/// Whether `.document-parser.env` has non-empty OSS keys.
///
/// 解析语义与运行时 dotenvy 一致（**first-wins**：同键多行取首个非注释行）。
pub fn oss_keys_configured(env_path: &Path) -> bool {
    let values = match read_env_values(env_path) {
        Ok(v) => v,
        Err(_) => return false,
    };
    values
        .get("OSS_ACCESS_KEY_ID")
        .is_some_and(|v| !v.is_empty())
        && values
            .get("OSS_ACCESS_KEY_SECRET")
            .is_some_and(|v| !v.is_empty())
}

/// Whether `.document-parser.env` enables the custom upload backend
/// (`DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL` non-empty).
pub fn custom_upload_configured(env_path: &Path) -> bool {
    match read_env_values(env_path) {
        Ok(values) => values
            .get("DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL")
            .is_some_and(|v| !v.is_empty()),
        Err(_) => false,
    }
}

/// 上传后端是否就绪：OSS 密钥或自定义上传后端**二选一**即可。
pub fn upload_backend_configured(env_path: &Path) -> bool {
    oss_keys_configured(env_path) || custom_upload_configured(env_path)
}

/// Read a single KEY's value from an `.env`-style file (quotes stripped, None if absent).
pub fn parse_env_file_value(env_path: &Path, key: &str) -> Option<String> {
    read_env_values(env_path).ok()?.get(key).cloned()
}

/// Read and parse an `.env`-style file once (KEY=VALUE, quotes stripped).
///
/// 解析语义：跳过注释与空行；**同键多行取首个**（first-wins）——与运行时
/// dotenvy 对 `.env` 的取值语义一致，也与 [`upsert_env_line`] 只改首个
/// 匹配行的写入语义自洽（读首个、写首个）。文件不存在视为空。
fn read_env_values(env_path: &Path) -> Result<std::collections::HashMap<String, String>> {
    if !env_path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let content =
        fs::read_to_string(env_path).with_context(|| format!("read {}", env_path.display()))?;
    Ok(parse_env_file_values(&content))
}

/// Parse `.env`-style content（first-wins）.
fn parse_env_file_values(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            // first-wins：首次出现的键生效（对齐 dotenvy 运行时语义）
            map.entry(k.trim().to_string())
                .or_insert_with(|| v.to_string());
        }
    }
    map
}

/// Read `.env` lines preserving everything (comments, order) for round-trip edits.
///
/// 文件存在但读失败时返回 Err（调用方中止）——**不**当作空文件，
/// 否则后续 write_env_lines 会用残缺内容整份覆盖已配置的凭证。
fn read_env_lines(env_path: &Path) -> Result<Vec<String>> {
    if !env_path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(env_path).with_context(|| format!("read {}", env_path.display()))?;
    Ok(content.lines().map(String::from).collect())
}

fn write_env_lines(env_path: &Path, lines: &[String]) -> Result<()> {
    let body = format!("{}\n", lines.join("\n"));
    crate::write_user_file(env_path, &body, Some(0o600))
        .map_err(|e| anyhow::anyhow!("write {}: {e}", env_path.display()))
}

fn upsert_env_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(line) = lines.iter_mut().find(|l| {
        let t = l.trim();
        !t.starts_with('#') && t.starts_with(&prefix)
    }) {
        *line = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_env(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let p = dir.path().join(".document-parser.env");
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn upload_backend_configured_three_states() {
        let dir = TempDir::new().unwrap();
        // 皆无 → false
        let p = write_env(&dir, "# empty\n");
        assert!(!oss_keys_configured(&p));
        assert!(!custom_upload_configured(&p));
        assert!(!upload_backend_configured(&p));

        // 仅 OSS 成对 → true
        let p = write_env(&dir, "OSS_ACCESS_KEY_ID=ak\nOSS_ACCESS_KEY_SECRET=sk\n");
        assert!(oss_keys_configured(&p));
        assert!(!custom_upload_configured(&p));
        assert!(upload_backend_configured(&p));

        // 仅 custom（base_url 非空，api_key 可空）→ true
        let p = write_env(
            &dir,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://agent.example.com\nDOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=\n",
        );
        assert!(!oss_keys_configured(&p));
        assert!(custom_upload_configured(&p));
        assert!(upload_backend_configured(&p));

        // 引号值剥壳
        let p = write_env(
            &dir,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL='https://x.com'\n",
        );
        assert!(custom_upload_configured(&p));

        // base_url 空串 = 未启用（与 document-parser 侧语义一致）
        let p = write_env(&dir, "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=\n");
        assert!(!custom_upload_configured(&p));

        // 文件不存在 → false
        assert!(!upload_backend_configured(&dir.path().join("nope.env")));
    }

    #[test]
    fn apply_env_upsert_is_idempotent_and_skips_commented() {
        let mut lines: Vec<String> = vec![
            "# OSS_ACCESS_KEY_ID=old".to_string(),
            "OSS_ACCESS_KEY_ID=first".to_string(),
            String::new(),
        ];
        upsert_env_line(&mut lines, "OSS_ACCESS_KEY_ID", "second");
        // 覆盖非注释行，不动注释行
        assert_eq!(lines[0], "# OSS_ACCESS_KEY_ID=old");
        assert_eq!(lines[1], "OSS_ACCESS_KEY_ID=second");

        // 新键追加
        upsert_env_line(
            &mut lines,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL",
            "https://x",
        );
        assert!(
            lines
                .last()
                .is_some_and(|l| l == "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://x")
        );
    }

    #[test]
    fn duplicate_keys_first_wins_aligning_with_dotenvy() {
        let dir = TempDir::new().unwrap();
        // 值在前、空行在后：first-wins 取首个非空值（对齐 dotenvy 运行时语义）
        let p = write_env(
            &dir,
            "OSS_ACCESS_KEY_ID=real_key\nOSS_ACCESS_KEY_ID=\nOSS_ACCESS_KEY_SECRET=real_secret\n",
        );
        assert!(oss_keys_configured(&p), "首个非注释行的值应生效");

        // 空行在前、值在后（模板注释化后不应出现，但防御）：first-wins 取空 → 未配置
        // 与运行时 dotenvy 行为一致（都取第一个），两侧判定不矛盾
        let p = write_env(
            &dir,
            "DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=\nDOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://x\n",
        );
        assert!(
            !custom_upload_configured(&p),
            "first-wins 下首个空值生效=未配置"
        );
    }

    #[test]
    fn parse_env_file_value_reads_and_strips_quotes() {
        let dir = TempDir::new().unwrap();
        let p = write_env(&dir, "# comment\nFOO='quoted'\nBAR=plain\nBAZ=\"dq\"\n");
        assert_eq!(parse_env_file_value(&p, "FOO").as_deref(), Some("quoted"));
        assert_eq!(parse_env_file_value(&p, "BAR").as_deref(), Some("plain"));
        assert_eq!(parse_env_file_value(&p, "BAZ").as_deref(), Some("dq"));
        assert_eq!(parse_env_file_value(&p, "MISSING"), None);
    }
}
