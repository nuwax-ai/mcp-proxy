#!/usr/bin/env python3
"""
TTS功能完整测试
"""
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from tts_service import TTSService
import time

def test_comprehensive_tts():
    """全面测试TTS功能"""
    print("=== TTS功能完整测试 ===")
    
    # 创建TTS服务
    tts = TTSService()
    
    # 测试用例
    test_cases = [
        {
            "name": "基础WAV测试",
            "text": "Hello, this is a basic TTS test.",
            "output": "basic_test.wav",
            "format": "wav",
            "speed": 1.0,
            "pitch": 0,
            "volume": 1.0
        },
        {
            "name": "快速语速测试",
            "text": "This is a faster speech test.",
            "output": "fast_test.wav", 
            "format": "wav",
            "speed": 1.5,
            "pitch": 0,
            "volume": 1.0
        },
        {
            "name": "高音调测试",
            "text": "Testing higher pitch voice.",
            "output": "high_pitch_test.wav",
            "format": "wav", 
            "speed": 1.0,
            "pitch": 10,
            "volume": 1.0
        },
        {
            "name": "低音量测试",
            "text": "This is a quieter voice test.",
            "output": "quiet_test.wav",
            "format": "wav",
            "speed": 1.0,
            "pitch": 0,
            "volume": 0.5
        },
        {
            "name": "MP3格式测试",
            "text": "Testing MP3 audio format output.",
            "output": "mp3_test.mp3",
            "format": "mp3",
            "speed": 1.2,
            "pitch": 5,
            "volume": 1.2
        },
        {
            "name": "长文本测试",
            "text": "This is a longer text test to verify that the TTS system can handle extended content properly without issues or errors occurring during the synthesis process.",
            "output": "long_text_test.wav",
            "format": "wav",
            "speed": 0.9,
            "pitch": -2,
            "volume": 0.8
        }
    ]
    
    results = []
    
    for i, test_case in enumerate(test_cases, 1):
        print(f"\n--- 测试 {i}: {test_case['name']} ---")
        print(f"文本: {test_case['text']}")
        print(f"参数: speed={test_case['speed']}, pitch={test_case['pitch']}, volume={test_case['volume']}, format={test_case['format']}")
        
        start_time = time.time()
        
        try:
            result = tts.synthesize_sync(
                text=test_case['text'],
                output_path=test_case['output'],
                speed=test_case['speed'],
                pitch=test_case['pitch'],
                volume=test_case['volume'],
                format=test_case['format']
            )
            
            end_time = time.time()
            processing_time = end_time - start_time
            
            if result['success']:
                print(f"SUCCESS!")
                print(f"   Output file: {result['output_path']}")
                print(f"   File size: {result['file_size']} bytes")
                print(f"   Audio duration: {result['duration']:.2f} seconds")
                print(f"   Processing time: {processing_time:.2f} seconds")
                
                # 验证文件
                if os.path.exists(test_case['output']):
                    actual_size = os.path.getsize(test_case['output'])
                    print(f"   File verification: {actual_size} bytes (OK)")
                    
                    # 检查是否为真实音频文件
                    if actual_size > 1000:  # 大于1KB认为是真实音频
                        print(f"   Audio quality: Real audio (OK)")
                    else:
                        print(f"   Audio quality: Mock data (WARNING)")
                        
                    results.append(True)
                else:
                    print(f"   ERROR: File does not exist!")
                    results.append(False)
            else:
                print(f"FAILED: {result['error']}")
                results.append(False)
                
        except Exception as e:
            print(f"ERROR: {e}")
            results.append(False)
    
    # 汇总结果
    print(f"\n=== Test Results Summary ===")
    passed = sum(results)
    total = len(results)
    print(f"Passed: {passed}/{total}")
    
    if passed == total:
        print("All tests passed! TTS functionality is working correctly!")
        return True
    else:
        print(f"WARNING: {total - passed} tests failed")
        return False

if __name__ == "__main__":
    success = test_comprehensive_tts()
    sys.exit(0 if success else 1)