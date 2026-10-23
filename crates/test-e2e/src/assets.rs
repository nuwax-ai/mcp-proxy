//! 零外部文件依赖的测试资产生成器（最小合法 PDF / WAV / PCM）。

/// 最小合法 PDF（单页 Helvetica 文本，正确计算 xref 偏移）——MinerU 真实解析用。
/// 内容刻意含多行文本，保证提取结果非空可断言。
pub fn minimal_pdf() -> Vec<u8> {
    let lines = [
        "E2E Parse Verification Report",
        "MinerU engine integration test line two.",
        "Third line for content extraction assertions.",
    ];
    let mut content = String::from("BT /F1 18 Tf 72 720 Td\n");
    for (i, l) in lines.iter().enumerate() {
        let esc = l.replace('(', "\\(").replace(')', "\\)");
        if i + 1 < lines.len() {
            content.push_str(&format!("({esc}) Tj\n0 -28 Td\n"));
        } else {
            content.push_str(&format!("({esc}) Tj\n"));
        }
    }
    content.push_str("ET\n");

    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes(),
    );
    out
}

/// 超过最小文件大小校验阈值的 Markdown（document-parser E013 防呆回归用；
/// 太小的文件会被"文件过小，可能已损坏"拦截）
pub fn markdown_asset() -> Vec<u8> {
    "# E2E 核心接口验证\n\n这是端到端集成测试生成的 Markdown 文档，用于 parse-sync 链路验证。\n\n- 第一项内容\n- 第二项内容\n\n## 补充小节\n\n足够长的正文段落，确保文件大小超过最小文件阈值，不触发 E013 校验拦截。\n"
        .as_bytes()
        .to_vec()
}

/// 正弦波 WAV（16k mono s16le）——sync/async 转写用。纯音调无语音，
/// 转写结果文本可为空/幻听，但链路（解码→模型→响应）必须完整。
pub fn sine_wav(seconds: f32) -> Vec<u8> {
    let sample_rate = 16000usize;
    let n = (sample_rate as f32 * seconds) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin();
        let s = (v * 6000.0) as i16;
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    wav_from_pcm_s16le(&pcm, sample_rate as u32)
}

/// 裸 PCM s16le/16k/mono（WS 流式 STT 二进制帧用）
pub fn sine_pcm_s16le(seconds: f32) -> Vec<u8> {
    let sample_rate = 16000usize;
    let n = (sample_rate as f32 * seconds) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin();
        let s = (v * 6000.0) as i16;
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    pcm
}

fn wav_from_pcm_s16le(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_is_valid_shape() {
        let pdf = minimal_pdf();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        assert!(pdf.len() > 500, "PDF 应含完整对象结构");
    }

    #[test]
    fn wav_header_shape() {
        let wav = sine_wav(0.1);
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        // 0.1s * 16000 * 2 bytes = 3200 数据字节
        assert_eq!(wav.len(), 44 + 3200);
    }
}
