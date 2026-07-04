#!/usr/bin/env python3
"""
Mock TTS service for testing purposes
"""

import os
import sys
import tempfile
from pathlib import Path

def create_mock_audio_file(text, output_path, format="mp3"):
    """Create a mock audio file for testing"""
    try:
        # Ensure output directory exists
        output_dir = Path(output_path).parent
        output_dir.mkdir(parents=True, exist_ok=True)
        
        # Create a mock audio file (just empty file for testing)
        with open(output_path, 'wb') as f:
            # Write some mock audio data (just zeros for testing)
            f.write(b'\x00' * 1024)  # 1KB of mock data
        
        return {
            "success": True,
            "output_path": output_path,
            "file_size": 1024,
            "duration": 1.0,
            "format": format
        }
    except Exception as e:
        return {
            "success": False,
            "error": str(e)
        }

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python mock_tts_service.py <text> [--output OUTPUT] [--format FORMAT]")
        sys.exit(1)
    
    text = sys.argv[1]
    output_path = None
    format = "mp3"
    
    # Parse arguments
    for i in range(2, len(sys.argv)):
        if sys.argv[i] == "--output" and i + 1 < len(sys.argv):
            output_path = sys.argv[i + 1]
        elif sys.argv[i] == "--format" and i + 1 < len(sys.argv):
            format = sys.argv[i + 1]
    
    # Use temporary file if no output specified
    if not output_path:
        with tempfile.NamedTemporaryFile(suffix=f".{format}", delete=False) as f:
            output_path = f.name
    
    # Create mock audio file
    result = create_mock_audio_file(text, output_path, format)
    
    if result["success"]:
        print(f"Mock TTS synthesis completed successfully!")
        print(f"Output file: {result['output_path']}")
        print(f"File size: {result['file_size']} bytes")
        print(f"Duration: {result['duration']} seconds")
    else:
        print(f"Mock TTS synthesis failed: {result['error']}")
        sys.exit(1)