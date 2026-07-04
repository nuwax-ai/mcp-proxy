fixtures 目录
=============

此目录用于存放测试所需的最小模型与音频样例。

获取方法：

```bash
./fixtures/get-fixtures.sh
```

完成后会生成：
- 模型：`fixtures/models/ggml-tiny.bin`
- 音频：`fixtures/audio/jfk.wav`

随后可运行 STT 示例：

```bash
cargo run -p stt --example transcribe_file -- fixtures/models/ggml-tiny.bin fixtures/audio/jfk.wav
```


