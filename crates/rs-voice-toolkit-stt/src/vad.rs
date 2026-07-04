//! 语音活动检测 (VAD) 模块
//!
//! 提供简单的语音活动检测功能，用于识别音频中的语音段

/// 简单的VAD实现
#[derive(Debug, Clone)]
pub struct SimpleVad {
    /// 语音检测阈值
    threshold: f32,
    /// 检测窗口大小（样本数）
    window_size: usize,
    #[allow(dead_code)]
    /// 音频采样率 (Hz)
    sample_rate: u32,
}

impl SimpleVad {
    /// 创建新的VAD检测器
    pub fn new(threshold: f32) -> Self {
        Self::new_with_sample_rate(threshold, 16000)
    }

    /// 创建新的VAD检测器，指定采样率
    pub fn new_with_sample_rate(threshold: f32, sample_rate: u32) -> Self {
        let window_size = (sample_rate as f64 * 0.02) as usize; // 20ms窗口
        Self {
            threshold,
            window_size,
            sample_rate,
        }
    }

    /// 检测音频样本中是否包含语音
    pub fn detect_speech(&self, samples: &[f32]) -> bool {
        if samples.is_empty() {
            return false;
        }

        // 计算RMS能量
        let rms = self.calculate_rms(samples);
        rms > self.threshold
    }

    /// 计算音频样本的RMS能量
    fn calculate_rms(&self, samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    /// 检测音频中的语音段
    pub fn detect_speech_segments(&self, samples: &[f32]) -> Vec<(usize, usize)> {
        let mut segments = Vec::new();
        let mut in_speech = false;
        let mut speech_start = 0;

        for (i, chunk) in samples.chunks(self.window_size).enumerate() {
            let has_speech = self.detect_speech(chunk);
            let sample_index = i * self.window_size;

            if has_speech && !in_speech {
                // 语音开始
                speech_start = sample_index;
                in_speech = true;
            } else if !has_speech && in_speech {
                // 语音结束
                segments.push((speech_start, sample_index));
                in_speech = false;
            }
        }

        // 如果音频结束时仍在语音中
        if in_speech {
            segments.push((speech_start, samples.len()));
        }

        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_vad() {
        let vad = SimpleVad::new(0.01);

        // 测试静音
        let silence = vec![0.0; 1000];
        assert!(!vad.detect_speech(&silence));

        // 测试有声音
        let speech = vec![0.1; 1000];
        assert!(vad.detect_speech(&speech));
    }

    #[test]
    fn test_vad_with_custom_sample_rate() {
        let vad = SimpleVad::new_with_sample_rate(0.01, 8000);

        // 测试静音
        let silence = vec![0.0; 500];
        assert!(!vad.detect_speech(&silence));

        // 测试有声音
        let speech = vec![0.1; 500];
        assert!(vad.detect_speech(&speech));
    }

    #[test]
    fn test_detect_speech_segments() {
        let vad = SimpleVad::new(0.05);

        // 创建测试音频：静音-语音-静音-语音-静音
        let mut samples = Vec::new();
        samples.extend(vec![0.0; 1000]); // 静音
        samples.extend(vec![0.1; 1000]); // 语音
        samples.extend(vec![0.0; 1000]); // 静音
        samples.extend(vec![0.1; 1000]); // 语音
        samples.extend(vec![0.0; 1000]); // 静音

        let segments = vad.detect_speech_segments(&samples);

        // 应该检测到两个语音段
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_calculate_rms() {
        let vad = SimpleVad::new(0.01);

        // 测试零信号
        let zeros = vec![0.0; 100];
        assert_eq!(vad.calculate_rms(&zeros), 0.0);

        // 测试已知信号
        let ones = vec![1.0; 100];
        assert_eq!(vad.calculate_rms(&ones), 1.0);

        // 测试空数组
        let empty: Vec<f32> = vec![];
        assert_eq!(vad.calculate_rms(&empty), 0.0);
    }
}
