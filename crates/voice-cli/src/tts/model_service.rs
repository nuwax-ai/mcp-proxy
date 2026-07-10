//! TTS 模型管理（sherpa-onnx；Kokoro / ZipVoice 两引擎）。
//!
//! # 模型目录布局
//! **Kokoro**（`{models_dir}/{model_id}/`）：
//! ```text
//! model.onnx voices.bin tokens.txt espeak-ng-data/ dict/(可选) lexicon[-]*.txt
//! ```
//! **ZipVoice**（`{zipvoice.model_dir}` 或 `{models_dir}/{model_id}/`）：
//! ```text
//! encoder.int8.onnx decoder.int8.onnx tokens.txt espeak-ng-data/ lexicon.txt
//! ```
//! ZipVoice 的 vocoder（`vocos_24khz.onnx`）独立下载，`config.zipvoice.vocoder=None` 时探测
//! 同目录 `vocos_24khz.onnx`。
//!
//! # 网络
//! 模型托管在 sherpa-onnx GitHub releases，本环境 HF 阻断；v1 不做自动下载
//! （`auto_download=false`），由部署方手动放置。缺模型时 [`TtsModelService::ensure_model`]
//! 返回 `ModelNotFound`（附手动放置指引，Fail Fast）。

use std::path::{Path, PathBuf};

use crate::models::config::{TtsBackend, TtsEngineConfig, ZipVoiceConfig};
use crate::tts::TtsError;

/// Kokoro 模型文件路径集合。
#[derive(Debug, Clone)]
pub struct KokoroPaths {
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
    /// lexicon：单语 = `lexicon.txt` 路径；多语 = 多个 `lexicon-*.txt` 逗号拼接（C 端约定）
    pub lexicon: Option<String>,
}

/// ZipVoice 模型文件路径集合。
#[derive(Debug, Clone)]
pub struct ZipVoicePaths {
    /// `encoder.int8.onnx`（text model）
    pub encoder: PathBuf,
    /// `decoder.int8.onnx`（flow-matching decoder）
    pub decoder: PathBuf,
    /// `vocos_24khz.onnx`（vocoder；独立于主模型包）
    pub vocoder: PathBuf,
    /// `tokens.txt`
    pub tokens: PathBuf,
    /// `espeak-ng-data/`
    pub data_dir: PathBuf,
    /// `lexicon.txt`（中文；英文用 espeak-ng-data 音素化，可为 None）
    pub lexicon: Option<String>,
}

/// TTS 模型路径（按引擎分；build_engine / Fail-Fast 校验各取所需，编译期保证不串台）。
#[derive(Debug, Clone)]
pub enum TtsModelPaths {
    Kokoro(KokoroPaths),
    ZipVoice(ZipVoicePaths),
}

