//! LocalAgreement 2（token-based，经典）：双解码取 token 公共前缀，稳定后 commit。
//!
//! Whisper 每次全量解码 buffer，输出**通常以 committed 为前缀**（同音频前缀的解码
//! 稳定）。`skip_prefix` 剥离已 commit token，对剩余取 A/B 公共前缀，稳定后 commit
//! 增量。token 级而非 segment 级：Whisper 常对一段音频只返回单个 segment（整句），
//! segment 级提交会在首个 segment 后把后续全过滤掉；token 级在 segment 内拆词对齐，
//! 单 segment 也能增量 commit。
//!
//! **治粘连**：granularity=Word 时 `join(" ")` 保留英文空格；CJK 用 Char（无分隔符）。
//! granularity 按 language 自动推断（`auto`）：CJK→char，其余→word。
//!
//! **治重复**：Word 粒度下整词为对齐单位，Whisper 对同音频前缀的解码稳定时
//! `skip_prefix` 干净剥离已 commit 词，只 commit 增量。Char 粒度下每字符是对齐单位，
//! 任一字符抖动即断裂、重复严重，故默认 Word。

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
    /// 按 language 自动推断（CJK→char，其余→word）
    pub fn auto_for_language(lang: Option<&str>) -> Self {
        match lang.map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("zh" | "chinese" | "ja" | "japanese" | "ko" | "korean") => Self::Char,
            _ => Self::Word,
        }
    }

    /// 解析配置字符串：`char`/`word` 强制；其余（含 `auto`）按 language 推断
    pub fn resolve(s: &str, lang: Option<&str>) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "char" => Self::Char,
            "word" => Self::Word,
            _ => Self::auto_for_language(lang),
        }
    }

    /// token 间拼接分隔符
    pub fn separator(self) -> &'static str {
        match self {
            Self::Word => " ",
            Self::Char => "",
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
            granularity: CompareGranularity::Word,
        }
    }
}

impl From<&StreamingConfig> for LaConfig {
    fn from(c: &StreamingConfig) -> Self {
        Self {
            min_agree_count: c.min_agree_count.max(1),
            granularity: CompareGranularity::resolve(&c.compare_granularity, None),
        }
    }
}

/// Whisper segment（流式 LA 输入；与同步 handler 映射源头一致）
#[derive(Debug, Clone, PartialEq)]
pub struct SttSegment {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// LA2 单次决策结果（raw token）
#[derive(Debug, Clone, Default)]
pub struct LaDecision {
    /// 本次新确认并入 committed 的 token（raw，保留大小写）
    pub newly_committed: Vec<String>,
    /// committed 之后的当前 A 解码剩余 token（partial 预览）
    pub partial: Vec<String>,
}

/// 带归一化的 token（raw 保留大小写用于输出，norm 小写用于比较）
#[derive(Debug, Clone)]
struct Tk {
    raw: String,
    norm: String,
}

/// LocalAgreement 2 状态机（token-based）
pub struct LocalAgreement {
    cfg: LaConfig,
    /// 已确认 token（只增；decoder 全量解码 buffer，结果应以其为前缀）
    committed: Vec<Tk>,
    /// 当前 A/B 公共前缀长度（相对 committed 之后的待确认区）
    pending_len: usize,
    /// 公共前缀连续不回退次数
    streak: u32,
    /// 最后一次 observe 的 partial（flush 时强制 commit）
    last_partial: Vec<Tk>,
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

    /// 已确认的完整文本（raw token 按 granularity 拼接）
    pub fn committed_text(&self) -> String {
        let sep = self.cfg.granularity.separator();
        self.committed
            .iter()
            .map(|t| t.raw.as_str())
            .collect::<Vec<_>>()
            .join(sep)
    }

    /// 观察一次双解码结果（A = 完整 buffer，B = 裁剪尾部 buffer）。
    ///
    /// streak 语义：`prefix_len >= pending_len` 则 streak+1（稳定/增长），否则清零。
    /// `streak >= min_agree_count` 时 commit 公共前缀 token 进 committed。
    pub fn observe(&mut self, segs_a: &[SttSegment], segs_b: &[SttSegment]) -> LaDecision {
        // segments → 文本（segment 间补空格，便于 Word 分词；Char 粒度空格被过滤）
        let a_text: String = segs_a
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let b_text: String = segs_b
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let a_tok = tokenize(&a_text, self.cfg.granularity);
        let b_tok = tokenize(&b_text, self.cfg.granularity);
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

        let mut newly: Vec<Tk> = Vec::new();
        if self.streak >= self.cfg.min_agree_count && self.pending_len > 0 {
            newly = a_new[..self.pending_len].to_vec();
            self.committed.extend_from_slice(&newly);
            self.pending_len = 0;
            self.streak = 0;
        }

        let partial = a_new[newly.len()..].to_vec();
        self.last_partial = partial.clone();
        LaDecision {
            newly_committed: newly.into_iter().map(|t| t.raw).collect(),
            partial: partial.into_iter().map(|t| t.raw).collect(),
        }
    }

