# Design Document

## Overview

The UV Python Environment Management system provides automated Python virtual environment setup and dependency management for the document-parser service. The system uses UV (Ultrafast Python package installer) to create isolated Python environments in the current working directory, automatically install MinerU and MarkItDown dependencies, and seamlessly integrate with the document parsing workflow. The design follows a two-phase approach: initialization via `document-parser uv-init` command, followed by manual server startup for HTTP-based document processing services.

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                UV Python Environment Management                 │
├─────────────────────────────────────────────────────────────────┤
│  Command Layer                                                  │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ uv-init Command │ │ check Command   │ │ server Command  │   │
│  │ - Environment   │ │ - Status Query  │ │ - HTTP Service  │   │
│  │ - Dependencies  │ │ - Health Check  │ │ - Auto-detect   │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Environment Management Layer                                   │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ UV Manager      │ │ Dependency Mgr  │ │ Validation Mgr  │   │
│  │ - venv Creation │ │ - MinerU Install│ │ - Health Checks │   │
│  │ - UV Install    │ │ - MarkItDown    │ │ - Version Check │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Execution Layer                                                │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Process Manager │ │ Path Manager    │ │ Progress Track  │   │
│  │ - Command Exec  │ │ - venv/bin/     │ │ - Status Report │   │
│  │ - Error Handle  │ │ - Auto-activate │ │ - Progress UI   │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Integration Layer                                              │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Parser Engines  │ │ System Detect   │ │ Config Manager  │   │
│  │ - MinerU CLI    │ │ - OS Detection  │ │ - Environment   │   │
│  │ - MarkItDown    │ │ - CUDA Check    │ │ - Variables     │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### Environment Setup Flow

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ document-parser │───▶│   uv-init       │───▶│ Environment     │
│ uv-init         │    │   Command       │    │ Validation      │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ Check UV Tool   │───▶│ Create venv     │───▶│ Install MinerU  │
│ Install if      │    │ ./venv/         │    │ & MarkItDown    │
│ Missing         │    │                 │    │                 │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ Manual Server   │◄───│ Ready for Use   │◄───│ Verify Install  │
│ Startup         │    │ ./venv/bin/     │    │ Test Commands   │
│ document-parser │    │ python, mineru  │    │                 │
│ server          │    │                 │    │                 │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

## Components and Interfaces

### 1. Command Layer

#### UV Init Command Handler
- **Location**: `src/main.rs` - `handle_uv_init_command()`
- **Purpose**: Orchestrate complete environment setup process
- **Key Responsibilities**:
  - Check current working directory
  - Validate system prerequisites
  - Coordinate UV installation and virtual environment creation
  - Install and verify parsing engine dependencies
  - Provide user feedback and next steps

```rust
async fn handle_uv_init_command(environment_manager: &EnvironmentManager) -> Result<()> {
    // 1. System validation
    // 2. UV tool management
    // 3. Virtual environment creation
    // 4. Dependency installation
    // 5. Verification and reporting
}
```

#### Environment Check Command
- **Location**: `src/main.rs` - `handle_check_command()`
- **Purpose**: Comprehensive environment status reporting
- **Enhanced Features**:
  - Virtual environment path detection
  - Dependency version reporting
  - CUDA capability assessment
  - Installation recommendations

### 2. Environment Management Layer

#### Enhanced Environment Manager
- **Location**: `src/utils/environment_manager.rs`
- **Key Enhancements**:

