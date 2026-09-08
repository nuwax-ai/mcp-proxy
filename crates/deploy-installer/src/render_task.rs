//! Windows 任务计划程序（Task Scheduler）任务定义 XML 渲染。
//!
//! 与 [`crate::render::render_unit`]（systemd）/ [`crate::render_plist::render_launchd_plist`]
//! （launchd）同层级：从 [`ServiceSpec`] 渲染出可注册的任务定义。schema 1.3（Win8+）。
//!
//! 平台语义映射：
//! - `LogonTrigger` + 30s 延迟 ≈ launchd LaunchAgent 的登录加载；
//!   `Principals` 用 `S4U`（无需用户登录桌面即运行，安装时也不需要提权）。
//! - `EnvironmentVariables`（Exec 子元素）：Task XML 没有 `EnvironmentFile=` 等价物，
//!   `spec.env_file` 的内容在渲染时**内联**（.env 解析语义与 dotenvy 运行时一致：
//!   first-wins、去引号）。凭证随任务定义明文落盘于 `%LOCALAPPDATA%` 的任务注册表
//!   中——与 systemd EnvironmentFile / launchd plist 内联 EXTRA_ENV 的既有暴露面同级。
//! - `RestartOnFailure`（1 分钟间隔 × 10 次）≈ `Restart=on-failure` + `RestartSec`。
//! - `kill_signal` / `timeout_stop_sec` / `syslog_identifier` / `supplementary_groups`
//!   在任务计划程序无对应物，忽略（服务自身写文件日志，stdout 不经任务捕获）。

use crate::error::{InstallerError, Result};
use crate::spec::ServiceSpec;

/// 渲染 Windows 任务计划程序任务定义 XML。
///
/// `run_at_logon=false` 时触发器渲染为禁用（对齐 launchd 的 `run_at_load` 语义，
/// 供 dry-run / --no-start 展示同一份定义）。
pub fn render_task_xml(spec: &ServiceSpec, run_at_logon: bool) -> Result<String> {
    let task_name = spec.task_name();
    let install_dir = crate::render::sanitize_path(&spec.install_dir)?;
    let Some((program, args)) = spec.exec_start.split_first() else {
        return Err(InstallerError::EmptyExecStart);
    };
    let program = crate::render::sanitize_path(std::path::Path::new(program))?;
    let args = shell_join_args(args)?;

    let user_id = crate::checks::current_user()?;
    let user_id = crate::render::sanitize_unit_value("UserId", &user_id)?;

    let env_block = task_xml_env_block(&collect_env_pairs(spec)?)?;
    let trigger_enabled = if run_at_logon { "true" } else { "false" };

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<!-- 由 deploy-installer 生成；任务名 {task_name}，手动检查: schtasks /query /tn {task_name} /xml -->
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
    <URI>\{task_name}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>{trigger_enabled}</Enabled>
      <Delay>PT30S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user_id}</UserId>
      <LogonType>S4U</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>10</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{program}</Command>
      <Arguments>{args}</Arguments>
      <WorkingDirectory>{install_dir}</WorkingDirectory>
{env_block}    </Exec>
  </Actions>
</Task>
"#,
        description = xml_escape(&spec.description),
    ))
}

/// 汇总任务内联环境变量：`env_file` 内容（有序 first-wins）追加 `extra_env`。
fn collect_env_pairs(spec: &ServiceSpec) -> Result<Vec<(String, String)>> {
    let mut pairs: Vec<(String, String)> = match &spec.env_file {
        Some(path) if path.is_file() => {
            let content =
                std::fs::read_to_string(path).map_err(|e| InstallerError::CommandFailed {
                    cmd: format!("read {}", path.display()),
                    detail: e.to_string(),
                })?;
            parse_env_pairs(&content)
        }
        _ => Vec::new(),
    };
    for (k, v) in &spec.extra_env {
        // first-wins：文件值优先，extra_env 仅补缺（与 launchd plist 的
        // PATH/HOME/TMPDIR + extra_env 组合语义一致）
        if !pairs.iter().any(|(ek, _)| ek == k) {
            pairs.push((k.clone(), v.clone()));
        }
    }
    Ok(pairs)
}

/// 解析 `.env` 风格内容为**有序**键值对（first-wins、跳过注释/空行、去成对引号）。
///
/// 语义对齐 `cli/common.rs::parse_env_file_values`（dotenvy 运行时行为）；
/// 单独成序是因为任务 XML 内联需要确定性顺序，HashMap 迭代序不稳定。
pub fn parse_env_pairs(content: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            let k = k.trim();
            if !pairs.iter().any(|(ek, _)| ek == k) {
                pairs.push((k.to_string(), v.to_string()));
            }
        }
    }
    pairs
}

/// 纯函数：有序键值对 → `<EnvironmentVariables>` 块（含缩进与结尾换行，空表输出空串）。
pub fn task_xml_env_block(env_pairs: &[(String, String)]) -> Result<String> {
    if env_pairs.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("      <EnvironmentVariables>\n");
    for (k, v) in env_pairs {
        let key = crate::render::sanitize_unit_value(k, k)?;
        out.push_str(&format!(
            "        <Variable Name=\"{}\">\n          <Value>{}</Value>\n        </Variable>\n",
            xml_escape(&key),
            xml_escape(v)
        ));
    }
    out.push_str("      </EnvironmentVariables>\n");
    Ok(out)
}

/// Task XML `Arguments` 拼接：每个参数过一遍 unit-value 清洗再空格连接。
fn shell_join_args(args: &[String]) -> Result<String> {
    let mut cleaned = Vec::with_capacity(args.len());
    for a in args {
        cleaned.push(crate::render::sanitize_unit_value("arg", a)?);
    }
    Ok(cleaned.join(" "))
}

/// 最小 XML 实体转义（属性值与文本节点足够）。
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_env_block_skips_empty() {
        assert_eq!(task_xml_env_block(&[]).unwrap(), "");
    }

    #[test]
    fn task_xml_env_block_escapes_specials() {
        let block = task_xml_env_block(&[("K&<".into(), "v>\"'".into())]).unwrap();
        assert!(block.contains("Name=\"K&amp;&lt;\""));
        assert!(block.contains("<Value>v&gt;&quot;&apos;</Value>"));
    }

    #[test]
    fn parse_env_pairs_first_wins_and_strips_quotes() {
        let pairs = parse_env_pairs("# c\n\nA=1\nA=2\nB=\"x y\"\nC='z'\nBAD\n");
        assert_eq!(
            pairs,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "x y".to_string()),
                ("C".to_string(), "z".to_string()),
            ]
        );
    }

    #[test]
    fn xml_escape_covers_five_entities() {
        assert_eq!(xml_escape("&<>\"'"), "&amp;&lt;&gt;&quot;&apos;");
    }
}
