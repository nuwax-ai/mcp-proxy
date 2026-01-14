#!/usr/bin/env python3
"""
简单测试脚本，验证TTS功能
"""
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from tts_service import TTSService

def test_tts():
    print("Testing TTS functionality...")
    
    # 创建TTS服务
    tts = TTSService()
    
    # 测试文本
    test_text = "Hello, this is a test of the TTS system."
    
    # 输出文件
    output_file = "test_tts_output.wav"
    
    try:
        # 执行合成
        result = tts.synthesize_sync(
            text=test_text,
            output_path=output_file,
            speed=1.0,
            pitch=0,
            volume=1.0,
            format="wav"
        )
        
        print(f"Synthesis result: {result}")
        
        if result["success"]:
            print(f"Output file: {result['output_path']}")
            print(f"File size: {result['file_size']} bytes")
            print(f"Duration: {result['duration']} seconds")
            
            # 检查文件是否存在
            if os.path.exists(output_file):
                print(f"File verification successful: {output_file}")
                
                # 获取文件信息
                import stat
                file_stat = os.stat(output_file)
                print(f"File status: {file_stat.st_size} bytes")
                
                return True
            else:
                print(f"File does not exist: {output_file}")
                return False
        else:
            print(f"Synthesis failed: {result['error']}")
            return False
            
    except Exception as e:
        print(f"Test failed: {e}")
        return False

if __name__ == "__main__":
    success = test_tts()
    sys.exit(0 if success else 1)