    /// 强制 commit 最后一次 A 解码的全部剩余（句末 / VAD 静音 / 会话结束）。
    pub fn flush_remaining(&mut self) -> Vec<String> {
        let newly = std::mem::take(&mut self.last_partial);
        self.committed.extend_from_slice(&newly);
        self.pending_len = 0;
        self.streak = 0;
        newly.into_iter().map(|t| t.raw).collect()
    }

    /// 重置为新一轮 utterance（清空全部状态）
    pub fn reset_for_new_utterance(&mut self) {
        self.committed.clear();
        self.pending_len = 0;
        self.streak = 0;
        self.last_partial.clear();
    }
}

/// 分词 + 归一化。Word 粒度按空白分词再去首尾标点；Char 粒度逐字。
/// 返回 Tk{raw（保留大小写）, norm（小写）}。
fn tokenize(text: &str, granularity: CompareGranularity) -> Vec<Tk> {
    match granularity {
        CompareGranularity::Char => text
            .chars()
            .filter(|c| !is_punctuation(*c) && !c.is_whitespace())
            .map(|c| Tk {
                raw: c.to_string(),
                norm: c.to_lowercase().to_string(),
            })
            .collect(),
        CompareGranularity::Word => text
            .split_whitespace()
            .filter_map(|w| {
                let raw = trim_punctuation(w);
                if raw.is_empty() {
                    return None;
                }
                Some(Tk {
                    norm: raw.to_lowercase(),
                    raw,
                })
            })
            .collect(),
    }
}

/// 去首尾 ASCII + CJK 标点（"Hello," → "Hello"）
fn trim_punctuation(w: &str) -> String {
    w.trim_matches(|c: char| is_punctuation(c) || c.is_whitespace())
        .to_string()
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

/// 两个 token 序列的最长公共前缀长度（按 norm 比较，大小写不敏感）
fn common_prefix_len(a: &[Tk], b: &[Tk]) -> usize {
    a.iter()
        .zip(b.iter())
        .take_while(|(x, y)| x.norm == y.norm)
        .count()
}

/// 跳过 tokens 中与 committed 公共前缀的部分，返回剩余切片。
/// 正常情况 decoder 输出以 committed 为前缀；偶发抖动取最长公共前缀，避免 panic。
fn skip_prefix<'a>(tokens: &'a [Tk], committed: &[Tk]) -> &'a [Tk] {
    let valid = tokens
        .iter()
        .zip(committed.iter())
        .take_while(|(t, c)| t.norm == c.norm)
        .count();
    &tokens[valid..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(granularity: CompareGranularity) -> LaConfig {
        LaConfig {
            min_agree_count: 2,
            granularity,
        }
    }

    fn seg(start: f32, end: f32, text: &str) -> SttSegment {
        SttSegment {
            start,
            end,
            text: text.to_string(),
        }
    }

    #[test]
    fn test_observe_stable_commit() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Word));
        let s = vec![seg(0.0, 1.0, "hello world")];
        let d1 = la.observe(&s, &s);
        assert!(d1.newly_committed.is_empty()); // streak=1，未达 2
        assert_eq!(d1.partial, vec!["hello", "world"]);
        let d2 = la.observe(&s, &s);
        assert_eq!(d2.newly_committed, vec!["hello", "world"]); // streak=2 → commit
        assert!(d2.partial.is_empty());
        assert_eq!(la.committed_text(), "hello world");
    }

    #[test]
    fn test_observe_regression_resets_streak() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Char));
        let full = vec![seg(0.0, 1.0, "abc")];
        la.observe(&full, &full); // streak=1, pending=3
        // B 裁剪更多，公共前缀回退到 2
        let b_short = vec![seg(0.0, 1.0, "ab")];
        la.observe(&full, &b_short); // prefix=2 < 3 → streak=0
        assert_eq!(la.streak, 0);
        la.observe(&full, &full); // prefix=3 >= 2 → streak=1
        la.observe(&full, &full); // streak=2 → commit
        assert_eq!(la.committed_text(), "abc");
    }

    #[test]
    fn test_flush_remaining() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Char));
        let s = vec![seg(0.0, 1.0, "abc")];
        la.observe(&s, &s); // streak=1, partial=abc，未 commit
        let flushed = la.flush_remaining();
        assert_eq!(flushed, vec!["a", "b", "c"]);
        assert_eq!(la.committed_text(), "abc");
    }

    /// 治粘连：Word 粒度 join(" ") 保留英文空格
    #[test]
    fn test_word_join_preserves_spaces() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Word));
        let s = vec![seg(0.0, 2.0, "hello world foo bar")];
        la.observe(&s, &s);
        la.observe(&s, &s);
        assert_eq!(la.committed_text(), "hello world foo bar");
    }

    /// CJK Char 粒度无分隔符
    #[test]
    fn test_char_join_cjk() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Char));
        let s = vec![seg(0.0, 2.0, "你好世界")];
        la.observe(&s, &s);
        la.observe(&s, &s);
        assert_eq!(la.committed_text(), "你好世界");
    }

    /// ★ 治重复核心：skip_prefix 剥离已 commit，后续解码含 committed 前缀时只 commit 增量
    #[test]
    fn test_skip_prefix_strips_committed_no_duplicate() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Word));
        // 第1轮 commit "and so"
        let s1 = vec![seg(0.0, 1.0, "and so")];
        la.observe(&s1, &s1);
        la.observe(&s1, &s1);
        assert_eq!(la.committed_text(), "and so");
        // 第2轮：A/B 解码更长文本 "and so my fellow"（含已 commit 前缀）
        let s2 = vec![seg(0.0, 2.0, "and so my fellow")];
        la.observe(&s2, &s2); // skip [and,so] → [my,fellow], streak=1
        let d = la.observe(&s2, &s2); // streak=2 → commit [my, fellow]
        assert_eq!(d.newly_committed, vec!["my", "fellow"]); // 增量，无重复
        assert_eq!(la.committed_text(), "and so my fellow");
    }

    #[test]
    fn test_multi_round_commit() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Word));
        let s1 = vec![seg(0.0, 1.0, "hello")];
        la.observe(&s1, &s1);
        la.observe(&s1, &s1); // commit hello
        assert_eq!(la.committed_text(), "hello");
        // 新内容追加（committed=hello，decoder 全量解码 helloworld）
        let s2 = vec![seg(0.0, 2.0, "hello world")];
        la.observe(&s2, &s2); // skip [hello] → [world], streak=1
        la.observe(&s2, &s2); // streak=2 → commit world
        assert_eq!(la.committed_text(), "hello world");
    }

    #[test]
    fn test_auto_granularity_by_language() {
        assert_eq!(
            CompareGranularity::auto_for_language(Some("en")),
            CompareGranularity::Word
        );
        assert_eq!(
            CompareGranularity::auto_for_language(Some("EN")),
            CompareGranularity::Word
        );
        assert_eq!(
            CompareGranularity::auto_for_language(Some("zh")),
            CompareGranularity::Char
        );
        assert_eq!(
            CompareGranularity::auto_for_language(Some("japanese")),
            CompareGranularity::Char
        );
        assert_eq!(
            CompareGranularity::auto_for_language(None),
            CompareGranularity::Word
        );
    }

    #[test]
    fn test_resolve_granularity() {
        assert_eq!(
            CompareGranularity::resolve("char", Some("en")),
            CompareGranularity::Char
        );
        assert_eq!(
            CompareGranularity::resolve("word", Some("zh")),
            CompareGranularity::Word
        );
        assert_eq!(
            CompareGranularity::resolve("auto", Some("zh")),
            CompareGranularity::Char
        );
        assert_eq!(
            CompareGranularity::resolve("auto", Some("en")),
            CompareGranularity::Word
        );
    }

    #[test]
    fn test_reset_for_new_utterance() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Char));
        let s = vec![seg(0.0, 1.0, "abc")];
        la.observe(&s, &s);
        la.observe(&s, &s);
        assert_eq!(la.committed_text(), "abc");
        la.reset_for_new_utterance();
        assert!(la.committed_text().is_empty());
    }

    /// common_prefix 大小写不敏感，commit 用 A 原始大小写
    #[test]
    fn test_common_prefix_case_insensitive() {
        let mut la = LocalAgreement::new(cfg(CompareGranularity::Word));
        let a = vec![seg(0.0, 1.0, "Hello World")];
        let b = vec![seg(0.0, 1.0, "hello world")];
        la.observe(&a, &b); // streak=1（norm 相等）
        let d = la.observe(&a, &b); // streak=2 → commit，保留 A 的 raw
        assert_eq!(d.newly_committed, vec!["Hello", "World"]);
    }

    #[test]
    fn test_trim_punctuation() {
        assert_eq!(trim_punctuation("hello,"), "hello");
        assert_eq!(trim_punctuation("\"world\""), "world");
        assert_eq!(trim_punctuation("..."), "");
    }
}
