use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mcp_proxy_args::{LoadedFallbackJson, RawFallbackImportSpec, try_load_fallback};

use crate::client::protocol::McpProtocol;
use crate::client::support::ConvertArgs;

#[derive(Debug)]
pub(super) enum DiscoveryMode {
    Normal,
    Import(LoadedFallbackJson),
    Export(ExportTargets),
}

#[derive(Debug, Clone)]
pub(super) struct ExportTargets {
    pub initialize: Option<PathBuf>,
    pub tools: Option<PathBuf>,
}

pub(super) fn prepare(
    args: &ConvertArgs,
    config_protocol: Option<&McpProtocol>,
) -> Result<DiscoveryMode> {
    let import_spec = RawFallbackImportSpec {
        initialize_inline: args.import_initialize.clone(),
        initialize_file: args.import_initialize_file.clone(),
        tools_inline: args.import_tools.clone(),
        tools_file: args.import_tools_file.clone(),
    };
    let import_requested = import_spec.has_any();
    let export_requested = args.export_initialize.is_some() || args.export_tools.is_some();
    let protocol_known = args.protocol.is_some()
        || matches!(
            config_protocol,
            Some(McpProtocol::Sse | McpProtocol::Stream)
        );

    validate_mode(
        import_requested,
        export_requested,
        protocol_known,
        args.export_initialize.as_deref(),
        args.export_tools.as_deref(),
    )?;

    if import_requested {
        let loaded = try_load_fallback(&import_spec)?.ok_or_else(|| {
            anyhow::anyhow!("fallback import was requested but no input was loaded")
        })?;
        return Ok(DiscoveryMode::Import(loaded));
    }
    if export_requested {
        return Ok(DiscoveryMode::Export(ExportTargets {
            initialize: args.export_initialize.clone(),
            tools: args.export_tools.clone(),
        }));
    }
    Ok(DiscoveryMode::Normal)
}

fn validate_mode(
    import_requested: bool,
    export_requested: bool,
    protocol_known: bool,
    export_initialize: Option<&Path>,
    export_tools: Option<&Path>,
) -> Result<()> {
    if import_requested && export_requested {
        bail!("fallback import and export options cannot be used together");
    }
    if (import_requested || export_requested) && !protocol_known {
        bail!(
            "fallback import/export requires an explicit SSE or Stream protocol from --protocol or remote service config"
        );
    }
    if export_initialize == Some(Path::new("-")) && export_tools == Some(Path::new("-")) {
        bail!("--export-initialize and --export-tools cannot both write to stdout");
    }
    let initialize_target = export_initialize.map(export_target_identity).transpose()?;
    let tools_target = export_tools.map(export_target_identity).transpose()?;
    if let (Some(initialize), Some(tools)) = (&initialize_target, &tools_target)
        && initialize == tools
    {
        bail!("--export-initialize and --export-tools must use different file targets");
    }
    Ok(())
}

fn export_target_identity(target: &Path) -> Result<std::path::PathBuf> {
    if target == Path::new("-") {
        return Ok(target.to_path_buf());
    }
    let file_name = target
        .file_name()
        .with_context(|| format!("invalid export target {}", target.display()))?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical_parent = parent.canonicalize().with_context(|| {
        format!(
            "failed to resolve export target directory {}",
            parent.display()
        )
    })?;
    Ok(canonical_parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(values: &[&str]) -> ConvertArgs {
        ConvertArgs::parse_from(values)
    }

    #[test]
    fn resolves_discovery_modes_once() {
        assert!(matches!(
            prepare(&args(&["convert"]), None).expect("normal mode"),
            DiscoveryMode::Normal
        ));
        assert!(matches!(
            prepare(
                &args(&[
                    "convert",
                    "--protocol",
                    "sse",
                    "--import-initialize",
                    "{}",
                    "--import-tools",
                    "{}",
                ]),
                None,
            )
            .expect("import mode"),
            DiscoveryMode::Import(_)
        ));
        assert!(matches!(
            prepare(
                &args(&["convert", "--protocol", "stream", "--export-tools", "-",]),
                None,
            )
            .expect("export mode"),
            DiscoveryMode::Export(_)
        ));
    }

    #[test]
    fn import_without_explicit_protocol_fails_before_loading() {
        let error = prepare(
            &args(&[
                "convert",
                "--import-initialize",
                "{invalid",
                "--import-tools",
                "{}",
            ]),
            None,
        )
        .expect_err("protocol validation must happen before JSON loading");

        assert!(error.to_string().contains("explicit SSE or Stream"));
    }

    #[test]
    fn rejects_import_without_known_protocol() {
        let error =
            validate_mode(true, false, false, None, None).expect_err("unknown protocol must fail");
        assert!(
            error
                .to_string()
                .contains("explicit SSE or Stream protocol")
        );
    }

    #[test]
    fn rejects_import_export_conflict_before_loading() {
        let error = validate_mode(true, true, true, None, Some(Path::new("tools.json")))
            .expect_err("import/export conflict must fail");
        assert!(error.to_string().contains("cannot be used together"));
    }

    #[test]
    fn rejects_ambiguous_export_targets() {
        validate_mode(
            false,
            true,
            true,
            Some(Path::new("-")),
            Some(Path::new("-")),
        )
        .expect_err("dual stdout must fail");
        validate_mode(
            false,
            true,
            true,
            Some(Path::new("snapshot.json")),
            Some(Path::new("snapshot.json")),
        )
        .expect_err("same file target must fail");
    }

    #[test]
    fn rejects_equivalent_relative_export_targets() {
        validate_mode(
            false,
            true,
            true,
            Some(Path::new("snapshot.json")),
            Some(Path::new("./snapshot.json")),
        )
        .expect_err("equivalent file targets must fail");
    }
}
