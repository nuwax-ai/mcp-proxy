# Voice CLI API Documentation

## Overview

The Voice CLI API provides speech-to-text transcription services using OpenAI's Whisper models. The service supports multiple audio formats, automatic format conversion, and various Whisper model sizes for different accuracy/speed trade-offs.

## Base URL

- **Development**: `http://localhost:8080`
- **Production**: `https://api.voice-cli.dev`

## Interactive Documentation

Once the server is running, you can access the interactive Swagger UI documentation at:

- **Swagger UI**: `http://localhost:8080/swagger-ui/`
- **OpenAPI JSON**: `http://localhost:8080/api-docs/openapi.json`

## Authentication

Currently, the API does not require authentication. This may change in future versions.

## Supported Audio Formats

The API supports the following audio formats with automatic conversion:

- **MP3** (.mp3)
- **WAV** (.wav) - preferred format
- **FLAC** (.flac)
- **M4A** (.m4a)
- **AAC** (.aac)
- **OGG** (.ogg)

## Whisper Models

The following Whisper models are supported:

| Model | Size | Languages | Speed | Accuracy | Use Case |
|-------|------|-----------|-------|----------|-----------|
| tiny | ~39 MB | EN/Multi | Fastest | Lowest | Real-time, low-resource |
| tiny.en | ~39 MB | English | Fastest | Lowest | Real-time English only |
| base | ~142 MB | EN/Multi | Fast | Good | General purpose |
| base.en | ~142 MB | English | Fast | Good | General English |
| small | ~244 MB | EN/Multi | Medium | Better | Higher accuracy needs |
| small.en | ~244 MB | English | Medium | Better | Higher accuracy English |
| medium | ~769 MB | EN/Multi | Slow | High | Professional transcription |
| medium.en | ~769 MB | English | Slow | High | Professional English |
| large-v1 | ~1.5 GB | Multi | Slowest | Highest | Best quality multilingual |
| large-v2 | ~1.5 GB | Multi | Slowest | Highest | Best quality multilingual |
| large-v3 | ~1.5 GB | Multi | Slowest | Highest | Latest best quality |

## API Endpoints

### 1. Health Check

#### `GET /health`

Returns the current health status of the service.

**Response Example:**
```json
{
  \"status\": \"healthy\",
  \"models_loaded\": [\"base\", \"small\"],
  \"uptime\": 3600,
  \"version\": \"0.1.0\"
}
```

**cURL Example:**
```bash
curl -X GET http://localhost:8080/health
```

### 2. List Models

#### `GET /models`

Returns information about available and loaded models.

**Response Example:**
```json
{
  \"available_models\": [\"tiny\", \"base\", \"small\", \"medium\", \"large-v3\"],
  \"loaded_models\": [\"base\"],
  \"model_info\": {
    \"base\": {
      \"size\": \"142 MB\",
      \"memory_usage\": \"388 MB\",
      \"status\": \"loaded\"
    }
  }
}
```

**cURL Example:**
```bash
curl -X GET http://localhost:8080/models
```

### 3. Transcribe Audio

#### `POST /transcribe`

Transcribes an audio file to text using Whisper models.

**Content-Type:** `multipart/form-data`
**Max File Size:** 200MB

**Form Parameters:**

| Parameter | Type | Required | Description | Example |
|-----------|------|----------|-------------|---------|
| `audio` | file | Yes | Audio file to transcribe | audio.mp3 |
| `model` | string | No | Whisper model to use | \"base\" |
| `language` | string | No | Language hint (ISO 639-1) | \"en\" |
| `response_format` | string | No | Output format | \"json\" |

**Response Format Options:**
- `json` (default): Structured JSON with segments
- `text`: Plain text only
- `verbose_json`: JSON with detailed information

**Response Example (JSON format):**
```json
{
  \"text\": \"Hello, this is a test transcription of the audio file.\",
  \"segments\": [
    {
      \"start\": 0.0,
      \"end\": 2.5,
      \"text\": \"Hello, this is a test transcription\",
      \"confidence\": 0.95
    },
    {
      \"start\": 2.5,
      \"end\": 4.0,
      \"text\": \"of the audio file.\",
      \"confidence\": 0.92
    }
  ],
  \"language\": \"en\",
  \"duration\": 4.0,
  \"processing_time\": 1.2
}
```

