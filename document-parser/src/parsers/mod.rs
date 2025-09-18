// 解析器模块
pub mod dual_engine_parser;
pub mod format_detector;
pub mod markitdown_parser;
pub mod mineru_parser;
pub mod parser_trait;

pub use dual_engine_parser::{DualEngineParser, ParserStats};
pub use format_detector::{DetectionMethod, DetectionResult, FormatDetector};
pub use markitdown_parser::MarkItDownParser;
pub use mineru_parser::MinerUParser;
pub use parser_trait::{DocumentParser, ParserFactory};
