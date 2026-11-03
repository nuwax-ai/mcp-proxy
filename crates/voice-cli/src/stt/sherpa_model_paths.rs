//! sherpa-onnx ASR 模型目录解析（SenseVoice / FireRedASR2-AED / Fun-ASR-Nano / Qwen3-ASR 四布局）。
//!
//! 镜像 [`crate::tts::model_service`]（Kokoro 目录解析），但各模型布局各异，故按 [`SherpaAsrKind`]
//! 分派。模型由部署方手动放置（HF 阻断，无自动下载；拉取见 `scripts/dev/fetch-asr-models.sh`、
//! `scripts/dev/fetch-sensevoice-model.sh`，或经 deploy-installer 资产包）。
//!
//! # 各模型目录布局（sherpa-onnx `asr-models` release 解压后）
//! - **SenseVoice**：`model.onnx`(fp32) / `model.int8.onnx`(int8，**默认**，228M) + `tokens.txt`
//! - **FireRedASR2-AED**：`encoder.int8.onnx` + `decoder.int8.onnx` + `tokens.txt`
//! - **Fun-ASR-Nano int8**：`encoder_adaptor.int8.onnx` + `llm.int8.onnx` + `embedding.int8.onnx`
//!   + `Qwen3-0.6B/`（tokenizer 目录，**非单文件**）
//! - **Qwen3-ASR int8**：`conv_frontend.onnx` + `encoder.int8.onnx` + `decoder.int8.onnx`
//!   + `tokenizer/`（目录）

use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

/// 默认规范目录名（sherpa-onnx `asr-models` release 解压后的目录名）。
pub const DEFAULT_FIREREDASR2_DIR: &str = "sherpa-onnx-fire-red-asr2-zh_en-int8-2026-02-26";
pub const DEFAULT_FUNASRNANO_DIR: &str = "sherpa-onnx-funasr-nano-int8-2025-12-30";
pub const DEFAULT_QWEN3ASR_DIR: &str = "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25";
pub const DEFAULT_SENSEVOICE_DIR: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";

/// SenseVoice 量化档位：决定读目录内哪个 onnx 文件。
///
/// sherpa-onnx `asr-models` 发布的 SenseVoice 目录**同时含** `model.onnx`（fp32，894M）与
/// `model.int8.onnx`（int8，228M）。int8 实测同文本输出、CPU 推理快 2.6×、内存少 4×，故为默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SenseVoiceQuant {
    /// `model.onnx`（fp32）
    Fp32,
    /// `model.fp16.onnx`
    Fp16,
    /// `model.int8.onnx`（**默认**）
    #[default]
    Int8,
    /// `model.int4.onnx`
    Int4,
}

impl SenseVoiceQuant {
    /// 解析量化等级字符串（大小写不敏感；空串 = 默认 int8）。
    pub fn parse(s: &str) -> Result<Self, SttError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "int8" | "i8" => Ok(Self::Int8),
            "fp32" | "f32" | "float32" => Ok(Self::Fp32),
            "fp16" | "f16" | "float16" => Ok(Self::Fp16),
            "int4" | "i4" => Ok(Self::Int4),
            other => Err(SttError::InvalidInput(format!(
                "未知 SenseVoice 量化等级: {other}（支持 fp32/fp16/int8/int4）"
            ))),
        }
    }

    /// 对应文件名（`model.{quant}.onnx`）。
    pub fn filename(self) -> &'static str {
        match self {
            Self::Fp32 => "model.onnx",
            Self::Fp16 => "model.fp16.onnx",
            Self::Int8 => "model.int8.onnx",
            Self::Int4 => "model.int4.onnx",
        }
    }
}

