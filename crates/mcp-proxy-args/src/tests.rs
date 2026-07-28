use std::fs;

use tempfile::tempdir;

use super::*;

const INIT: &str = r#"{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}"#;
const TOOLS: &str = r#"{"tools":[{"name":"search","inputSchema":{"type":"object"}}]}"#;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn loads_mixed_sources() {
    let directory = tempdir().expect("tempdir");
    let tools_path = directory.path().join("tools.json");
    fs::write(&tools_path, TOOLS).expect("write tools");
    let loaded = try_load_fallback(&RawFallbackImportSpec {
        initialize_inline: Some(INIT.to_string()),
        tools_file: Some(tools_path.clone()),
        ..Default::default()
    })
    .expect("load")
    .expect("fallback");

    assert_eq!(loaded.initialize.source, ImportSource::Inline);
    assert_eq!(loaded.tools.source, ImportSource::File(tools_path));
}

#[test]
fn rejects_incomplete_pair() {
    let error = try_load_fallback(&RawFallbackImportSpec {
        initialize_inline: Some(INIT.to_string()),
        ..Default::default()
    })
    .expect_err("incomplete pair");
    assert!(matches!(error, LoadError::IncompletePair));
}

#[test]
fn recognizes_only_real_convert_subcommand() {
    assert!(is_mcp_proxy_convert(
        "/usr/local/bin/mcp-proxy",
        &strings(&["-v", "convert", "http://localhost"])
    ));
    assert!(is_mcp_proxy_convert(
        "mcp-proxy.exe",
        &strings(&["convert"])
    ));
    assert!(is_mcp_proxy_convert(
        r"C:\Program Files\mcp-proxy.exe",
        &strings(&["convert"])
    ));
    assert!(!is_mcp_proxy_convert(
        "mcp-proxy",
        &strings(&["proxy", "convert"])
    ));
    assert!(!is_mcp_proxy_convert(
        "mcp-proxy",
        &strings(&["-", "convert"])
    ));
    assert!(!is_mcp_proxy_convert("node", &strings(&["convert"])));
}

#[test]
fn non_target_has_no_side_effects() {
    let directory = tempdir().expect("tempdir");
    let cache = directory.path().join("missing");
    let args = strings(&["proxy", "--import-tools", TOOLS]);
    let result =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &RewriteOptions::new(&cache))
            .expect("rewrite");
    assert!(!result.changed);
    assert_eq!(result.args, args);
    assert!(!cache.exists());
}

#[test]
fn rewrites_inline_forms_and_reuses_files() {
    let directory = tempdir().expect("tempdir");
    let options = RewriteOptions::new(directory.path());
    let args = vec![
        "convert".to_string(),
        "http://localhost".to_string(),
        format!("--{}={INIT}", flags::IMPORT_INITIALIZE),
        format!("--{}={TOOLS}", flags::IMPORT_TOOLS),
        "--quiet".to_string(),
    ];

    let first =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &options).expect("first rewrite");
    assert!(first.changed);
    assert_eq!(first.created_files.len(), 2);
    assert!(
        !first
            .args
            .iter()
            .any(|value| value == INIT || value == TOOLS)
    );

    let second =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &options).expect("second rewrite");
    assert!(second.changed);
    assert!(second.created_files.is_empty());
    assert_eq!(first.args, second.args);
}

#[test]
fn leaves_existing_file_arguments_unchanged() {
    let directory = tempdir().expect("tempdir");
    let initialize_path = directory.path().join("init.json");
    let tools_path = directory.path().join("tools.json");
    fs::write(&initialize_path, INIT).expect("write initialize");
    fs::write(&tools_path, TOOLS).expect("write tools");
    let args = vec![
        "convert".to_string(),
        "http://localhost".to_string(),
        "--import-initialize-file".to_string(),
        initialize_path.display().to_string(),
        "--import-tools-file".to_string(),
        tools_path.display().to_string(),
    ];
    let result = rewrite_convert_import_args_to_files(
        "mcp-proxy",
        &args,
        &RewriteOptions::new(directory.path()),
    )
    .expect("rewrite");
    assert!(!result.changed);
    assert_eq!(result.args, args);
}

