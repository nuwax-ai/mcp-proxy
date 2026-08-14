//! 依赖（MinerU / MarkItDown）版本兼容性验证与增强依赖报告。

use super::*;

/// Python包信息
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub version: String,
}

/// 包版本兼容性信息
#[derive(Debug, Clone)]
pub struct PackageCompatibility {
    pub package_name: String,
    pub current_version: String,
    pub minimum_version: String,
    pub recommended_version: Option<String>,
    pub is_compatible: bool,
    pub compatibility_issues: Vec<String>,
    pub upgrade_available: bool,
    pub upgrade_recommendation: Option<String>,
}

/// 依赖验证结果
#[derive(Debug, Clone)]
pub struct DependencyVerificationResult {
    pub mineru_status: DependencyStatus,
    pub markitdown_status: DependencyStatus,
    pub overall_compatible: bool,
    pub recommendations: Vec<String>,
    pub critical_issues: Vec<String>,
}

/// 依赖状态
#[derive(Debug, Clone)]
pub struct DependencyStatus {
    pub package_name: String,
    pub is_available: bool,
    pub is_functional: bool,
    pub version_info: Option<PackageInfo>,
    pub compatibility: Option<PackageCompatibility>,
    pub issues: Vec<String>,
    pub path: Option<String>,
}

