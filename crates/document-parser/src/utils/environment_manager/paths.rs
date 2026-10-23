//! 跨平台路径与虚拟环境静态工具（venv 内 python/可执行文件/激活脚本路径等）。

use super::*;

impl EnvironmentManager {
    /// 获取虚拟环境中的Python可执行文件路径（跨平台）
    pub fn get_venv_python_path(venv_path: &Path) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/python.exe
            venv_path.join("Scripts").join("python.exe")
        } else {
            // Unix-like: bin/python
            venv_path.join("bin").join("python")
        }
    }

    /// 获取虚拟环境中的可执行文件路径（跨平台）
    pub fn get_venv_executable_path(venv_path: &Path, executable_name: &str) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/{executable}.exe
            let exe_name = if executable_name.ends_with(".exe") {
                executable_name.to_string()
            } else {
                format!("{executable_name}.exe")
            };
            venv_path.join("Scripts").join(exe_name)
        } else {
            // Unix-like: bin/{executable}
            venv_path.join("bin").join(executable_name)
        }
    }

    /// 获取虚拟环境激活脚本路径（跨平台）
    pub fn get_venv_activation_script(venv_path: &Path) -> std::path::PathBuf {
        if cfg!(windows) {
            // Windows: Scripts/activate.bat or Scripts/Activate.ps1
            venv_path.join("Scripts").join("activate.bat")
        } else {
            // Unix-like: bin/activate
            venv_path.join("bin").join("activate")
        }
    }

    /// 获取系统Python可执行文件名（跨平台）
    pub fn get_system_python_executable() -> Vec<String> {
        if cfg!(windows) {
            // Windows: python.exe, python3.exe, py.exe
            vec![
                "python.exe".to_string(),
                "python3.exe".to_string(),
                "py.exe".to_string(),
            ]
        } else {
            // Unix-like: python3, python
            vec!["python3".to_string(), "python".to_string()]
        }
    }

    /// 检查可执行文件是否存在于PATH中（跨平台）
    pub async fn is_executable_in_path(executable: &str) -> bool {
        let which_cmd = if cfg!(windows) { "where" } else { "which" };

        match Command::new(which_cmd).arg(executable).output().await {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// 构造 uv 子进程命令，解决 nohup/SSH 非交互/systemd 场景下进程 PATH 不含
    /// `~/.local/bin` 导致「装完 uv 仍探测不到」的问题。
    ///
    /// 解析顺序：PATH 命中（复用 [`Self::is_executable_in_path`]）→ 沿用裸命令名
    /// `uv`（由 OS 解析，行为与历史一致）；否则依次探测 [`uv_candidate_dirs`]
    /// 中的常见安装目录，命中则用绝对路径；全部未命中 → 回退裸命令名 `uv`，
    /// 保留既有的 NotFound 报错链路（如「uv安装后仍不可用」）。
    /// 不写进程环境变量（edition 2024 下 `set_var` 为 unsafe，避免使用）。
    pub async fn uv_command() -> Command {
        if Self::is_executable_in_path("uv").await {
            return Command::new("uv");
        }
        let candidates = uv_candidate_dirs(std::env::home_dir().as_deref());
        if let Some(path) = find_uv_executable(&candidates) {
            debug!("uv not on PATH; using candidate {}", path.display());
            return Command::new(path);
        }
        Command::new("uv")
    }

    /// 测试虚拟环境激活（跨平台）
    pub async fn test_virtual_environment_activation(
        &self,
        venv_path: &Path,
    ) -> Result<bool, AppError> {
        let python_exe = Self::get_venv_python_path(venv_path);

        if !python_exe.exists() {
            return Ok(false);
        }

        // 测试Python可执行文件是否工作
        let test_cmd = Command::new(&python_exe)
            .arg("-c")
            .arg("import sys; print('VENV_TEST_SUCCESS'); print(sys.prefix)")
            .output();

        match timeout(Duration::from_secs(10), test_cmd).await {
            Ok(Ok(output)) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("VENV_TEST_SUCCESS") {
                    debug!(
                        "Virtual environment activation test successful: {}",
                        python_exe.display()
                    );
                    Ok(true)
                } else {
                    debug!("Virtual environment activation test failed: Incorrect output");
                    Ok(false)
                }
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!("Virtual environment activation test failed: {}", stderr);
                Ok(false)
            }
            Ok(Err(e)) => {
                debug!(
                    "Virtual environment activation test execution failed: {}",
                    e
                );
                Ok(false)
            }
            Err(_) => {
                debug!("Virtual environment activation test timed out");
                Ok(false)
            }
        }
    }

    /// 获取虚拟环境信息（跨平台）
    pub async fn get_virtual_environment_info(
        &self,
        venv_path: &Path,
    ) -> Result<VirtualEnvInfo, AppError> {
        let python_exe = Self::get_venv_python_path(venv_path);
        let activation_script = Self::get_venv_activation_script(venv_path);
        let pip_exe = Self::get_venv_executable_path(venv_path, "pip");

        let is_valid = self.test_virtual_environment_activation(venv_path).await?;

        Ok(VirtualEnvInfo {
            path: venv_path.to_path_buf(),
            python_executable: python_exe,
            pip_executable: pip_exe,
            activation_script,
            is_valid,
            platform: if cfg!(windows) {
                "windows".to_string()
            } else {
                "unix".to_string()
            },
        })
    }

    /// 获取跨平台环境变量设置
    pub fn get_cross_platform_env_vars(
        &self,
        venv_path: &Path,
    ) -> std::collections::HashMap<String, String> {
        let mut env_vars = std::collections::HashMap::new();

        if cfg!(windows) {
            // Windows环境变量
            env_vars.insert(
                "VIRTUAL_ENV".to_string(),
                venv_path.to_string_lossy().to_string(),
            );
            env_vars.insert(
                "PATH".to_string(),
                format!(
                    "{};{}",
                    venv_path.join("Scripts").to_string_lossy(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        } else {
            // Unix-like环境变量
            env_vars.insert(
                "VIRTUAL_ENV".to_string(),
                venv_path.to_string_lossy().to_string(),
            );
            env_vars.insert(
                "PATH".to_string(),
                format!(
                    "{}:{}",
                    venv_path.join("bin").to_string_lossy(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        }

        env_vars
    }
}

/// uv 常见安装目录候选（不含 PATH 搜索）。
///
/// unix 顺序：`~/.local/bin`（astral 安装脚本默认位置）、`/usr/local/bin`（系统级
/// 安装/管理员手工放置）、`~/.cargo/bin`（rustup 环境习惯位置）；windows 仅
/// `%USERPROFILE%\.local\bin`。home 由调用方传入（不读进程环境），保证测试零副作用。
fn uv_candidate_dirs(home: Option<&Path>) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local").join("bin"));
    }
    if cfg!(windows) {
        return dirs;
    }
    dirs.push(std::path::PathBuf::from("/usr/local/bin"));
    if let Some(home) = home {
        dirs.push(home.join(".cargo").join("bin"));
    }
    dirs
}

/// 返回候选目录中第一个存在的 uv 可执行文件（unix: `uv`；windows: `uv.exe`）。
fn find_uv_executable(candidate_dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(windows) { "uv.exe" } else { "uv" };
    candidate_dirs
        .iter()
        .map(|d| d.join(exe_name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cross_platform_path_functions() {
        use std::path::Path;

        let venv_path = Path::new("test_venv");

        // 测试Python路径生成
        let python_path = EnvironmentManager::get_venv_python_path(venv_path);
        if cfg!(windows) {
            assert_eq!(python_path, venv_path.join("Scripts").join("python.exe"));
        } else {
            assert_eq!(python_path, venv_path.join("bin").join("python"));
        }

        // 测试可执行文件路径生成
        let mineru_path = EnvironmentManager::get_venv_executable_path(venv_path, "mineru");
        if cfg!(windows) {
            assert_eq!(mineru_path, venv_path.join("Scripts").join("mineru.exe"));
        } else {
            assert_eq!(mineru_path, venv_path.join("bin").join("mineru"));
        }

        // 测试激活脚本路径
        let activation_script = EnvironmentManager::get_venv_activation_script(venv_path);
        if cfg!(windows) {
            assert_eq!(
                activation_script,
                venv_path.join("Scripts").join("activate.bat")
            );
        } else {
            assert_eq!(activation_script, venv_path.join("bin").join("activate"));
        }

        // 测试系统Python可执行文件列表
        let python_executables = EnvironmentManager::get_system_python_executable();
        assert!(!python_executables.is_empty());

        if cfg!(windows) {
            assert!(python_executables.contains(&"python.exe".to_string()));
            assert!(python_executables.contains(&"python3.exe".to_string()));
        } else {
            assert!(python_executables.contains(&"python3".to_string()));
            assert!(python_executables.contains(&"python".to_string()));
        }
    }

    #[tokio::test]
    async fn test_cross_platform_environment_variables() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        let venv_path = temp_dir.path().join("venv");
        let env_vars = manager.get_cross_platform_env_vars(&venv_path);

        // 验证VIRTUAL_ENV变量
        assert_eq!(
            env_vars.get("VIRTUAL_ENV").unwrap(),
            &venv_path.to_string_lossy().to_string()
        );

        // 验证PATH变量包含正确的路径
        let path_var = env_vars.get("PATH").unwrap();
        if cfg!(windows) {
            assert!(path_var.contains(&venv_path.join("Scripts").to_string_lossy().to_string()));
        } else {
            assert!(path_var.contains(&venv_path.join("bin").to_string_lossy().to_string()));
        }
    }

    #[tokio::test]
    async fn test_virtual_environment_info() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = EnvironmentManager::new(
            "python3".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        );

        let venv_path = temp_dir.path().join("venv");

        // 测试虚拟环境信息获取（即使虚拟环境不存在）
        let venv_info_result = manager.get_virtual_environment_info(&venv_path).await;
        assert!(venv_info_result.is_ok());

        let venv_info = venv_info_result.unwrap();
        assert_eq!(venv_info.path, venv_path);
        assert!(!venv_info.is_valid); // 因为虚拟环境不存在

        // 验证平台特定路径
        if cfg!(windows) {
            assert_eq!(
                venv_info.python_executable,
                venv_path.join("Scripts").join("python.exe")
            );
            assert_eq!(
                venv_info.pip_executable,
                venv_path.join("Scripts").join("pip.exe")
            );
            assert_eq!(
                venv_info.activation_script,
                venv_path.join("Scripts").join("activate.bat")
            );
            assert_eq!(venv_info.platform, "windows");
        } else {
            assert_eq!(
                venv_info.python_executable,
                venv_path.join("bin").join("python")
            );
            assert_eq!(venv_info.pip_executable, venv_path.join("bin").join("pip"));
            assert_eq!(
                venv_info.activation_script,
                venv_path.join("bin").join("activate")
            );
            assert_eq!(venv_info.platform, "unix");
        }
    }
}

#[cfg(test)]
mod uv_resolution_tests {
    use super::*;

    #[test]
    fn test_uv_candidate_dirs_order_unix() {
        let home = std::path::Path::new("/home/t");
        let dirs = uv_candidate_dirs(Some(home));
        if cfg!(windows) {
            assert_eq!(dirs, vec![home.join(".local").join("bin")]);
        } else {
            assert_eq!(
                dirs,
                vec![
                    home.join(".local").join("bin"),
                    std::path::PathBuf::from("/usr/local/bin"),
                    home.join(".cargo").join("bin"),
                ]
            );
        }
        // 无 home 时仅保留系统级候选
        let dirs = uv_candidate_dirs(None);
        if !cfg!(windows) {
            assert_eq!(dirs, vec![std::path::PathBuf::from("/usr/local/bin")]);
        }
    }

    #[test]
    fn test_find_uv_executable_first_existing_wins() {
        let first = tempfile::TempDir::new().unwrap();
        let second = tempfile::TempDir::new().unwrap();
        let exe_name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let uv_path = first.path().join(exe_name);
        std::fs::write(&uv_path, b"stub").unwrap();

        let found = find_uv_executable(&[first.path().to_path_buf(), second.path().to_path_buf()]);
        assert_eq!(found, Some(uv_path));
    }

    #[test]
    fn test_find_uv_executable_none_when_missing() {
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();
        assert_eq!(
            find_uv_executable(&[a.path().to_path_buf(), b.path().to_path_buf()]),
            None
        );
    }

    #[tokio::test]
    async fn test_uv_command_program_ends_with_uv() {
        // 冒烟：无论解析到裸名还是绝对路径，program 必须指向 uv
        let cmd = EnvironmentManager::uv_command().await;
        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert!(program.ends_with("uv"), "unexpected program: {program}");
    }
}
