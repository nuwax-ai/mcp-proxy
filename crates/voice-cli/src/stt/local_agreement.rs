//! LocalAgreement 2 算法：双解码取公共前缀，稳定后才 commit。
//!
//! 流式 STT 中，单次解码会抖动（同一段音频多次解码结果可能不同）。
//! LA2 用两个 decoder 视角的结果取最长公共前缀，当公共前缀连续 N 次
//! 不回退（稳定）时才 commit，避免抖动导致的丢字 / 重复。
//!
//! 典型用法：A = 完整 audio buffer 解码，B = 裁剪尾部 `tail_trim_sec` 后解码。
//! 尾部音频不足时 A/B 差异大、公共前缀短；尾部稳定后 A/B 趋同、公共前缀增长。
//!
//! 纯函数式状态机，无 tokio 依赖，便于高密度单测（见 `streaming_session` 调度）。

use crate::models::config::StreamingConfig;

/// LA2 比较粒度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareGranularity {
    /// 按字符（中日韩，无空格分词）
    Char,
    /// 按词（空格分隔语种）
    Word,
}

impl CompareGranularity {
    /// 从配置字符串解析粒度（非 FromStr：默认 char，不报错）
    pub fn parse_granularity(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "word" => Self::Word,
            _ => Self::Char, // 默认 char（CJK 友好）
        }
    }
}

/// LA2 配置
#[derive(Debug, Clone)]
pub struct LaConfig {
    /// 前缀连续不回退多少次才 commit（标准 LocalAgreement 2 = 2）
    pub min_agree_count: u32,
    /// 比较粒度
    pub granularity: CompareGranularity,
}

impl Default for LaConfig {
    fn default() -> Self {
        Self {
            min_agree_count: 2,
            granularity: CompareGranularity::Char,
        }
    }
}

impl From<&StreamingConfig> for LaConfig {
    fn from(c: &StreamingConfig) -> Self {
        Self {
            min_agree_count: c.min_agree_count.max(1),
            granularity: CompareGranularity::parse_granularity(&c.compare_granularity),
        }
    }
}

/// LA2 单次决策结果
#[derive(Debug, Clone, Default)]
pub struct LaDecision {
    /// 本次新确认并入 committed 的 token（用于推 committed 增量事件）
    pub newly_committed: Vec<String>,
    /// committed 之后的当前 A 解码（用于推 partial 预览事件 = committed_total + partial）
    pub partial: Vec<String>,
}

/// LocalAgreement 2 状态机
pub struct LocalAgreement {
    cfg: LaConfig,
    /// 已确认的 token（只增；decoder 全量解码 buffer，结果应以其为前缀）
    committed: Vec<String>,
    /// 当前 A/B 公共前缀长度（相对 committed 之后的待确认区）
    pending_len: usize,
    /// 公共前缀连续不回退次数
    streak: u32,
    /// 最后一次 observe 的 partial（flush 时强制 commit）
    last_partial: Vec<String>,
}

impl LocalAgreement {
    pub fn new(cfg: LaConfig) -> Self {
        Self {
            cfg,
            committed: Vec::new(),
            pending_len: 0,
            streak: 0,
            last_partial: Vec::new(),
        }
    }

    /// 已确认的完整文本（Word 粒度用空格连接，Char 粒度直接拼接）
    pub fn committed_text(&self) -> String {
        match self.cfg.granularity {
            CompareGranularity::Word => self.committed.join(" "),
            CompareGranularity::Char => self.committed.join(""),
        }
    }

