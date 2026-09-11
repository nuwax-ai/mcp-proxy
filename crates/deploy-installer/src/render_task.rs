//! Windows 任务计划程序（Task Scheduler）任务定义 XML 渲染。
//!
//! 与 [`crate::render::render_unit`]（systemd）/ [`crate::render_plist::render_launchd_plist`]
//! （launchd）同层级：从 [`ServiceSpec`] 渲染出可注册的任务定义。schema 1.3（Win8+）。
//!
//! 平台语义映射：
//! - `LogonTrigger` + 30s 延迟 ≈ launchd LaunchAgent 的登录加载；
//!   `Principals` 用 `S4U`（无需用户登录桌面即运行，安装时也不需要提权）。
//! - **环境变量不经任务定义传递**：`schtasks /create` 实测拒绝 `<Exec>` 下的
//!   `<EnvironmentVariables>` 元素（Win11 报"系统找不到指定的文件"）。凭证与
//!   配置走 `.env` 文件由**服务自读**（与 launchd 后端同一模式——launchd 无
//!   `EnvironmentFile=`，systemd 的 EnvironmentFile 只是冗余便利）。
//! - **无 RestartOnFailure**（曾配 1 分钟 × 10 次，53 实测撤除）：`/end` 强杀被
//!   记为失败，1 分钟后的自动重拉会**停掉刚被 restart 命令拉起的实例**
//!   （failure-restart 绕过 MultipleInstancesPolicy=IgnoreNew 直接重启任务），
//!   双起互杀致 restart 假成功。失败自愈交给安装器 restart / 用户。
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
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{program}</Command>
      <Arguments>{args}</Arguments>
      <WorkingDirectory>{install_dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        description = xml_escape(&spec.description),
    ))
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

    fn minimal_spec() -> ServiceSpec {
        ServiceSpec {
            name: "document-parser".into(),
            description: "test".into(),
            identity: crate::spec::ServiceIdentity {
                user: "u".into(),
                group: "g".into(),
            },
            install_dir: std::path::PathBuf::from("C:\\dp"),
            exec_start: vec![
                "C:\\dp\\document-parser.exe".into(),
                "--config".into(),
                "C:\\dp\\config.yml".into(),
                "server".into(),
            ],
            env_file: Some(std::path::PathBuf::from("C:\\dp\\.env")),
            extra_env: vec![("RUST_LOG".into(), "info".into())],
            kill_signal: None,
            timeout_stop_sec: None,
            syslog_identifier: None,
            drop_ins: vec![],
            supplementary_groups: vec![],
            required_paths: vec![],
            listen_port: None,
        }
    }

    #[test]
    fn render_task_xml_no_env_inline_and_core_fields() {
        let spec = minimal_spec();
        let xml = render_task_xml(&spec, true).unwrap();
        // schtasks 实测拒绝 Exec 下的 EnvironmentVariables——绝不渲染
        assert!(!xml.contains("EnvironmentVariables"));
        assert!(xml.contains("<Command>C:\\dp\\document-parser.exe</Command>"));
        assert!(xml.contains("<WorkingDirectory>C:\\dp</WorkingDirectory>"));
        assert!(xml.contains("S4U</LogonType>"));
        assert!(xml.contains("PT0S</ExecutionTimeLimit>"));
        // RestartOnFailure 已撤除（53 实测：failure-restart 会停掉刚拉起的实例）
        assert!(!xml.contains("RestartOnFailure"));
    }

    #[test]
    fn xml_escape_covers_five_entities() {
        assert_eq!(xml_escape("&<>\"'"), "&amp;&lt;&gt;&quot;&apos;");
    }
}
