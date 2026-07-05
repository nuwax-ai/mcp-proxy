//! TTS 模型管理（sherpa-onnx Kokoro 模型布局）。
//!
//! Kokoro 模型目录布局（与 sherpa-onnx 官方 release 一致）：
//! ```text
//! {models_dir}/tts/{model_id}/
//!   model.onnx        ← Kokoro 主模型
//!   voices.bin        ← 多音色嵌入（fixed multi-voice）
//!   tokens.txt        ← token 词表
//!   espeak-ng-data/   ← phonemizer 数据（data_dir）
//!   dict/             ← 中文分词词典（可选，dict_dir；非中文模型可缺）
//! ```
//!
//! # 网络
//! 模型托管在 HuggingFace `k2-fsa/sherpa-onnx-models`，本环境 HF 被阻断；
//! v1 不做自动下载（`auto_download=false`），由部署方手动放置模型目录。
//! 缺模型时 [`ensure_model`] 返回 `ModelNotFound`，附手动放置指引（Fail Fast）。

use std::path::{Path, PathBuf};

use crate::tts::TtsError;

/// Kokoro 模型所需文件路径集合（由 [`TtsModelService`] 从模型目录解析）。
#[derive(Debug, Clone)]
pub struct TtsModelPaths {
    /// `model.onnx`（主模型）
    pub model: PathBuf,
    /// `voices.bin`（多音色嵌入）
    pub voices: PathBuf,
    /// `tokens.txt`
    pub tokens: PathBuf,
    /// `espeak-ng-data/`（phonemizer 数据目录）
    pub data_dir: PathBuf,
    /// `dict/`（中文分词词典；非中文模型可为 None）
    pub dict_dir: Option<PathBuf>,
    /// `lexicon.txt`（可选，部分模型需要）
    pub lexicon: Option<PathBuf>,
}

/// TTS 模型服务：按 model_id 解析模型目录 + 校验必备文件。
///
/// 与 STT 的 `ModelService` 对称，但 v1 不做自动下载（HF 阻断 + 模型体积大）。
#[derive(Debug)]
pub struct TtsModelService {
    models_dir: PathBuf,
}

impl TtsModelService {
    /// 从配置的 `tts.engine.models_dir` 构造。
    pub fn new(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }

    /// 模型根目录（`{models_dir}/{model_id}`）。
    pub fn model_root(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id)
    }

    /// 确保模型就绪：校验目录 + 必备文件存在。
    ///
    /// v1 不自动下载；缺失即报 `ModelNotFound`（带手动放置指引）。
    pub fn ensure_model(&self, model_id: &str) -> Result<(), TtsError> {
        let root = self.model_root(model_id);
        if !root.is_dir() {
            return Err(TtsError::ModelNotFound {
                model: format!(
                    "{}（Kokoro 模型目录不存在。请从 https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models \
                     下载 kokoro 模型，解压到 {}）",
                    root.display(),
                    root.display()
                ),
            });
        }
        // 校验必备文件（Fail Fast：缺任一即报，避免传到 sherpa-onnx 才在 C 端失败）
        let paths = self.resolve_paths(model_id)?;
        for (name, p) in [
            ("model.onnx", &paths.model),
            ("voices.bin", &paths.voices),
            ("tokens.txt", &paths.tokens),
            ("espeak-ng-data/", &paths.data_dir),
        ] {
            if !p.exists() {
                return Err(TtsError::ModelNotFound {
                    model: format!("{} 缺失：{}", name, p.display()),
                });
            }
        }
        Ok(())
    }

    /// 解析模型目录下的标准文件路径（不校验存在性，由 [`ensure_model`] 负责）。
    pub fn resolve_paths(&self, model_id: &str) -> Result<TtsModelPaths, TtsError> {
        let root = self.model_root(model_id);
        let dict_dir = root.join("dict");
        let lexicon = root.join("lexicon.txt");
        Ok(TtsModelPaths {
            model: root.join("model.onnx"),
            voices: root.join("voices.bin"),
            tokens: root.join("tokens.txt"),
            data_dir: root.join("espeak-ng-data"),
            dict_dir: dict_dir.is_dir().then_some(dict_dir),
            lexicon: lexicon.is_file().then_some(lexicon),
        })
    }

    /// 校验模型目录是否存在（轻量探测，不做完整文件校验）。
    pub fn model_exists(&self, model_id: &str) -> bool {
        self.model_root(model_id).is_dir()
    }
}

/// 把 `Path` 转成 sherpa-onnx 期望的 `Option<String>`（空路径 → None）。
pub fn path_to_opt_string(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    if s.is_empty() {
        None
    } else {
        Some(s.into_owned())
    }
}