/// TTS 模型服务：按 model_id + backend 解析模型目录 + 校验必备文件。
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

    /// 确认模型就绪：校验目录 + 必备文件（按 backend）。
    pub fn ensure_model(&self, model_id: &str, engine: &TtsEngineConfig) -> Result<(), TtsError> {
        match engine.backend {
            TtsBackend::Kokoro => self.ensure_kokoro(model_id),
            TtsBackend::ZipVoice => self.ensure_zipvoice(model_id, &engine.zipvoice),
        }
    }

    /// 解析模型路径（不校验存在性，由 [`ensure_model`] 负责）。
    pub fn resolve_paths(
        &self,
        model_id: &str,
        engine: &TtsEngineConfig,
    ) -> Result<TtsModelPaths, TtsError> {
        match engine.backend {
            TtsBackend::Kokoro => self.resolve_kokoro(model_id).map(TtsModelPaths::Kokoro),
            TtsBackend::ZipVoice => self
                .resolve_zipvoice(model_id, &engine.zipvoice)
                .map(TtsModelPaths::ZipVoice),
        }
    }

    /// 校验模型目录是否存在（轻量探测，不做完整文件校验）。
    pub fn model_exists(&self, model_id: &str) -> bool {
        self.model_root(model_id).is_dir()
    }

    // ---- Kokoro ----

    fn ensure_kokoro(&self, model_id: &str) -> Result<(), TtsError> {
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
        let paths = self.resolve_kokoro(model_id)?;
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

    fn resolve_kokoro(&self, model_id: &str) -> Result<KokoroPaths, TtsError> {
        let root = self.model_root(model_id);
        let dict_dir = root.join("dict");

        // lexicon 探测（对齐 sherpa-onnx 官方 kokoro-multi-lang 示例）：
        // - 单语 `lexicon.txt` → 单路径
        // - 多语优先 `lexicon-us-en.txt,lexicon-zh.txt`（不含 gb-en：与 us-en 词表重叠触发 C++ 异常）
        // - 兜底：所有 `lexicon-*.txt` 排序后逗号拼接
        let single = root.join("lexicon.txt");
        let lexicon = if single.is_file() {
            Some(single.to_string_lossy().into_owned())
        } else {
            let prefer = ["lexicon-us-en.txt", "lexicon-zh.txt"]
                .into_iter()
                .map(|n| root.join(n))
                .filter(|p| p.is_file())
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if prefer.len() == 2 {
                Some(prefer.join(","))
            } else {
                let mut files: Vec<String> = std::fs::read_dir(&root)
                    .map_err(|e| TtsError::ModelNotFound {
                        model: format!("读取模型目录失败 {}: {e}", root.display()),
                    })?
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("lexicon-") && n.ends_with(".txt"))
                            .unwrap_or(false)
                    })
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                files.sort();
                if files.is_empty() {
                    None
                } else {
                    Some(files.join(","))
                }
            }
        };

        Ok(KokoroPaths {
            model: root.join("model.onnx"),
            voices: root.join("voices.bin"),
            tokens: root.join("tokens.txt"),
            data_dir: root.join("espeak-ng-data"),
            dict_dir: dict_dir.is_dir().then_some(dict_dir),
            lexicon,
        })
    }

    // ---- ZipVoice ----

    fn ensure_zipvoice(&self, model_id: &str, cfg: &ZipVoiceConfig) -> Result<(), TtsError> {
        // 先校验目录（清晰错误优先），再 resolve 路径（对齐 ensure_kokoro 顺序）
        let root = self.zipvoice_root(model_id, cfg);
        if !root.is_dir() {
            return Err(TtsError::ModelNotFound {
                model: format!(
                    "{}（ZipVoice 模型目录不存在。请从 https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models \
                     下载 sherpa-onnx-zipvoice-distill-int8-zh-en-emilia，解压；另下 vocos_24khz.onnx \
                     放同目录或 config.tts.engine.zipvoice.vocoder 指定）",
                    root.display()
                ),
            });
        }
        let paths = self.resolve_zipvoice(model_id, cfg)?;
        for (name, p) in [
            ("encoder.int8.onnx", &paths.encoder),
            ("decoder.int8.onnx", &paths.decoder),
            ("vocoder (vocos_24khz.onnx)", &paths.vocoder),
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

    /// ZipVoice 模型根目录：优先 `config.zipvoice.model_dir`，否则 `{models_dir}/{model_id}`。
    fn zipvoice_root(&self, model_id: &str, cfg: &ZipVoiceConfig) -> PathBuf {
        cfg.model_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.model_root(model_id))
    }

    fn resolve_zipvoice(
        &self,
        model_id: &str,
        cfg: &ZipVoiceConfig,
    ) -> Result<ZipVoicePaths, TtsError> {
        let root = self.zipvoice_root(model_id, cfg);
        // vocoder：优先 config 指定，否则探测同目录 vocos_24khz.onnx
        let vocoder = cfg
            .vocoder
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("vocos_24khz.onnx"));
        // lexicon：单语 lexicon.txt（英文模型可无）
        let lexicon = {
            let p = root.join("lexicon.txt");
            p.is_file().then(|| p.to_string_lossy().into_owned())
        };
        Ok(ZipVoicePaths {
            encoder: root.join("encoder.int8.onnx"),
            decoder: root.join("decoder.int8.onnx"),
            vocoder,
            tokens: root.join("tokens.txt"),
            data_dir: root.join("espeak-ng-data"),
            lexicon,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::config::{TtsBackend, TtsEngineConfig, ZipVoiceConfig};
    use std::fs;
    use tempfile::TempDir;

    /// 造一个 backend=Kokoro 的 engine config。
    fn kokoro_engine() -> TtsEngineConfig {
        TtsEngineConfig {
            backend: TtsBackend::Kokoro,
            ..Default::default()
        }
    }

    /// 造一个 backend=ZipVoice 的 engine config（model_dir 指向 root）。
    fn zipvoice_engine(root: &Path) -> TtsEngineConfig {
        TtsEngineConfig {
            backend: TtsBackend::ZipVoice,
            zipvoice: ZipVoiceConfig {
                model_dir: Some(root.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// 在 tmp/<model_id>/ 下造一份完整 Kokoro 模型。
    fn kokoro_fixture(
        model_id: &str,
        lexicon_files: &[&str],
        with_dict: bool,
    ) -> (TtsModelService, PathBuf, TempDir) {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join(model_id);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("model.onnx"), b"dummy").unwrap();
        fs::write(root.join("voices.bin"), b"dummy").unwrap();
        fs::write(root.join("tokens.txt"), b"dummy").unwrap();
        fs::create_dir_all(root.join("espeak-ng-data")).unwrap();
        if with_dict {
            fs::create_dir_all(root.join("dict")).unwrap();
        }
        for name in lexicon_files {
            fs::write(root.join(name), b"dummy").unwrap();
        }
        let svc = TtsModelService::new(tmp.path());
        (svc, root, tmp)
    }

    /// 在 tmp/<model_id>/ 下造一份完整 ZipVoice 模型。
    fn zipvoice_fixture(model_id: &str) -> (TtsModelService, PathBuf, TempDir) {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join(model_id);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("encoder.int8.onnx"), b"dummy").unwrap();
        fs::write(root.join("decoder.int8.onnx"), b"dummy").unwrap();
        fs::write(root.join("vocos_24khz.onnx"), b"dummy").unwrap();
        fs::write(root.join("tokens.txt"), b"dummy").unwrap();
        fs::create_dir_all(root.join("espeak-ng-data")).unwrap();
        fs::write(root.join("lexicon.txt"), b"dummy").unwrap();
        let svc = TtsModelService::new(tmp.path());
        (svc, root, tmp)
    }

    #[test]
    fn resolve_kokoro_paths_single_lexicon() {
        let (svc, root, _g) = kokoro_fixture("kokoro", &["lexicon.txt"], true);
        let p = match svc.resolve_paths("kokoro", &kokoro_engine()).unwrap() {
            TtsModelPaths::Kokoro(k) => k,
            _ => panic!("expected Kokoro"),
        };
        assert_eq!(p.model, root.join("model.onnx"));
        assert_eq!(p.voices, root.join("voices.bin"));
        assert_eq!(p.tokens, root.join("tokens.txt"));
        assert_eq!(p.data_dir, root.join("espeak-ng-data"));
        assert_eq!(p.dict_dir.as_deref(), Some(root.join("dict").as_path()));
        assert_eq!(
            p.lexicon.as_deref(),
            Some(root.join("lexicon.txt").to_str().unwrap())
        );
    }

    #[test]
    fn resolve_kokoro_multi_lang_prefers_us_en_zh() {
        let (svc, _root, _g) = kokoro_fixture(
            "kokoro",
            &["lexicon-gb-en.txt", "lexicon-us-en.txt", "lexicon-zh.txt"],
            false,
        );
        let p = match svc.resolve_paths("kokoro", &kokoro_engine()).unwrap() {
            TtsModelPaths::Kokoro(k) => k,
            _ => panic!("expected Kokoro"),
        };
        let lex = p.lexicon.expect("lexicon");
        let parts: Vec<&str> = lex.split(',').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().any(|s| s.ends_with("lexicon-us-en.txt")));
        assert!(parts.iter().any(|s| s.ends_with("lexicon-zh.txt")));
        assert!(!lex.contains("gb-en"));
    }

    #[test]
    fn resolve_zipvoice_paths_defaults() {
        let (svc, root, _g) = zipvoice_fixture("zipvoice");
        let p = match svc
            .resolve_paths("zipvoice", &zipvoice_engine(&root))
            .unwrap()
        {
            TtsModelPaths::ZipVoice(z) => z,
            _ => panic!("expected ZipVoice"),
        };
        assert_eq!(p.encoder, root.join("encoder.int8.onnx"));
        assert_eq!(p.decoder, root.join("decoder.int8.onnx"));
        assert_eq!(p.vocoder, root.join("vocos_24khz.onnx"));
        assert_eq!(p.tokens, root.join("tokens.txt"));
        assert_eq!(p.data_dir, root.join("espeak-ng-data"));
        assert_eq!(
            p.lexicon.as_deref(),
            Some(root.join("lexicon.txt").to_str().unwrap())
        );
    }

    #[test]
    fn resolve_zipvoice_vocoder_override() {
        let (svc, root, _g) = zipvoice_fixture("zipvoice");
        let vocoder_elsewhere = root.join("elsewhere_vocoder.onnx");
        fs::write(&vocoder_elsewhere, b"dummy").unwrap();
        let engine = TtsEngineConfig {
            backend: TtsBackend::ZipVoice,
            zipvoice: ZipVoiceConfig {
                model_dir: Some(root.to_string_lossy().into_owned()),
                vocoder: Some(vocoder_elsewhere.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let p = match svc.resolve_paths("zipvoice", &engine).unwrap() {
            TtsModelPaths::ZipVoice(z) => z,
            _ => panic!("expected ZipVoice"),
        };
        assert_eq!(p.vocoder, vocoder_elsewhere);
    }

    #[test]
    fn ensure_kokoro_ok_when_complete() {
        let (svc, _root, _g) = kokoro_fixture("kokoro", &["lexicon.txt"], true);
        assert!(svc.ensure_model("kokoro", &kokoro_engine()).is_ok());
    }

    #[test]
    fn ensure_zipvoice_ok_when_complete() {
        let (svc, root, _g) = zipvoice_fixture("zipvoice");
        assert!(
            svc.ensure_model("zipvoice", &zipvoice_engine(&root))
                .is_ok()
        );
    }

    #[test]
    fn ensure_zipvoice_missing_vocoder_detected() {
        let (svc, root, _g) = zipvoice_fixture("zipvoice");
        fs::remove_file(root.join("vocos_24khz.onnx")).unwrap();
        let err = svc
            .ensure_model("zipvoice", &zipvoice_engine(&root))
            .unwrap_err();
        assert!(matches!(err, TtsError::ModelNotFound { .. }));
        assert!(err.to_string().contains("vocoder"));
    }

    #[test]
    fn ensure_model_missing_dir() {
        let tmp = TempDir::new().unwrap();
        let svc = TtsModelService::new(tmp.path());
        let err = svc.ensure_model("nope", &kokoro_engine()).unwrap_err();
        assert!(matches!(err, TtsError::ModelNotFound { .. }));
    }

    #[test]
    fn model_exists_lightweight_probe() {
        let (svc, _root, _g) = kokoro_fixture("kokoro", &[], false);
        assert!(svc.model_exists("kokoro"));
        assert!(!svc.model_exists("missing"));
    }
}