/// sherpa-onnx ASR 引擎种类 + per-model 生成参数（hotwords / max_new_tokens）。
///
/// [`resolve_paths`] 只看 discriminant；[`crate::stt::sherpa_engine_pool`] 的 `build_recognizer`
/// 读全部字段（Fun/Qwen 的 hotwords、Qwen 的 max_new_tokens、SenseVoice 的 language/itn/量化）。
#[derive(Debug, Clone)]
pub enum SherpaAsrKind {
    /// SenseVoice（中英日韩粤；仅批量；中文 CER 优于 Whisper、原生简体）
    SenseVoice {
        /// 语种提示（`auto`/`zh`/`en`/`ja`/`ko`/`yue`；`None` = sherpa 默认）
        language: Option<String>,
        /// 逆文本归一化（口语数字/日期 → 书面，默认 true）
        use_itn: bool,
        /// 量化档位（选 `model.{quant}.onnx`，默认 int8）
        quant: SenseVoiceQuant,
    },
    /// FireRedASR2-AED（encoder-decoder，无自回归 LLM；Mac CoreML 受益最大）
    FireRedAsr2,
    /// Fun-ASR-Nano（audio-encoder + Qwen3-0.6B LLM decoder）
    FunAsrNano {
        /// 热词（逗号分隔）
        hotwords: Option<String>,
    },
    /// Qwen3-ASR-0.6B（conv frontend + encoder + LLM decoder）
    Qwen3Asr {
        /// 热词（逗号分隔）
        hotwords: Option<String>,
        /// 生成 token 上限（长音频调高）
        max_new_tokens: i32,
    },
}

/// 各模型的必备文件路径集合（由 [`resolve_paths`] 从模型目录解析 + Fail Fast 校验）。
#[derive(Debug, Clone)]
pub enum SherpaAsrPaths {
    /// SenseVoice：主模型（按量化档位选）+ tokens（**须显式传**，见下方注释）
    SenseVoice { model: PathBuf, tokens: PathBuf },
    /// FireRedASR2-AED：encoder + decoder + tokens
    FireRedAsr2 {
        encoder: PathBuf,
        decoder: PathBuf,
        tokens: PathBuf,
    },
    /// Fun-ASR-Nano：encoder_adaptor + llm + embedding + tokenizer 目录（`Qwen3-0.6B/`）
    FunAsrNano {
        encoder_adaptor: PathBuf,
        llm: PathBuf,
        embedding: PathBuf,
        tokenizer_dir: PathBuf,
    },
    /// Qwen3-ASR：conv_frontend + encoder + decoder + tokenizer 目录（`tokenizer/`）
    Qwen3Asr {
        conv_frontend: PathBuf,
        encoder: PathBuf,
        decoder: PathBuf,
        tokenizer_dir: PathBuf,
    },
}