    /// 观察一次双解码结果（A = 完整 buffer，B = 裁剪尾部 buffer）。
    ///
    /// streak 语义：`prefix_len >= pending_len` 则 streak+1（稳定 / 增长），
    /// 否则 streak 清零（回退）。`streak >= min_agree_count` 时 commit
    /// 公共前缀 `[0..pending_len]` 进 committed。
    pub fn observe(&mut self, decode_a: &str, decode_b: &str) -> LaDecision {
        let a_tok = tokenize(decode_a, self.cfg.granularity);
        let b_tok = tokenize(decode_b, self.cfg.granularity);
        // 跳过已 committed 的前缀（decoder 全量解码，结果含 committed）
        let a_new = skip_prefix(&a_tok, &self.committed);
        let b_new = skip_prefix(&b_tok, &self.committed);
        let prefix_len = common_prefix_len(a_new, b_new);

        if prefix_len >= self.pending_len {
            self.streak += 1;
        } else {
            self.streak = 0;
        }
        self.pending_len = prefix_len;

        let mut newly_committed = Vec::new();
        let mut committed_this_round = 0usize;
        if self.streak >= self.cfg.min_agree_count && self.pending_len > 0 {
            committed_this_round = self.pending_len;
            newly_committed = a_new[..committed_this_round].to_vec();
            self.committed.extend_from_slice(&newly_committed);
            self.pending_len = 0;
            self.streak = 0;
        }

        let partial = a_new[committed_this_round..].to_vec();
        self.last_partial = partial.clone();
        LaDecision {
            newly_committed,
            partial,
        }
    }

    /// 强制 commit 最后一次 A 解码的全部剩余（句末 / VAD 静音 / 会话结束）。
    pub fn flush_remaining(&mut self) -> Vec<String> {
        let newly = std::mem::take(&mut self.last_partial);
        self.committed.extend_from_slice(&newly);
        self.pending_len = 0;
        self.streak = 0;
        newly
    }

    /// 重置为新一轮 utterance（清空全部状态）
    pub fn reset_for_new_utterance(&mut self) {
        self.committed.clear();
        self.pending_len = 0;
        self.streak = 0;
        self.last_partial.clear();
    }
}

/// 分词 + 归一化（去标点 / 空白，小写）。
///
/// 去标点避免「你好」vs「你好，」被判定为前缀回退；小写避免大小写差异。
/// Word 粒度必须先按空白分词再去标点（否则去空白后无法分词）。
fn tokenize(text: &str, granularity: CompareGranularity) -> Vec<String> {
    match granularity {
        CompareGranularity::Char => text
            .chars()
            .filter(|c| !is_punctuation(*c) && !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .map(|c| c.to_string())
            .collect(),
        CompareGranularity::Word => text
            .split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| !is_punctuation(*c))
                    .flat_map(|c| c.to_lowercase())
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect(),
    }
}

/// ASCII 标点 + 常见 CJK 标点
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '，' | '。'
                | '、'
                | '？'
                | '！'
                | '：'
                | '；'
                | '「'
                | '」'
                | '『'
                | '』'
                | '（'
                | '）'
                | '《'
                | '》'
                | '…'
                | '—'
        )
}

