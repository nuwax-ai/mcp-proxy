use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use thiserror::Error;

use crate::cache::{CacheError, write_content_addressed};
use crate::flags::{
    IMPORT_INITIALIZE, IMPORT_INITIALIZE_FILE, IMPORT_TOOLS, IMPORT_TOOLS_FILE, long_flag,
};
use crate::load::{LoadError, RawFallbackImportSpec, try_load_fallback};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteOptions {
    pub cache_dir: PathBuf,
    pub file_prefix: String,
}

impl RewriteOptions {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
            file_prefix: "mcp-proxy-import".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResult {
    pub args: Vec<String>,
    pub created_files: Vec<PathBuf>,
    pub changed: bool,
}

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("flag --{flag} is missing its value")]
    MissingValue { flag: &'static str },
    #[error("flag --{flag} was provided more than once")]
    DuplicateFlag { flag: &'static str },
    #[error("fallback import must include both initialize and tools metadata")]
    IncompletePair,
    #[error("both inline and file sources were provided for {kind}")]
    ConflictingSources { kind: &'static str },
    #[error("cache path is not valid UTF-8: {path}")]
    NonUtf8CachePath { path: PathBuf },
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Cache(#[from] CacheError),
}

#[derive(Debug, Clone)]
struct ParsedValue {
    flag_index: usize,
    value_index: Option<usize>,
    value: String,
}

#[derive(Debug, Default)]
struct ParsedImports {
    initialize_inline: Option<ParsedValue>,
    initialize_file: Option<ParsedValue>,
    tools_inline: Option<ParsedValue>,
    tools_file: Option<ParsedValue>,
}

impl ParsedImports {
    fn insert(
        slot: &mut Option<ParsedValue>,
        value: ParsedValue,
        flag: &'static str,
    ) -> Result<(), RewriteError> {
        if slot.is_some() {
            return Err(RewriteError::DuplicateFlag { flag });
        }
        *slot = Some(value);
        Ok(())
    }

    fn validate(&self) -> Result<(), RewriteError> {
        if self.initialize_inline.is_some() && self.initialize_file.is_some() {
            return Err(RewriteError::ConflictingSources { kind: "initialize" });
        }
        if self.tools_inline.is_some() && self.tools_file.is_some() {
            return Err(RewriteError::ConflictingSources { kind: "tools" });
        }

        let has_initialize = self.initialize_inline.is_some() || self.initialize_file.is_some();
        let has_tools = self.tools_inline.is_some() || self.tools_file.is_some();
        if has_initialize != has_tools {
            return Err(RewriteError::IncompletePair);
        }
        Ok(())
    }
}

fn executable_name(command: &str) -> Option<&str> {
    command
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

pub fn is_mcp_proxy_convert(command: &str, args: &[String]) -> bool {
    if !matches!(
        executable_name(command),
        Some("mcp-proxy" | "mcp-proxy.exe")
    ) {
        return false;
    }

    for arg in args {
        if arg == "--" {
            return false;
        }
        if matches!(arg.as_str(), "-v" | "--verbose" | "-q" | "--quiet")
            || (arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.len() > 1
                && arg[1..].chars().all(|ch| matches!(ch, 'v' | 'q')))
        {
            continue;
        }
        return arg == "convert";
    }
    false
}

fn recognized_flag(arg: &str) -> Option<(&'static str, Option<&str>)> {
    for name in [
        IMPORT_INITIALIZE,
        IMPORT_INITIALIZE_FILE,
        IMPORT_TOOLS,
        IMPORT_TOOLS_FILE,
    ] {
        let full = long_flag(name);
        if arg == full {
            return Some((name, None));
        }
        if let Some(value) = arg.strip_prefix(&format!("{full}=")) {
            return Some((name, Some(value)));
        }
    }
    None
}

fn parse_imports(args: &[String]) -> Result<ParsedImports, RewriteError> {
    let mut parsed = ParsedImports::default();
    let mut index = 0;

    while index < args.len() {
        if args[index] == "--" {
            break;
        }
        let Some((flag, inline_value)) = recognized_flag(&args[index]) else {
            index += 1;
            continue;
        };

        let (value, value_index) = if let Some(value) = inline_value {
            (value.to_string(), None)
        } else {
            let next = index + 1;
            let value = args
                .get(next)
                .ok_or(RewriteError::MissingValue { flag })?
                .clone();
            if recognized_flag(&value).is_some() || value == "--" {
                return Err(RewriteError::MissingValue { flag });
            }
            (value, Some(next))
        };
        let parsed_value = ParsedValue {
            flag_index: index,
            value_index,
            value,
        };

        match flag {
            IMPORT_INITIALIZE => ParsedImports::insert(
                &mut parsed.initialize_inline,
                parsed_value,
                IMPORT_INITIALIZE,
            )?,
            IMPORT_INITIALIZE_FILE => ParsedImports::insert(
                &mut parsed.initialize_file,
                parsed_value,
                IMPORT_INITIALIZE_FILE,
            )?,
            IMPORT_TOOLS => {
                ParsedImports::insert(&mut parsed.tools_inline, parsed_value, IMPORT_TOOLS)?
            }
            IMPORT_TOOLS_FILE => {
                ParsedImports::insert(&mut parsed.tools_file, parsed_value, IMPORT_TOOLS_FILE)?
            }
            _ => {}
        }

        index = value_index.map_or(index + 1, |value_index| value_index + 1);
    }

    parsed.validate()?;
    Ok(parsed)
}

pub fn rewrite_convert_import_args_to_files(
    command: &str,
    args: &[String],
    options: &RewriteOptions,
) -> Result<RewriteResult, RewriteError> {
    if !is_mcp_proxy_convert(command, args) {
        return Ok(RewriteResult {
            args: args.to_vec(),
            created_files: Vec::new(),
            changed: false,
        });
    }

    let parsed = parse_imports(args)?;
    let has_any = parsed.initialize_inline.is_some()
        || parsed.initialize_file.is_some()
        || parsed.tools_inline.is_some()
        || parsed.tools_file.is_some();
    if !has_any || (parsed.initialize_inline.is_none() && parsed.tools_inline.is_none()) {
        validate_sources(&parsed)?;
        return Ok(RewriteResult {
            args: args.to_vec(),
            created_files: Vec::new(),
            changed: false,
        });
    }

    validate_sources(&parsed)?;
    if options.cache_dir.to_str().is_none() {
        return Err(RewriteError::NonUtf8CachePath {
            path: options.cache_dir.clone(),
        });
    }

    let mut replacements: HashMap<usize, Vec<String>> = HashMap::new();
    let mut skipped_indexes = HashSet::new();
    let mut created_files = Vec::new();

    for (kind, file_flag, parsed_value) in [
        (
            "initialize",
            IMPORT_INITIALIZE_FILE,
            parsed.initialize_inline.as_ref(),
        ),
        ("tools", IMPORT_TOOLS_FILE, parsed.tools_inline.as_ref()),
    ] {
        let Some(parsed_value) = parsed_value else {
            continue;
        };
        let (path, created) = write_content_addressed(
            &options.cache_dir,
            &options.file_prefix,
            kind,
            parsed_value.value.as_bytes(),
        )?;
        if created {
            created_files.push(path.clone());
        }
        let path_arg = path
            .to_str()
            .ok_or_else(|| RewriteError::NonUtf8CachePath { path: path.clone() })?
            .to_string();
        replacements.insert(
            parsed_value.flag_index,
            vec![long_flag(file_flag), path_arg],
        );
        if let Some(value_index) = parsed_value.value_index {
            skipped_indexes.insert(value_index);
        }
    }

    let mut rewritten = Vec::with_capacity(args.len());
    for (index, arg) in args.iter().enumerate() {
        if skipped_indexes.contains(&index) {
            continue;
        }
        if let Some(values) = replacements.get(&index) {
            rewritten.extend(values.iter().cloned());
        } else {
            rewritten.push(arg.clone());
        }
    }

    Ok(RewriteResult {
        args: rewritten,
        created_files,
        changed: true,
    })
}

fn validate_sources(parsed: &ParsedImports) -> Result<(), RewriteError> {
    let spec = RawFallbackImportSpec {
        initialize_inline: parsed
            .initialize_inline
            .as_ref()
            .map(|value| value.value.clone()),
        initialize_file: parsed
            .initialize_file
            .as_ref()
            .map(|value| PathBuf::from(&value.value)),
        tools_inline: parsed
            .tools_inline
            .as_ref()
            .map(|value| value.value.clone()),
        tools_file: parsed
            .tools_file
            .as_ref()
            .map(|value| PathBuf::from(&value.value)),
    };
    try_load_fallback(&spec)?;
    Ok(())
}