impl EnvironmentManager {
    /// 获取增强的依赖验证报告
    pub async fn get_enhanced_dependency_report(&self) -> Result<String, AppError> {
        let verification_result = self.verify_dependency_compatibility().await?;

        let mut report = String::new();
        report.push_str("=== 增强依赖验证报告 ===\n\n");

        // 总体状态
        report.push_str(&format!(
            "总体兼容性: {}\n",
            if verification_result.overall_compatible {
                "✓ 兼容"
            } else {
                "✗ 不兼容"
            }
        ));
        report.push('\n');

        // MinerU状态
        report.push_str("=== MinerU 状态 ===\n");
        let mineru = &verification_result.mineru_status;
        report.push_str(&format!(
            "可用性: {}\n",
            if mineru.is_available {
                "✓ 可用"
            } else {
                "✗ 不可用"
            }
        ));
        report.push_str(&format!(
            "功能性: {}\n",
            if mineru.is_functional {
                "✓ 正常"
            } else {
                "✗ 异常"
            }
        ));

        if let Some(ref version_info) = mineru.version_info {
            report.push_str(&format!("版本: {}\n", version_info.version));
        }

        if let Some(ref path) = mineru.path {
            report.push_str(&format!("路径: {path}\n"));
        }

        if let Some(ref compat) = mineru.compatibility {
            report.push_str(&format!(
                "版本兼容性: {}\n",
                if compat.is_compatible {
                    "✓ 兼容"
                } else {
                    "✗ 不兼容"
                }
            ));
            report.push_str(&format!("当前版本: {}\n", compat.current_version));
            report.push_str(&format!("最低要求: {}\n", compat.minimum_version));

            if !compat.compatibility_issues.is_empty() {
                report.push_str("兼容性问题:\n");
                for issue in &compat.compatibility_issues {
                    report.push_str(&format!("  - {issue}\n"));
                }
            }

            if compat.upgrade_available
                && let Some(ref rec) = compat.upgrade_recommendation
            {
                report.push_str(&format!("升级建议: {rec}\n"));
            }
        }

        if !mineru.issues.is_empty() {
            report.push_str("问题:\n");
            for issue in &mineru.issues {
                report.push_str(&format!("  - {issue}\n"));
            }
        }
        report.push('\n');

        // MarkItDown状态
        report.push_str("=== MarkItDown 状态 ===\n");
        let markitdown = &verification_result.markitdown_status;
        report.push_str(&format!(
            "可用性: {}\n",
            if markitdown.is_available {
                "✓ 可用"
            } else {
                "✗ 不可用"
            }
        ));
        report.push_str(&format!(
            "功能性: {}\n",
            if markitdown.is_functional {
                "✓ 正常"
            } else {
                "✗ 异常"
            }
        ));

        if let Some(ref version_info) = markitdown.version_info {
            report.push_str(&format!("版本: {}\n", version_info.version));
        }

        if let Some(ref path) = markitdown.path {
            report.push_str(&format!("路径: {path}\n"));
        }

        if let Some(ref compat) = markitdown.compatibility {
            report.push_str(&format!(
                "版本兼容性: {}\n",
                if compat.is_compatible {
                    "✓ 兼容"
                } else {
                    "✗ 不兼容"
                }
            ));
            report.push_str(&format!("当前版本: {}\n", compat.current_version));
            report.push_str(&format!("最低要求: {}\n", compat.minimum_version));

            if !compat.compatibility_issues.is_empty() {
                report.push_str("兼容性问题:\n");
                for issue in &compat.compatibility_issues {
                    report.push_str(&format!("  - {issue}\n"));
                }
            }

            if compat.upgrade_available
                && let Some(ref rec) = compat.upgrade_recommendation
            {
                report.push_str(&format!("升级建议: {rec}\n"));
            }
        }

        if !markitdown.issues.is_empty() {
            report.push_str("问题:\n");
            for issue in &markitdown.issues {
                report.push_str(&format!("  - {issue}\n"));
            }
        }
        report.push('\n');

        // 关键问题
        if !verification_result.critical_issues.is_empty() {
            report.push_str("=== 关键问题 ===\n");
            for issue in &verification_result.critical_issues {
                report.push_str(&format!("⚠️  {issue}\n"));
            }
            report.push('\n');
        }

        // 推荐操作
        if !verification_result.recommendations.is_empty() {
            report.push_str("=== 推荐操作 ===\n");
            for (i, rec) in verification_result.recommendations.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, rec));
            }
        }

        Ok(report)
    }

    /// 验证依赖版本兼容性
    pub async fn verify_dependency_compatibility(
        &self,
    ) -> Result<DependencyVerificationResult, AppError> {
        debug!("Start relying on version compatibility verification");

        let (mineru_result, markitdown_result) = tokio::join!(
            self.verify_mineru_dependency(),
            self.verify_markitdown_dependency()
        );

        let mineru_status = mineru_result.unwrap_or_else(|e| DependencyStatus {
            package_name: "MinerU".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: vec![e.to_string()],
            path: None,
        });

        let markitdown_status = markitdown_result.unwrap_or_else(|e| DependencyStatus {
            package_name: "MarkItDown".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: vec![e.to_string()],
            path: None,
        });

        let overall_compatible = mineru_status.is_available
            && mineru_status.is_functional
            && markitdown_status.is_available
            && markitdown_status.is_functional
            && mineru_status
                .compatibility
                .as_ref()
                .is_none_or(|c| c.is_compatible)
            && markitdown_status
                .compatibility
                .as_ref()
                .is_none_or(|c| c.is_compatible);

        let mut recommendations = Vec::new();
        let mut critical_issues = Vec::new();

        // 收集推荐和关键问题
        if let Some(ref compat) = mineru_status.compatibility {
            if !compat.is_compatible {
                critical_issues.push(format!(
                    "MinerU版本不兼容: {} (最低要求: {})",
                    compat.current_version, compat.minimum_version
                ));
            }
            if compat.upgrade_available
                && let Some(ref rec) = compat.upgrade_recommendation
            {
                recommendations.push(rec.clone());
            }
        }

        if let Some(ref compat) = markitdown_status.compatibility {
            if !compat.is_compatible {
                critical_issues.push(format!(
                    "MarkItDown版本不兼容: {} (最低要求: {})",
                    compat.current_version, compat.minimum_version
                ));
            }
            if compat.upgrade_available
                && let Some(ref rec) = compat.upgrade_recommendation
            {
                recommendations.push(rec.clone());
            }
        }

        // 添加通用推荐
        if !mineru_status.is_available {
            recommendations.push("安装MinerU: uv pip install -U \"mineru[core]\"".to_string());
        }
        if !markitdown_status.is_available {
            recommendations.push("安装MarkItDown: uv pip install markitdown".to_string());
        }

        Ok(DependencyVerificationResult {
            mineru_status,
            markitdown_status,
            overall_compatible,
            recommendations,
            critical_issues,
        })
    }

    /// 验证MinerU依赖
    async fn verify_mineru_dependency(&self) -> Result<DependencyStatus, AppError> {
        let current_dir = Path::new(&self.base_dir).to_path_buf();
        let venv_path = current_dir.join("venv");
        let mineru_path = Self::get_venv_executable_path(&venv_path, "mineru");

        let mut status = DependencyStatus {
            package_name: "MinerU".to_string(),
            is_available: mineru_path.exists(),
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: Vec::new(),
            path: Some(mineru_path.to_string_lossy().to_string()),
        };

        if !status.is_available {
            status.issues.push("MinerU命令不存在".to_string());
            return Ok(status);
        }

        // 检查功能性
        match self.check_mineru_environment().await {
            Ok(package_info) => {
                status.is_functional = true;
                status.version_info = Some(package_info.clone());

                // 验证版本兼容性
                status.compatibility = Some(
                    self.check_mineru_version_compatibility(&package_info.version)
                        .await,
                );
            }
            Err(e) => {
                status.issues.push(format!("MinerU功能检查失败: {e}"));
            }
        }

        Ok(status)
    }

    /// 验证MarkItDown依赖
    async fn verify_markitdown_dependency(&self) -> Result<DependencyStatus, AppError> {
        let current_dir = Path::new(&self.base_dir).to_path_buf();
        let venv_path = current_dir.join("venv");
        let python_path = Self::get_venv_python_path(&venv_path);

        let mut status = DependencyStatus {
            package_name: "MarkItDown".to_string(),
            is_available: false,
            is_functional: false,
            version_info: None,
            compatibility: None,
            issues: Vec::new(),
            path: Some(python_path.to_string_lossy().to_string()),
        };

        // 检查功能性
        match self.check_markitdown_environment().await {
            Ok(package_info) => {
                status.is_available = true;
                status.is_functional = true;
                status.version_info = Some(package_info.clone());

                // 验证版本兼容性
                status.compatibility = Some(
                    self.check_markitdown_version_compatibility(&package_info.version)
                        .await,
                );
            }
            Err(e) => {
                status.issues.push(format!("MarkItDown检查失败: {e}"));
            }
        }

        Ok(status)
    }

    /// 检查MinerU版本兼容性
    async fn check_mineru_version_compatibility(
        &self,
        current_version: &str,
    ) -> PackageCompatibility {
        let minimum_version = "0.1.0"; // MinerU最低版本要求
        let recommended_version = "latest"; // 推荐版本

        let is_compatible = self.is_version_compatible(current_version, minimum_version);
        let upgrade_available = current_version != "latest" && current_version != "available";

        let mut compatibility_issues = Vec::new();
        let mut upgrade_recommendation = None;

        if !is_compatible {
            compatibility_issues.push(format!(
                "当前版本 {current_version} 低于最低要求版本 {minimum_version}"
            ));
        }

        if upgrade_available {
            upgrade_recommendation =
                Some("升级MinerU到最新版本: uv pip install -U \"mineru[core]\"".to_string());
        }

        // 检查特定版本的已知问题
        if current_version.contains("0.0.") {
            compatibility_issues.push("检测到早期版本，可能存在稳定性问题".to_string());
        }

        PackageCompatibility {
            package_name: "MinerU".to_string(),
            current_version: current_version.to_string(),
            minimum_version: minimum_version.to_string(),
            recommended_version: Some(recommended_version.to_string()),
            is_compatible,
            compatibility_issues,
            upgrade_available,
            upgrade_recommendation,
        }
    }

    /// 检查MarkItDown版本兼容性
    async fn check_markitdown_version_compatibility(
        &self,
        current_version: &str,
    ) -> PackageCompatibility {
        let minimum_version = "0.0.1"; // MarkItDown最低版本要求
        let recommended_version = "latest"; // 推荐版本

        let is_compatible = self.is_version_compatible(current_version, minimum_version);
        let upgrade_available = current_version != "latest" && current_version != "available";

        let mut compatibility_issues = Vec::new();
        let mut upgrade_recommendation = None;

        if !is_compatible {
            compatibility_issues.push(format!(
                "当前版本 {current_version} 低于最低要求版本 {minimum_version}"
            ));
        }

        if upgrade_available {
            upgrade_recommendation =
                Some("升级MarkItDown到最新版本: uv pip install -U markitdown".to_string());
        }

        PackageCompatibility {
            package_name: "MarkItDown".to_string(),
            current_version: current_version.to_string(),
            minimum_version: minimum_version.to_string(),
            recommended_version: Some(recommended_version.to_string()),
            is_compatible,
            compatibility_issues,
            upgrade_available,
            upgrade_recommendation,
        }
    }

    /// 比较版本号兼容性（简单的语义版本比较；解析统一走 parse_version_tuple）
    fn is_version_compatible(&self, current: &str, minimum: &str) -> bool {
        // 处理特殊版本字符串
        if current == "available" || current == "latest" || current == "unknown" {
            return true; // 假设可用
        }

        // 简单的版本比较逻辑
        let (current_parts, min_parts) = match (
            self.parse_version_tuple(current),
            self.parse_version_tuple(minimum),
        ) {
            (Some(current_parts), Some(min_parts)) => (current_parts, min_parts),
            _ => return true, // 无法解析版本时假设兼容
        };

        for (current_part, min_part) in [current_parts.0, current_parts.1, current_parts.2]
            .into_iter()
            .zip([min_parts.0, min_parts.1, min_parts.2])
        {
            if current_part > min_part {
                return true;
            } else if current_part < min_part {
                return false;
            }
        }
        true // 版本相等
    }
}
