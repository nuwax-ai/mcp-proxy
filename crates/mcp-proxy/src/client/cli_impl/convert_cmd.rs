//! Convert command configuration resolution and dispatch.

use anyhow::Result;

use crate::client::core::{UrlModeTarget, run_command_mode, run_url_mode_with_retry};
use crate::client::support::{
    ConvertArgs, McpConfigSource, init_logging, merge_headers_checked, parse_convert_config,
};
use crate::proxy::ToolFilter;

enum ResolvedConvertTarget {
    Remote(UrlModeTarget),
    Local {
        name: String,
        command: String,
        args: Vec<String>,
        env: std::collections::HashMap<String, String>,
    },
}

pub async fn run_convert_command(args: ConvertArgs, verbose: bool, quiet: bool) -> Result<()> {
    let tool_filter = build_tool_filter(&args)?;
    let config_source = parse_convert_config(&args)?;
    let mcp_name = config_name(&config_source);

    init_logging(&args, mcp_name, quiet, verbose)?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        diagnostic = args.logging.diagnostic,
        "Starting convert command"
    );

    match resolve_target(config_source, &args, quiet)? {
        ResolvedConvertTarget::Remote(target) => {
            run_url_mode_with_retry(args, target, tool_filter, verbose, quiet).await
        }
        ResolvedConvertTarget::Local {
            name,
            command,
            args: command_args,
            env,
        } => {
            warn_ignored_local_import(&args);
            if args.export_initialize.is_some() || args.export_tools.is_some() {
                anyhow::bail!("fallback metadata export is only supported for URL MCP services");
            }
            run_command_mode(&name, &command, command_args, env, tool_filter, quiet).await
        }
    }
}

fn build_tool_filter(args: &ConvertArgs) -> Result<ToolFilter> {
    match (&args.allow_tools, &args.deny_tools) {
        (Some(_), Some(_)) => {
            anyhow::bail!(
                "--allow-tools and --deny-tools cannot be used together, please choose only one"
            )
        }
        (Some(tools), None) => Ok(ToolFilter::allow(tools.clone())),
        (None, Some(tools)) => Ok(ToolFilter::deny(tools.clone())),
        (None, None) => Ok(ToolFilter::default()),
    }
}

fn config_name(config: &McpConfigSource) -> Option<&str> {
    match config {
        McpConfigSource::RemoteService { name, .. }
        | McpConfigSource::LocalCommand { name, .. } => Some(name),
        McpConfigSource::DirectUrl { .. } => None,
    }
}

fn resolve_target(
    config: McpConfigSource,
    args: &ConvertArgs,
    quiet: bool,
) -> Result<ResolvedConvertTarget> {
    match config {
        McpConfigSource::DirectUrl { url } => {
            let headers = merge_headers_checked(
                std::collections::HashMap::new(),
                &args.header,
                args.auth.as_ref(),
            )?;
            Ok(ResolvedConvertTarget::Remote(UrlModeTarget {
                url,
                headers,
                protocol: None,
                timeout_secs: None,
            }))
        }
        McpConfigSource::RemoteService {
            name,
            url,
            protocol,
            headers,
            timeout,
        } => {
            if !quiet {
                eprintln!("🚀 MCP-Stdio-Proxy: {name} ({url}) → stdio");
            }
            Ok(ResolvedConvertTarget::Remote(UrlModeTarget {
                url,
                headers: merge_headers_checked(headers, &args.header, args.auth.as_ref())?,
                protocol,
                timeout_secs: timeout,
            }))
        }
        McpConfigSource::LocalCommand {
            name,
            command,
            args,
            env,
        } => Ok(ResolvedConvertTarget::Local {
            name,
            command,
            args,
            env,
        }),
    }
}

fn warn_ignored_local_import(args: &ConvertArgs) {
    let has_import = args.import_initialize.is_some()
        || args.import_initialize_file.is_some()
        || args.import_tools.is_some()
        || args.import_tools_file.is_some();
    if has_import {
        tracing::warn!(
            "fallback import options are ignored because the resolved MCP is a local command"
        );
    }
}
