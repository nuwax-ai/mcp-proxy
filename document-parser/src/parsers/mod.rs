// 解析器模块
pub mod parser_trait;
pub mod mineru_parser;
pub mod markitdown_parser;
pub mod dual_engine_parser;
pub mod format_detector;

pub use parser_trait::{DocumentParser, ParserFactory};
pub use mineru_parser::MinerUParser;
pub use markitdown_parser::MarkItDownParser;
pub use dual_engine_parser::{DualEngineParser, ParserStats};
pub use format_detector::{FormatDetector, DetectionResult, DetectionMethod};
