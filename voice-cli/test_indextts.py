#!/usr/bin/env python3
"""
IndexTTS 测试脚本
"""
import os
import sys
import subprocess
import tempfile

def test_indextts():
    print("开始测试 IndexTTS...")
    
    # 检查模型文件
    required_files = ['checkpoints/config.yaml', 'checkpoints/gpt.pth']
    for file in required_files:
        if not os.path.exists(file):
            print(f"错误: 缺少模型文件 {file}")
            return False
    
    # 检查参考语音文件
    if not os.path.exists('reference_voice.wav'):
        print("错误: 缺少参考语音文件 reference_voice.wav")
        return False
    
    try:
        # 测试合成
        test_text = "你好，这是一个测试。"
        output_file = "test_output.wav"
        
        cmd = [
            'indextts',
            test_text,
            '--voice', 'reference_voice.wav',
            '--output_path', output_file,
            '--model_dir', 'checkpoints',
            '--config', 'checkpoints/config.yaml',
            '--force'
        ]
        
        print(f"执行命令: {' '.join(cmd)}")
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        if result.returncode == 0 and os.path.exists(output_file):
            print(f"测试成功! 输出文件: {output_file}")
            print(f"文件大小: {os.path.getsize(output_file)} bytes")
            return True
        else:
            print(f"测试失败")
            print(f"返回码: {result.returncode}")
            print(f"输出: {result.stdout}")
            print(f"错误: {result.stderr}")
            return False
            
    except Exception as e:
        print(f"测试异常: {e}")
        return False

if __name__ == "__main__":
    success = test_indextts()
    sys.exit(0 if success else 1)