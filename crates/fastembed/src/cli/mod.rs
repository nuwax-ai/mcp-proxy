use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod models;

/// FastEmbed - 文本向量化服务
#[derive(Parser, Debug)]
#[command(name = "fastembed")]
#[command(about = "FastEmbed 文本向量化服务", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动 HTTP 服务
    Server(ServerArgs),

    /// 模型管理
    Models(ModelsCmd),
}

/// HTTP 服务启动参数
#[derive(Parser, Debug)]
pub struct ServerArgs {
    /// 监听端口（未指定时取配置文件 / FASTEMBED_PORT / 默认 8068）
    #[arg(short, long)]
    pub port: Option<u16>,

    /// 配置文件路径
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// 模型包下载 URL（覆盖配置文件 / FASTEMBED_MODEL_URL）
    #[arg(long)]
    pub model_url: Option<String>,

    /// 缓存目录（覆盖配置文件 / FASTEMBED_CACHE_DIR）
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}

/// 模型管理子命令
#[derive(Parser, Debug)]
pub struct ModelsCmd {
    #[command(subcommand)]
    pub command: ModelsSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ModelsSubcommand {
    /// 下载模型到本地缓存（从 HuggingFace）
    Download(DownloadArgs),

    /// 列出已下载的模型
    List(ListArgs),

    /// 从 HTTP(S) URL 拉取模型包（.tar.gz）到缓存目录
    Pull(PullArgs),
}

/// 模型下载参数
#[derive(Parser, Debug)]
pub struct DownloadArgs {
    /// 模型类型: text | image | sparse
    #[arg(long, default_value = "text")]
    pub r#type: String,

    /// 内置模型变体名，如 BGELargeZHV15
    #[arg(long)]
    pub model: Option<String>,

    /// Hugging Face 模型代码，如 Xenova/bge-large-zh-v1.5
    #[arg(long)]
    pub code: Option<String>,

    /// BYO 模式：ONNX 文件名（暂未实现，传了会报错）
    #[arg(long)]
    pub onnx: Option<String>,

    /// BYO 模式：Tokenizer 文件名（暂未实现，传了会报错）
    #[arg(long)]
    pub tokenizer: Option<String>,

    /// BYO 模式：Config 文件名（暂未实现，传了会报错）
    #[arg(long)]
    pub config: Option<String>,

    /// BYO 模式：Special tokens map 文件名（暂未实现，传了会报错）
    #[arg(long, alias = "special_tokens")]
    pub special_tokens_map: Option<String>,

    /// BYO 模式：Tokenizer config 文件名（暂未实现，传了会报错）
    #[arg(long)]
    pub tokenizer_config: Option<String>,

    /// 缓存目录
    #[arg(long, default_value = ".fastembed_cache")]
    pub cache_dir: PathBuf,

    /// 显示下载进度（传 --progress=false 关闭）
    #[arg(long, default_value_t = true)]
    pub progress: bool,
}

/// 模型列表参数
#[derive(Parser, Debug)]
pub struct ListArgs {
    /// 模型类型筛选: text | image | sparse
    #[arg(long, default_value = "text")]
    pub r#type: String,

    /// 缓存目录
    #[arg(long, default_value = ".fastembed_cache")]
    pub cache_dir: PathBuf,
}

/// 从 URL 拉取模型包参数
#[derive(Parser, Debug)]
pub struct PullArgs {
    /// 模型包下载地址（http/https）
    #[arg(long)]
    pub url: String,

    /// 缓存目录
    #[arg(long, default_value = ".fastembed_cache")]
    pub cache_dir: PathBuf,
}