```rust
impl EnvironmentManager {
    /// Create environment manager for current working directory
    pub fn for_current_directory() -> Result<Self, AppError> {
        let current_dir = std::env::current_dir()?;
        let venv_path = current_dir.join("venv");
        let python_path = if cfg!(windows) {
            venv_path.join("Scripts").join("python.exe")
        } else {
            venv_path.join("bin").join("python")
        };
        
        Ok(Self::new(
            python_path.to_string_lossy().to_string(),
            current_dir.to_string_lossy().to_string(),
        ))
    }
    
    /// Check and install UV tool if missing
    pub async fn ensure_uv_available(&self) -> Result<(), AppError> {
        // Check if uv is installed
        // If not, provide installation instructions or auto-install
    }
    
    /// Create virtual environment using UV
    pub async fn create_uv_virtual_environment(&self) -> Result<(), AppError> {
        // Execute: uv venv venv
        // Verify creation success
        // Set up activation scripts
    }
    
    /// Install MinerU using UV in virtual environment
    pub async fn install_mineru_with_uv(&self) -> Result<(), AppError> {
        // Execute: uv pip install -U "mineru[core]"
        // Handle China region model source configuration
        // Verify mineru command availability
    }
    
    /// Install MarkItDown using UV in virtual environment
    pub async fn install_markitdown_with_uv(&self) -> Result<(), AppError> {
        // Execute: uv pip install markitdown
        // Verify markitdown module availability
    }
    
    /// Comprehensive environment validation
    pub async fn validate_complete_environment(&self) -> Result<EnvironmentStatus, AppError> {
        // Check all components and their integration
        // Return detailed status with actionable feedback
    }
}
```

#### UV Tool Manager
- **Purpose**: Manage UV tool installation and operations
- **Key Features**:

```rust
pub struct UvToolManager {
    installation_path: Option<PathBuf>,
    version: Option<String>,
}

impl UvToolManager {
    pub async fn check_installation(&self) -> Result<bool, AppError>;
    pub async fn install_uv(&self) -> Result<(), AppError>;
    pub async fn create_virtual_environment(&self, path: &Path) -> Result<(), AppError>;
    pub async fn install_package(&self, venv_path: &Path, package: &str) -> Result<(), AppError>;
    pub async fn get_version(&self) -> Result<String, AppError>;
}
```

### 3. Execution Layer

#### Process Manager
- **Purpose**: Handle external command execution with proper error handling
- **Key Features**:

```rust
pub struct ProcessManager {
    timeout_duration: Duration,
    retry_config: RetryConfig,
}

impl ProcessManager {
    pub async fn execute_command(&self, cmd: &mut Command) -> Result<CommandOutput, AppError>;
    pub async fn execute_with_progress<F>(&self, cmd: &mut Command, progress_callback: F) -> Result<CommandOutput, AppError>
    where F: Fn(String) + Send + Sync;
    pub async fn execute_in_venv(&self, venv_path: &Path, cmd: &mut Command) -> Result<CommandOutput, AppError>;
}
```

#### Path Manager
- **Purpose**: Manage virtual environment paths and activation
- **Key Features**:

```rust
pub struct PathManager {
    base_directory: PathBuf,
    venv_directory: PathBuf,
}

impl PathManager {
    pub fn new(base_dir: PathBuf) -> Self;
    pub fn get_python_executable(&self) -> PathBuf;
    pub fn get_mineru_executable(&self) -> PathBuf;
    pub fn get_activation_script(&self) -> PathBuf;
    pub fn is_virtual_environment_active(&self) -> bool;
    pub fn get_environment_variables(&self) -> HashMap<String, String>;
}
```

### 4. Integration Layer

#### Parser Engine Integration
- **Enhanced MinerU Integration**:

```rust
impl MinerUParser {
    /// Create parser with automatic virtual environment detection
    pub fn with_auto_venv_detection() -> Result<Self, AppError> {
        let path_manager = PathManager::new(std::env::current_dir()?);
        let python_path = path_manager.get_python_executable();
        let mineru_path = path_manager.get_mineru_executable();
        
        let config = MinerUConfig {
            python_path: python_path.to_string_lossy().to_string(),
            mineru_command_path: mineru_path.to_string_lossy().to_string(),
            // ... other config
        };
        
        Ok(Self::new(config))
    }
    
    /// Execute MinerU command in virtual environment
    async fn execute_mineru_in_venv(&self, args: &[&str]) -> Result<CommandOutput, AppError> {
        let path_manager = PathManager::new(std::env::current_dir()?);
        let mut cmd = Command::new(path_manager.get_mineru_executable());
        cmd.args(args);
        
        // Set virtual environment variables
        for (key, value) in path_manager.get_environment_variables() {
            cmd.env(key, value);
        }
        
        self.process_manager.execute_command(&mut cmd).await
    }
}
```

