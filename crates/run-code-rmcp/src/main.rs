use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use log::{error, info};
use serde_json::Value;
use std::{fs, path::PathBuf};

use run_code_rmcp::CodeFileCache;
use run_code_rmcp::{CodeExecutor, CodeScriptExecutionResult, LanguageScript};

#[derive(Parser)]
#[command(name = "run_code_rmcp")]
#[command(about = "Execute JavaScript and Python code using MCP SDK", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Show execution logs
    #[arg(short, long)]
    show_logs: bool,

    /// Use MCP SDK integration
    #[cfg(feature = "mcp")]
    #[arg(short, long)]
    use_mcp: bool,

    /// Clear cache before execution
    #[arg(short = 'c', long)]
    clear_cache: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute JavaScript code
    Js(CodeArgs),

    /// Execute TypeScript code
    Ts(CodeArgs),

    /// Execute Python code
    Python(CodeArgs),

    /// Clear cache files
    ClearCache {
        /// Language to clear cache for (js, ts, python, or all)
        #[arg(short, long)]
        language: String,
    },
}

#[derive(Args)]
struct CodeArgs {
    /// Path to the code file
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Code content as string
    #[arg(short, long)]
    code: Option<String>,

    /// Parameters to pass to the script (JSON format)
    #[arg(short, long)]
    params: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 解析命令行参数
    let cli = Cli::parse();

    // 初始化日志
    let mut builder = env_logger::Builder::from_default_env();
    if cli.show_logs {
        // 如果--show-logs参数为true，则至少显示info级别的日志
        builder.filter_level(log::LevelFilter::Info);
    }
    builder.init();

    if let Commands::ClearCache { language } = &cli.command {
        match language.to_lowercase().as_str() {
            "js" => {
                CodeFileCache::clear_cache_by_language(&LanguageScript::Js).await?;
                info!("已清除 JavaScript 缓存");
            }
            "ts" => {
                CodeFileCache::clear_cache_by_language(&LanguageScript::Ts).await?;
                info!("已清除 TypeScript 缓存");
            }
            "python" => {
                CodeFileCache::clear_cache_by_language(&LanguageScript::Python).await?;
                info!("已清除 Python 缓存");
            }
            "all" => {
                CodeFileCache::clear_all_cache().await?;
                info!("已清除所有缓存");
            }
            _ => {
                info!("无效的语言类型，可选项: js, ts, python, all");
                return Ok(());
            }
        }
        return Ok(());
    }

    // 解析传递给脚本的参数
    let params = match &cli.command {
        Commands::Js(args) => parse_params(&args.params)?,
        Commands::Ts(args) => parse_params(&args.params)?,
        Commands::Python(args) => parse_params(&args.params)?,
        _ => None,
    };

    // 从参数获取代码
    let (code, language) = match &cli.command {
        Commands::Js(args) => (get_code(args)?, LanguageScript::Js),
        Commands::Ts(args) => (get_code(args)?, LanguageScript::Ts),
        Commands::Python(args) => (get_code(args)?, LanguageScript::Python),
        _ => unreachable!(),
    };

    // 如果指定了清除缓存选项，则清除对应语言的缓存
    if cli.clear_cache {
        CodeFileCache::clear_cache_by_language(&language).await?;
        info!("已清除 {language:?} 缓存");
    }

    // 执行代码
    #[cfg(feature = "mcp")]
    let result = if cli.use_mcp {
        // 使用MCP SDK集成
        CodeExecutor::execute_with_params_compat(&code, language, params).await?
    } else {
        // 直接执行
        CodeExecutor::execute_with_params_compat(&code, language, params).await?
    };

    #[cfg(not(feature = "mcp"))]
    let result = CodeExecutor::execute_with_params_compat(&code, language, params).await?;

    // 打印结果
    print_result(result);

    Ok(())
}

fn get_code(args: &CodeArgs) -> Result<String> {
    if let Some(file) = &args.file {
        fs::read_to_string(file).context("Failed to read code file")
    } else if let Some(code) = &args.code {
        Ok(code.clone())
    } else {
        anyhow::bail!("Either file or code must be provided")
    }
}

fn parse_params(params_str: &Option<String>) -> Result<Option<Value>> {
    match params_str {
        Some(s) if !s.is_empty() => {
            let parsed: Value =
                serde_json::from_str(s).context("Failed to parse parameters as JSON")?;
            Ok(Some(parsed))
        }
        _ => Ok(None),
    }
}

fn print_result(result: CodeScriptExecutionResult) {
    if !result.logs.is_empty() {
        info!("--- Logs ---");
        for log in result.logs {
            info!("{log}");
        }
        info!("------------");
    }

    if let Some(result_val) = result.result {
        // 使用 serde_json 序列化结果，确保所有类型都能正确显示
        match serde_json::to_string_pretty(&result_val) {
            Ok(json_str) => info!("Result: {json_str}"),
            Err(_) => info!("Result: {result_val}"),
        }
    }

    if let Some(error) = result.error {
        error!("Error: {error}");
    }
}