/// 两个 token 序列的最长公共前缀长度
fn common_prefix_len(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

/// 跳过 tokens 中与 committed 公共前缀的部分，返回剩余切片。
///
/// 正常情况 decoder 输出以 committed 为前缀（全量解码 buffer）；
/// 偶发抖动时取最长公共前缀，避免 panic。
fn skip_prefix<'a>(tokens: &'a [String], committed: &[String]) -> &'a [String] {
    let valid = tokens
        .iter()
        .zip(committed.iter())
        .take_while(|(t, c)| t == c)
        .count();
    &tokens[valid..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_char() -> LaConfig {
        LaConfig {
            min_agree_count: 2,
            granularity: CompareGranularity::Char,
        }
    }

    #[test]
    fn test_common_prefix_len() {
        assert_eq!(common_prefix_len(&[], &[]), 0);
        assert_eq!(common_prefix_len(&["a".to_string()], &[]), 0);
        assert_eq!(
            common_prefix_len(
                &["a".into(), "b".into(), "c".into()],
                &["a".into(), "b".into()]
            ),
            2
        );
        assert_eq!(
            common_prefix_len(&["a".into(), "x".into()], &["a".into(), "b".into()]),
            1
        );
    }

    #[test]
    fn test_tokenize_char_normalizes() {
        let t = tokenize("Hello, World!", CompareGranularity::Char);
        assert_eq!(t, vec!["h", "e", "l", "l", "o", "w", "o", "r", "l", "d"]);
    }

    #[test]
    fn test_tokenize_word() {
        let t = tokenize("Hello, World!", CompareGranularity::Word);
        assert_eq!(t, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_cjk_punctuation() {
        let t = tokenize("你好，世界！", CompareGranularity::Char);
        assert_eq!(t, vec!["你", "好", "世", "界"]);
    }

    #[test]
    fn test_observe_stable_commit() {
        let mut la = LocalAgreement::new(cfg_char());
        // 两次相同解码，streak=2 → commit
        let d1 = la.observe("abc", "abc");
        assert!(d1.newly_committed.is_empty()); // streak=1，未达 2
        assert_eq!(d1.partial, vec!["a", "b", "c"]);
        let d2 = la.observe("abc", "abc");
        assert_eq!(d2.newly_committed, vec!["a", "b", "c"]); // streak=2 → commit
        assert!(d2.partial.is_empty());
        assert_eq!(la.committed_text(), "abc");
    }

    #[test]
    fn test_observe_growing_buffer() {
        let mut la = LocalAgreement::new(cfg_char());
        la.observe("ab", "ab"); // streak=1, pending=2
        // A/B 公共前缀增长到 3，3>=2 streak=2 → commit abc
        let d = la.observe("abcd", "abc");
        assert_eq!(d.newly_committed, vec!["a", "b", "c"]);
        assert_eq!(d.partial, vec!["d"]); // committed 之后的 A 解码剩余
    }

    #[test]
    fn test_observe_regression_resets_streak() {
        let mut la = LocalAgreement::new(cfg_char());
        la.observe("abc", "abc"); // streak=1, pending=3
        // B 裁剪更多，公共前缀回退到 2
        la.observe("abc", "ab"); // prefix=2 < 3 → streak=0
        assert_eq!(la.streak, 0);
        la.observe("abc", "abc"); // prefix=3 >= 2 → streak=1
        la.observe("abc", "abc"); // streak=2 → commit
        assert_eq!(la.committed_text(), "abc");
    }

    #[test]
    fn test_flush_remaining() {
        let mut la = LocalAgreement::new(cfg_char());
        la.observe("abc", "abc"); // streak=1, partial=abc，未 commit
        let flushed = la.flush_remaining();
        assert_eq!(flushed, vec!["a", "b", "c"]);
        assert_eq!(la.committed_text(), "abc");
    }

    #[test]
    fn test_multi_round_commit() {
        let mut la = LocalAgreement::new(cfg_char());
        la.observe("hello", "hello"); // streak=1
        la.observe("hello", "hello"); // streak=2 → commit hello
        assert_eq!(la.committed_text(), "hello");
        // 新内容追加（committed=hello，decoder 全量解码 helloworld）
        la.observe("helloworld", "helloworld"); // a_new=world, streak=1
        la.observe("helloworld", "helloworld"); // streak=2 → commit world
        assert_eq!(la.committed_text(), "helloworld");
    }

    #[test]
    fn test_reset_for_new_utterance() {
        let mut la = LocalAgreement::new(cfg_char());
        la.observe("abc", "abc");
        la.observe("abc", "abc");
        assert_eq!(la.committed_text(), "abc");
        la.reset_for_new_utterance();
        assert!(la.committed.is_empty());
        assert_eq!(la.committed_text(), "");
    }

    #[test]
    fn test_word_granularity() {
        let cfg = LaConfig {
            min_agree_count: 2,
            granularity: CompareGranularity::Word,
        };
        let mut la = LocalAgreement::new(cfg);
        la.observe("hello world", "hello world");
        la.observe("hello world", "hello world"); // streak=2 → commit
        assert_eq!(la.committed_text(), "hello world");
    }
}