/// 解析模型目录：显式配置优先，否则 `{models_dir}/{subdir}/{default_release_dir}`。
///
/// SenseVoice / FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR 四引擎共用（host 预置，无自动下载）。
pub fn resolve_model_dir(
    models_dir: &Path,
    subdir: &str,
    default_release_dir: &str,
    explicit: Option<&str>,
) -> PathBuf {
    if let Some(d) = explicit {
        let d = d.trim();
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    models_dir.join(subdir).join(default_release_dir)
}

/// 按 [`SherpaAsrKind`] 分派解析必备文件路径 + **Fail Fast** 校验存在性。
///
/// 在 sherpa `get_or_init_engine` 持锁段内调用（首次加载时一次性校验，避免传到 sherpa-onnx
/// C 端才报不可读错误）。
pub fn resolve_paths(kind: &SherpaAsrKind, model_dir: &Path) -> Result<SherpaAsrPaths, SttError> {
    if !model_dir.is_dir() {
        return Err(SttError::ModelNotFound {
            model: format!(
                "{}（sherpa-onnx ASR 模型目录不存在。用 scripts/dev/fetch-asr-models.sh、\
                 fetch-sensevoice-model.sh 拉取，或 deploy-installer 资产包）",
                model_dir.display()
            ),
        });
    }
    match kind {
        SherpaAsrKind::SenseVoice { quant, .. } => {
            // ⚠️ tokens 必须显式解析并传给 sherpa：`OfflineSenseVoiceModelConfig` 没有 tokens 字段，
            // 而 C 端 `offline-model-config.cc:Validate` 强制校验全局 `tokens`，不会从 model.onnx
            // 同目录自动推导（T0 实测：漏传即 `tokens: '' does not exist` → create 返回 None）。
            let tokens = require_file(model_dir, "tokens.txt")?;
            // 量化档位文件缺失时回退 model.onnx（fp32 发布名），并告警——不静默。
            let want = quant.filename();
            let model = match require_file(model_dir, want) {
                Ok(p) => p,
                Err(_) if want != "model.onnx" => {
                    let fallback = require_file(model_dir, "model.onnx")?;
                    tracing::warn!(
                        "SenseVoice 量化档位 {:?} 文件 {} 不存在，回退 model.onnx（fp32，内存↑）",
                        quant,
                        want
                    );
                    fallback
                }
                Err(e) => return Err(e),
            };
            Ok(SherpaAsrPaths::SenseVoice { model, tokens })
        }
        SherpaAsrKind::FireRedAsr2 => Ok(SherpaAsrPaths::FireRedAsr2 {
            encoder: require_file(model_dir, "encoder.int8.onnx")?,
            decoder: require_file(model_dir, "decoder.int8.onnx")?,
            tokens: require_file(model_dir, "tokens.txt")?,
        }),
        SherpaAsrKind::FunAsrNano { .. } => Ok(SherpaAsrPaths::FunAsrNano {
            encoder_adaptor: require_file(model_dir, "encoder_adaptor.int8.onnx")?,
            llm: require_file(model_dir, "llm.int8.onnx")?,
            embedding: require_file(model_dir, "embedding.int8.onnx")?,
            tokenizer_dir: require_dir(model_dir, "Qwen3-0.6B")?,
        }),
        SherpaAsrKind::Qwen3Asr { .. } => Ok(SherpaAsrPaths::Qwen3Asr {
            conv_frontend: require_file(model_dir, "conv_frontend.onnx")?,
            encoder: require_file(model_dir, "encoder.int8.onnx")?,
            decoder: require_file(model_dir, "decoder.int8.onnx")?,
            tokenizer_dir: require_dir(model_dir, "tokenizer")?,
        }),
    }
}

/// 校验文件存在（Fail Fast：缺失即 `ModelNotFound`，附目录上下文）。
fn require_file(dir: &Path, name: &str) -> Result<PathBuf, SttError> {
    let p = dir.join(name);
    if !p.is_file() {
        return Err(SttError::ModelNotFound {
            model: format!("{name} 缺失：{}（模型目录 {}）", p.display(), dir.display()),
        });
    }
    Ok(p)
}

/// 校验子目录存在（tokenizer 目录等）。
fn require_dir(dir: &Path, name: &str) -> Result<PathBuf, SttError> {
    let p = dir.join(name);
    if !p.is_dir() {
        return Err(SttError::ModelNotFound {
            model: format!(
                "{name}/ 目录缺失：{}（模型目录 {}）",
                p.display(),
                dir.display()
            ),
        });
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 造一个 FireRedASR2 目录骨架。
    fn fixture_fireredasr2() -> (PathBuf, TempDir) {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join(DEFAULT_FIREREDASR2_DIR);
        fs::create_dir_all(&root).unwrap();
        for f in ["encoder.int8.onnx", "decoder.int8.onnx", "tokens.txt"] {
            fs::write(root.join(f), b"dummy").unwrap();
        }
        (root, tmp)
    }

    #[test]
    fn resolve_fireredasr2_ok() {
        let (root, _g) = fixture_fireredasr2();
        let p = resolve_paths(&SherpaAsrKind::FireRedAsr2, &root).unwrap();
        match p {
            SherpaAsrPaths::FireRedAsr2 {
                encoder,
                decoder,
                tokens,
            } => {
                assert!(encoder.ends_with("encoder.int8.onnx"));
                assert!(decoder.ends_with("decoder.int8.onnx"));
                assert!(tokens.ends_with("tokens.txt"));
            }
            _ => panic!("expected FireRedAsr2"),
        }
    }

    #[test]
    fn resolve_missing_file_fail_fast() {
        let (root, _g) = fixture_fireredasr2();
        fs::remove_file(root.join("decoder.int8.onnx")).unwrap();
        let err = resolve_paths(&SherpaAsrKind::FireRedAsr2, &root).unwrap_err();
        assert!(err.to_string().contains("decoder.int8.onnx"));
    }

    #[test]
    fn resolve_missing_dir_fail_fast() {
        let tmp = TempDir::new().unwrap();
        // 传一个不存在的子路径 → 命中"目录不存在"分支（不是 require_file）
        let missing = tmp.path().join("does-not-exist");
        let err = resolve_paths(&SherpaAsrKind::FireRedAsr2, &missing).unwrap_err();
        assert!(err.to_string().contains("不存在"));
    }

    #[test]
    fn resolve_funasr_needs_tokenizer_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(DEFAULT_FUNASRNANO_DIR);
        fs::create_dir_all(&root).unwrap();
        for f in [
            "encoder_adaptor.int8.onnx",
            "llm.int8.onnx",
            "embedding.int8.onnx",
        ] {
            fs::write(root.join(f), b"dummy").unwrap();
        }
        // 缺 Qwen3-0.6B/ 目录 → Fail Fast
        let err = resolve_paths(&SherpaAsrKind::FunAsrNano { hotwords: None }, &root).unwrap_err();
        assert!(err.to_string().contains("Qwen3-0.6B"));
        // 补上目录后成功
        fs::create_dir_all(root.join("Qwen3-0.6B")).unwrap();
        let p = resolve_paths(&SherpaAsrKind::FunAsrNano { hotwords: None }, &root).unwrap();
        assert!(matches!(p, SherpaAsrPaths::FunAsrNano { .. }));
    }

    #[test]
    fn resolve_model_dir_explicit_wins() {
        let d = resolve_model_dir(
            Path::new("./models"),
            "fireredasr2",
            DEFAULT_FIREREDASR2_DIR,
            Some("/custom/dir"),
        );
        assert_eq!(d, PathBuf::from("/custom/dir"));
    }

    #[test]
    fn resolve_model_dir_default_when_empty() {
        let d = resolve_model_dir(
            Path::new("./models"),
            "fireredasr2",
            DEFAULT_FIREREDASR2_DIR,
            Some("   "),
        );
        assert!(d.ends_with(DEFAULT_FIREREDASR2_DIR));
    }

    #[test]
    fn resolve_model_dir_default_when_none() {
        let d = resolve_model_dir(
            Path::new("./models"),
            "qwen3asr",
            DEFAULT_QWEN3ASR_DIR,
            None,
        );
        assert!(d.starts_with("./models"));
        assert!(d.ends_with(DEFAULT_QWEN3ASR_DIR));
    }

    /// 造一个 SenseVoice 目录骨架（int8 + fp32 双档，同 sherpa 发布布局）。
    fn fixture_sensevoice(with_int8: bool) -> (PathBuf, TempDir) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(DEFAULT_SENSEVOICE_DIR);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("tokens.txt"), b"dummy").unwrap();
        fs::write(root.join("model.onnx"), b"dummy").unwrap();
        if with_int8 {
            fs::write(root.join("model.int8.onnx"), b"dummy").unwrap();
        }
        (root, tmp)
    }

    #[test]
    fn quant_parse_defaults_to_int8() {
        assert_eq!(SenseVoiceQuant::parse("").unwrap(), SenseVoiceQuant::Int8);
        assert_eq!(
            SenseVoiceQuant::parse("INT8").unwrap(),
            SenseVoiceQuant::Int8
        );
        assert_eq!(
            SenseVoiceQuant::parse("fp32").unwrap(),
            SenseVoiceQuant::Fp32
        );
        assert_eq!(
            SenseVoiceQuant::parse(" fp16 ").unwrap(),
            SenseVoiceQuant::Fp16
        );
        let err = SenseVoiceQuant::parse("int9").unwrap_err();
        assert!(err.to_string().contains("int9"));
    }

    #[test]
    fn resolve_sensevoice_prefers_int8() {
        let (root, _g) = fixture_sensevoice(true);
        let kind = SherpaAsrKind::SenseVoice {
            language: Some("auto".into()),
            use_itn: true,
            quant: SenseVoiceQuant::Int8,
        };
        match resolve_paths(&kind, &root).unwrap() {
            SherpaAsrPaths::SenseVoice { model, tokens } => {
                assert!(model.ends_with("model.int8.onnx"));
                assert!(tokens.ends_with("tokens.txt"));
            }
            _ => panic!("expected SenseVoice"),
        }
    }

    #[test]
    fn resolve_sensevoice_falls_back_to_fp32_when_int8_absent() {
        let (root, _g) = fixture_sensevoice(false);
        let kind = SherpaAsrKind::SenseVoice {
            language: None,
            use_itn: false,
            quant: SenseVoiceQuant::Int8,
        };
        match resolve_paths(&kind, &root).unwrap() {
            SherpaAsrPaths::SenseVoice { model, .. } => {
                assert!(model.ends_with("model.onnx"));
            }
            _ => panic!("expected SenseVoice"),
        }
    }

    #[test]
    fn resolve_sensevoice_missing_tokens_fail_fast() {
        let (root, _g) = fixture_sensevoice(true);
        fs::remove_file(root.join("tokens.txt")).unwrap();
        let kind = SherpaAsrKind::SenseVoice {
            language: None,
            use_itn: true,
            quant: SenseVoiceQuant::Int8,
        };
        let err = resolve_paths(&kind, &root).unwrap_err();
        assert!(err.to_string().contains("tokens.txt"));
    }
}