**cURL Examples:**

**Basic transcription:**
```bash
curl -X POST http://localhost:8080/transcribe \\n  -F \"audio=@example.mp3\"
```

**With specific model and language:**
```bash
curl -X POST http://localhost:8080/transcribe \\n  -F \"audio=@example.wav\" \\n  -F \"model=small\" \\n  -F \"language=en\"
```

**Text-only response:**
```bash
curl -X POST http://localhost:8080/transcribe \\n  -F \"audio=@example.flac\" \\n  -F \"response_format=text\"
```

**JavaScript/Fetch Example:**
```javascript
const formData = new FormData();
formData.append('audio', audioFile);
formData.append('model', 'base');
formData.append('language', 'en');

fetch('http://localhost:8080/transcribe', {
  method: 'POST',
  body: formData
})
.then(response => response.json())
.then(data => {
  console.log('Transcription:', data.text);
  console.log('Processing time:', data.processing_time);
})
.catch(error => console.error('Error:', error));
```

**Python Example:**
```python
import requests

with open('audio.mp3', 'rb') as audio_file:
    files = {'audio': audio_file}
    data = {
        'model': 'base',
        'language': 'en',
        'response_format': 'json'
    }
    
    response = requests.post(
        'http://localhost:8080/transcribe',
        files=files,
        data=data
    )
    
    if response.status_code == 200:
        result = response.json()
        print(f\"Transcription: {result['text']}\")
        print(f\"Processing time: {result['processing_time']}s\")
    else:
        print(f\"Error: {response.status_code} - {response.text}\")
```

## Error Responses

All endpoints return structured error responses:

```json
{
  \"error\": \"Error description\",
  \"status\": 400
}
```

**Common Error Codes:**

- `400 Bad Request`: Missing required fields, invalid parameters
- `413 Payload Too Large`: File exceeds 200MB limit
- `415 Unsupported Media Type`: Unsupported audio format
- `500 Internal Server Error`: Server-side processing error

## Rate Limits

Currently, there are no rate limits imposed. This may change in future versions based on usage patterns.

## Best Practices

1. **Audio Quality**: Use high-quality audio (16kHz or higher) for better transcription accuracy
2. **File Size**: Keep files under 200MB for optimal performance
3. **Model Selection**: 
   - Use `tiny` or `base` for real-time applications
   - Use `small` or `medium` for better accuracy
   - Use `large-v3` for the highest quality transcription
4. **Language Hints**: Provide language hints when known for better accuracy
5. **Format**: WAV format is preferred for fastest processing

## Language Support

Whisper supports 99+ languages. Common language codes:

- `en` - English
- `zh` - Chinese
- `es` - Spanish
- `fr` - French
- `de` - German
- `ja` - Japanese
- `ko` - Korean
- `pt` - Portuguese
- `ru` - Russian
- `ar` - Arabic

## Troubleshooting

### Common Issues

1. **\"File too large\" error**: Ensure your audio file is under 200MB
2. **\"Unsupported format\" error**: Use supported audio formats (MP3, WAV, FLAC, etc.)
3. **Slow processing**: Try using a smaller model like `tiny` or `base`
4. **Poor accuracy**: Use a larger model and provide language hints

### Server Logs

Check server logs for detailed error information:
```bash
# View real-time logs
tail -f logs/voice-cli.log

# Check service status
voice-cli server status
```

## SDK and Libraries

Official SDKs and community libraries:

- **JavaScript/TypeScript**: Coming soon
- **Python**: Coming soon
- **Go**: Coming soon
- **cURL**: Use examples above

## Support

For support and questions:

- **GitHub Issues**: [Voice CLI Issues](https://github.com/your-org/voice-cli/issues)
- **Documentation**: This API documentation
- **Email**: support@voice-cli.dev

## Changelog

### v0.1.0 (Current)
- Initial API release
- Basic transcription functionality
- Support for multiple audio formats
- Whisper model management
- OpenAPI documentation