- **Enhanced MarkItDown Integration**:

```rust
impl MarkItDownParser {
    /// Create parser with virtual environment Python
    pub fn with_venv_python() -> Result<Self, AppError> {
        let path_manager = PathManager::new(std::env::current_dir()?);
        let python_path = path_manager.get_python_executable();
        
        let config = MarkItDownConfig {
            python_path: python_path.to_string_lossy().to_string(),
            // ... other config
        };
        
        Ok(Self::new(config))
    }
    
    /// Execute MarkItDown in virtual environment
    async fn execute_markitdown_in_venv(&self, file_path: &str) -> Result<String, AppError> {
        let path_manager = PathManager::new(std::env::current_dir()?);
        let mut cmd = Command::new(path_manager.get_python_executable());
        cmd.args(&["-m", "markitdown", file_path]);
        
        // Set virtual environment variables
        for (key, value) in path_manager.get_environment_variables() {
            cmd.env(key, value);
        }
        
        let output = self.process_manager.execute_command(&mut cmd).await?;
        Ok(output.stdout)
    }
}
```

## Data Models

### Enhanced Environment Status

```rust
#[derive(Debug, Clone)]
pub struct EnvironmentStatus {
    // Existing fields...
    pub current_directory: PathBuf,
    pub virtual_environment_path: Option<PathBuf>,
    pub virtual_environment_active: bool,
    pub uv_tool_available: bool,
    pub uv_version: Option<String>,
    pub mineru_command_path: Option<PathBuf>,
    pub markitdown_module_available: bool,
    pub installation_recommendations: Vec<InstallationRecommendation>,
    pub activation_instructions: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstallationRecommendation {
    pub component: String,
    pub action: RecommendedAction,
    pub command: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum RecommendedAction {
    Install,
    Update,
    Reinstall,
    Configure,
    Verify,
}
```

### UV Configuration

```rust
#[derive(Debug, Clone)]
pub struct UvConfig {
    pub installation_method: UvInstallationMethod,
    pub virtual_environment_name: String,
    pub python_version: Option<String>,
    pub index_url: Option<String>,
    pub extra_index_urls: Vec<String>,
    pub trusted_hosts: Vec<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum UvInstallationMethod {
    SystemPackageManager,
    CurlScript,
    PipInstall,
    Manual,
}
```

## Error Handling

### UV-Specific Error Types

```rust
#[derive(Error, Debug)]
pub enum UvError {
    #[error("UV工具未安装: {message}")]
    UvNotInstalled { message: String },
    
    #[error("虚拟环境创建失败: {path} - {error}")]
    VirtualEnvironmentCreationFailed { path: String, error: String },
    
    #[error("依赖安装失败: {package} - {error}")]
    DependencyInstallationFailed { package: String, error: String },
    
    #[error("虚拟环境激活失败: {path}")]
    VirtualEnvironmentActivationFailed { path: String },
    
    #[error("命令执行失败: {command} - {error}")]
    CommandExecutionFailed { command: String, error: String },
    
    #[error("环境验证失败: {component} - {issue}")]
    EnvironmentValidationFailed { component: String, issue: String },
}
```

### Error Recovery Strategies