#[test]
fn validates_file_sources_before_writing_inline_cache() {
    let directory = tempdir().expect("tempdir");
    let cache = directory.path().join("cache");
    let missing_tools = directory.path().join("missing-tools.json");
    let args = vec![
        "convert".to_string(),
        "--import-initialize".to_string(),
        INIT.to_string(),
        "--import-tools-file".to_string(),
        missing_tools.display().to_string(),
    ];

    let error =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &RewriteOptions::new(&cache))
            .expect_err("all sources must validate before cache files are written");

    assert!(matches!(
        error,
        RewriteError::Load(LoadError::ReadFile { kind: "tools", .. })
    ));
    assert!(!cache.exists());
}

#[test]
fn stops_parsing_at_double_dash() {
    let directory = tempdir().expect("tempdir");
    let args = strings(&[
        "convert",
        "http://localhost",
        "--",
        "--import-initialize",
        INIT,
    ]);
    let result = rewrite_convert_import_args_to_files(
        "mcp-proxy",
        &args,
        &RewriteOptions::new(directory.path()),
    )
    .expect("rewrite");
    assert!(!result.changed);
}

#[test]
fn rejects_duplicate_conflicting_and_missing_values_before_writing() {
    let directory = tempdir().expect("tempdir");
    let options = RewriteOptions::new(directory.path().join("cache"));
    let cases = [
        strings(&[
            "convert",
            "--import-initialize",
            INIT,
            "--import-initialize",
            INIT,
            "--import-tools",
            TOOLS,
        ]),
        strings(&[
            "convert",
            "--import-initialize",
            INIT,
            "--import-initialize-file",
            "/tmp/init.json",
            "--import-tools",
            TOOLS,
        ]),
        strings(&["convert", "--import-initialize", "--import-tools", TOOLS]),
    ];

    for args in cases {
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &options)
            .expect_err("invalid arguments must fail");
        assert!(!options.cache_dir.exists());
    }
}

#[test]
fn concurrent_rewrites_publish_one_consistent_pair() {
    let directory = tempdir().expect("tempdir");
    let options = RewriteOptions::new(directory.path());
    let args = strings(&[
        "convert",
        "--import-initialize",
        INIT,
        "--import-tools",
        TOOLS,
    ]);
    let handles = (0..8)
        .map(|_| {
            let args = args.clone();
            let options = options.clone();
            std::thread::spawn(move || {
                rewrite_convert_import_args_to_files("mcp-proxy", &args, &options)
                    .expect("concurrent rewrite")
            })
        })
        .collect::<Vec<_>>();

    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("rewrite thread"))
        .collect::<Vec<_>>();
    assert!(results.iter().all(|result| result.args == results[0].args));
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("read cache")
            .filter_map(Result::ok)
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn writes_private_files() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("tempdir");
    let args = strings(&[
        "convert",
        "--import-initialize",
        INIT,
        "--import-tools",
        TOOLS,
    ]);
    let result = rewrite_convert_import_args_to_files(
        "mcp-proxy",
        &args,
        &RewriteOptions::new(directory.path()),
    )
    .expect("rewrite");
    let directory_mode = fs::metadata(directory.path())
        .expect("directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(directory_mode, 0o700);
    for path in result.created_files {
        let mode = fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_cache_path_before_writing() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempdir().expect("tempdir");
    let cache = directory
        .path()
        .join(OsString::from_vec(vec![b'c', b'a', b'c', b'h', b'e', 0xff]));
    let args = strings(&[
        "convert",
        "--import-initialize",
        INIT,
        "--import-tools",
        TOOLS,
    ]);

    let error =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &RewriteOptions::new(&cache))
            .expect_err("non UTF-8 cache paths cannot be represented in process arguments");

    assert!(matches!(error, RewriteError::NonUtf8CachePath { .. }));
    assert!(!cache.exists());
}

#[cfg(unix)]
#[test]
fn rejects_symlink_cache_directory() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("tempdir");
    let actual_cache = directory.path().join("actual-cache");
    fs::create_dir(&actual_cache).expect("create actual cache");
    let cache_link = directory.path().join("cache-link");
    symlink(&actual_cache, &cache_link).expect("create cache symlink");
    let args = strings(&[
        "convert",
        "--import-initialize",
        INIT,
        "--import-tools",
        TOOLS,
    ]);

    let error =
        rewrite_convert_import_args_to_files("mcp-proxy", &args, &RewriteOptions::new(&cache_link))
            .expect_err("cache directory symlinks must be rejected");

    assert!(matches!(
        error,
        RewriteError::Cache(crate::cache::CacheError::InvalidDirectoryType { .. })
    ));
    assert_eq!(
        fs::read_dir(&actual_cache)
            .expect("read actual cache")
            .count(),
        0
    );
}
