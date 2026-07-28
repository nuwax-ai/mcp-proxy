use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawFallbackImportSpec {
    pub initialize_inline: Option<String>,
    pub initialize_file: Option<PathBuf>,
    pub tools_inline: Option<String>,
    pub tools_file: Option<PathBuf>,
}

impl RawFallbackImportSpec {
    pub fn has_any(&self) -> bool {
        self.initialize_inline.is_some()
            || self.initialize_file.is_some()
            || self.tools_inline.is_some()
            || self.tools_file.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSource {
    Inline,
    File(PathBuf),
}

impl std::fmt::Display for ImportSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inline => formatter.write_str("inline"),
            Self::File(path) => write!(formatter, "{}", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedJson {
    pub json: String,
    pub source: ImportSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedFallbackJson {
    pub initialize: LoadedJson,
    pub tools: LoadedJson,
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("fallback import must include both initialize and tools metadata")]
    IncompletePair,
    #[error("both inline and file sources were provided for {kind}")]
    ConflictingSources { kind: &'static str },
    #[error("failed to read {kind} fallback file {path}: {source}")]
    ReadFile {
        kind: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON syntax for {kind} fallback from {source_name}: {source}")]
    InvalidJsonSyntax {
        kind: &'static str,
        source_name: String,
        #[source]
        source: serde_json::Error,
    },
}

fn load_one(
    kind: &'static str,
    inline: &Option<String>,
    file: &Option<PathBuf>,
) -> Result<Option<LoadedJson>, LoadError> {
    let loaded = match (inline, file) {
        (Some(_), Some(_)) => return Err(LoadError::ConflictingSources { kind }),
        (None, None) => return Ok(None),
        (Some(json), None) => LoadedJson {
            json: json.clone(),
            source: ImportSource::Inline,
        },
        (None, Some(path)) => {
            let json = fs::read_to_string(path).map_err(|source| LoadError::ReadFile {
                kind,
                path: path.clone(),
                source,
            })?;
            LoadedJson {
                json,
                source: ImportSource::File(path.clone()),
            }
        }
    };

    serde_json::from_str::<Value>(&loaded.json).map_err(|source| {
        let source_name = loaded.source.to_string();
        LoadError::InvalidJsonSyntax {
            kind,
            source_name,
            source,
        }
    })?;

    Ok(Some(loaded))
}

pub fn try_load_fallback(
    spec: &RawFallbackImportSpec,
) -> Result<Option<LoadedFallbackJson>, LoadError> {
    if !spec.has_any() {
        return Ok(None);
    }

    let initialize = load_one("initialize", &spec.initialize_inline, &spec.initialize_file)?;
    let tools = load_one("tools", &spec.tools_inline, &spec.tools_file)?;

    match (initialize, tools) {
        (Some(initialize), Some(tools)) => Ok(Some(LoadedFallbackJson { initialize, tools })),
        _ => Err(LoadError::IncompletePair),
    }
}