```rust
impl UvError {
    pub fn get_recovery_suggestions(&self) -> Vec<String> {
        match self {
            UvError::UvNotInstalled { .. } => vec![
                "运行以下命令安装UV: curl -LsSf https://astral.sh/uv/install.sh | sh".to_string(),
                "或使用pip安装: pip install uv".to_string(),
                "重新运行 document-parser uv-init".to_string(),
            ],
            UvError::VirtualEnvironmentCreationFailed { path, .. } => vec![
                format!("检查目录权限: {}", path),
                "清理现有虚拟环境目录".to_string(),
                "确保有足够的磁盘空间".to_string(),
            ],
            // ... other error recovery suggestions
        }
    }
}
```

## Testing Strategy

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_uv_virtual_environment_creation() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::for_directory(temp_dir.path()).unwrap();
        
        let result = env_manager.create_uv_virtual_environment().await;
        assert!(result.is_ok());
        
        let venv_path = temp_dir.path().join("venv");
        assert!(venv_path.exists());
        assert!(venv_path.join("bin").join("python").exists() || 
                venv_path.join("Scripts").join("python.exe").exists());
    }
    
    #[tokio::test]
    async fn test_dependency_installation() {
        // Test MinerU and MarkItDown installation in isolated environment
    }
    
    #[tokio::test]
    async fn test_environment_validation() {
        // Test comprehensive environment status checking
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_complete_uv_init_workflow() {
    let temp_dir = TempDir::new().unwrap();
    std::env::set_current_dir(temp_dir.path()).unwrap();
    
    // Simulate complete uv-init process
    let env_manager = EnvironmentManager::for_current_directory().unwrap();
    let result = handle_uv_init_command(&env_manager).await;
    
    assert!(result.is_ok());
    
    // Verify environment is ready
    let status = env_manager.check_environment().await.unwrap();
    assert!(status.is_ready());
    assert!(status.mineru_available);
    assert!(status.markitdown_available);
}
```

## Performance Optimization

### Caching Strategy

```rust
pub struct InstallationCache {
    cache_directory: PathBuf,
    package_cache: HashMap<String, CachedPackage>,
}

impl InstallationCache {
    pub async fn get_cached_package(&self, package: &str) -> Option<PathBuf>;
    pub async fn cache_package(&self, package: &str, path: PathBuf) -> Result<(), AppError>;
    pub async fn is_cache_valid(&self, package: &str) -> bool;
}
```

### Parallel Installation

```rust
pub async fn install_dependencies_parallel(&self) -> Result<(), AppError> {
    let (mineru_result, markitdown_result) = tokio::join!(
        self.install_mineru_with_uv(),
        self.install_markitdown_with_uv()
    );
    
    mineru_result?;
    markitdown_result?;
    
    Ok(())
}
```

## Security Considerations

### Command Injection Prevention

```rust
fn sanitize_command_args(args: &[String]) -> Result<Vec<String>, AppError> {
    for arg in args {
        if arg.contains(';') || arg.contains('|') || arg.contains('&') {
            return Err(AppError::Security("Potentially dangerous command argument".to_string()));
        }
    }
    Ok(args.to_vec())
}
```

### File System Security

```rust
fn validate_installation_path(path: &Path) -> Result<(), AppError> {
    // Ensure path is within expected boundaries
    // Check for symlink attacks
    // Validate permissions
    Ok(())
}
```

## Deployment Considerations

### Container Support

```rust
impl EnvironmentManager {
    pub fn is_container_environment(&self) -> bool {
        std::env::var("CONTAINER").is_ok() || 
        Path::new("/.dockerenv").exists()
    }
    
    pub async fn setup_container_environment(&self) -> Result<(), AppError> {
        // Handle container-specific setup requirements
        // Manage volume mounts and permissions
        // Configure for read-only filesystems
    }
}
```

### CI/CD Integration

```rust
impl EnvironmentManager {
    pub async fn setup_ci_environment(&self) -> Result<(), AppError> {
        // Non-interactive installation mode
        // Optimized for build environments
        // Caching for faster builds
    }
}
```

This design provides a comprehensive, robust foundation for UV-based Python environment management that integrates seamlessly with the existing document-parser architecture while providing an excellent developer experience through automated setup and intelligent error handling.