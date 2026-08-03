pub const IMPORT_INITIALIZE: &str = "import-initialize";
pub const IMPORT_INITIALIZE_FILE: &str = "import-initialize-file";
pub const IMPORT_TOOLS: &str = "import-tools";
pub const IMPORT_TOOLS_FILE: &str = "import-tools-file";
pub const EXPORT_INITIALIZE: &str = "export-initialize";
pub const EXPORT_TOOLS: &str = "export-tools";

pub(crate) fn long_flag(name: &str) -> String {
    format!("--{name}")
}
