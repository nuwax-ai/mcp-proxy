use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::discovery_options::ExportTargets;

struct StagedFile {
    target: PathBuf,
    temporary: tempfile::NamedTempFile,
}

pub fn publish_discovery(
    targets: &ExportTargets,
    initialize_json: &str,
    tools_json: &str,
) -> Result<()> {
    let mut stdout_json = None;
    let mut staged = Vec::new();

    if let Some(target) = &targets.initialize {
        stage_output(target, initialize_json, &mut stdout_json, &mut staged)?;
    }
    if let Some(target) = &targets.tools {
        stage_output(target, tools_json, &mut stdout_json, &mut staged)?;
    }

    let mut published = Vec::new();
    for staged_file in staged {
        let target = staged_file.target.clone();
        if let Err(error) = staged_file.temporary.persist(&target) {
            let published = published
                .iter()
                .map(|path: &PathBuf| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "failed to publish export {}: {}; already published: [{}]",
                target.display(),
                error.error,
                published
            );
        }
        published.push(target);
    }

    if let Some(json) = stdout_json {
        println!("{json}");
    }
    Ok(())
}

fn stage_output(
    target: &Path,
    json: &str,
    stdout_json: &mut Option<String>,
    staged: &mut Vec<StagedFile>,
) -> Result<()> {
    let json = format!("{}\n", json.trim_end());
    if target == Path::new("-") {
        if stdout_json.replace(json.trim_end().to_string()).is_some() {
            bail!("--export-initialize and --export-tools cannot both write to stdout");
        }
        return Ok(());
    }

    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create export temporary file in {}",
            parent.display()
        )
    })?;
    temporary.write_all(json.as_bytes()).with_context(|| {
        format!(
            "failed to write export temporary file for {}",
            target.display()
        )
    })?;
    temporary.as_file().sync_all().with_context(|| {
        format!(
            "failed to sync export temporary file for {}",
            target.display()
        )
    })?;
    staged.push(StagedFile {
        target: target.to_path_buf(),
        temporary,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrites_existing_files_with_trailing_newline() {
        let directory = tempfile::tempdir().expect("tempdir");
        let initialize = directory.path().join("initialize.json");
        std::fs::write(&initialize, "old").expect("seed target");

        publish_discovery(
            &ExportTargets {
                initialize: Some(initialize.clone()),
                tools: None,
            },
            r#"{"server":"new"}"#,
            r#"{"tools":[]}"#,
        )
        .expect("publish initialize");

        assert_eq!(
            std::fs::read_to_string(initialize).expect("read initialize"),
            "{\"server\":\"new\"}\n"
        );
    }

    #[test]
    fn stages_both_outputs_before_publishing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let initialize = directory.path().join("initialize.json");
        let tools = directory.path().join("tools.json");

        publish_discovery(
            &ExportTargets {
                initialize: Some(initialize.clone()),
                tools: Some(tools.clone()),
            },
            "{}",
            r#"{"tools":[]}"#,
        )
        .expect("publish both files");

        assert_eq!(
            std::fs::read_to_string(initialize).expect("read initialize"),
            "{}\n"
        );
        assert_eq!(
            std::fs::read_to_string(tools).expect("read tools"),
            "{\"tools\":[]}\n"
        );
    }

    #[test]
    fn partial_publish_error_lists_already_published_targets() {
        let directory = tempfile::tempdir().expect("tempdir");
        let initialize = directory.path().join("initialize.json");
        let invalid_tools_target = directory.path().join("tools-target");
        std::fs::create_dir(&invalid_tools_target).expect("create invalid target directory");

        let error = publish_discovery(
            &ExportTargets {
                initialize: Some(initialize.clone()),
                tools: Some(invalid_tools_target),
            },
            "{}",
            r#"{"tools":[]}"#,
        )
        .expect_err("second publish must fail");

        assert!(
            error
                .to_string()
                .contains(&initialize.display().to_string())
        );
        assert_eq!(
            std::fs::read_to_string(initialize).expect("read published initialize"),
            "{}\n"
        );
    }
}
