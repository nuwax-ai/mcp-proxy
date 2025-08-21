# 如何使用OSS签名URL上传文件

## 概述

您已经成功获取到了OSS上传签名URL，现在可以使用这个URL直接上传文件到阿里云OSS，无需再次经过我们的服务器。

## 获取到的响应信息

```json
{
    "code": "0000",
    "message": "操作成功",
    "data": {
        "upload_url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md?Expires=1755523578&OSSAccessKeyId=LTAI5tJX2MNkCtoTGEMM2wSa&Signature=tgfK1xKmZ%2BGcgQrcSfyX9h%2Bdfo4%3D",
        "oss_file_name": "edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md",
        "oss_bucket": "nuwa-packages",
        "expires_in_hours": 4,
        "content_type": "application/octet-stream"
    }
}
```

## 字段说明

- **upload_url**: 预签名的上传URL，有效期4小时
- **oss_file_name**: 文件在OSS中的完整路径
- **oss_bucket**: OSS存储桶名称
- **expires_in_hours**: URL有效期（小时）
- **content_type**: 文件的MIME类型

## 使用方法

### 1. 使用curl命令上传

```bash
curl -X PUT \
  "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md?Expires=1755523578&OSSAccessKeyId=LTAI5tJX2MNkCtoTGEMM2wSa&Signature=tgfK1xKmZ%2BGcgQrcSfyX9h%2Bdfo4%3D" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @your-file.md
```

### 2. 使用JavaScript/Fetch API

```javascript
async function uploadFile(file, uploadUrl) {
    try {
        const response = await fetch(uploadUrl, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/octet-stream'
            },
            body: file
        });
        
        if (response.ok) {
            console.log('文件上传成功');
            return true;
        } else {
            console.error('上传失败:', response.status, response.statusText);
            return false;
        }
    } catch (error) {
        console.error('上传错误:', error);
        return false;
    }
}

// 使用示例
const fileInput = document.getElementById('fileInput');
const file = fileInput.files[0];
const uploadUrl = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md?Expires=1755523578&OSSAccessKeyId=LTAI5tJX2MNkCtoTGEMM2wSa&Signature=tgfK1xKmZ%2BGcgQrcSfyX9h%2Bdfo4%3D";

uploadFile(file, uploadUrl);
```

### 3. 使用Python requests

```python
import requests

def upload_file(file_path, upload_url):
    try:
        with open(file_path, 'rb') as file:
            headers = {
                'Content-Type': 'application/octet-stream'
            }
            
            response = requests.put(upload_url, data=file, headers=headers)
            
            if response.status_code == 200:
                print('文件上传成功')
                return True
            else:
                print(f'上传失败: {response.status_code} {response.text}')
                return False
                
    except Exception as e:
        print(f'上传错误: {e}')
        return False

# 使用示例
upload_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md?Expires=1755523578&OSSAccessKeyId=LTAI5tJX2MNkCtoTGEMM2wSa&Signature=tgfK1xKmZ%2BGcgQrcSfyX9h%2Bdfo4%3D"
file_path = "your-file.md"

upload_file(file_path, upload_url)
```

### 4. 使用HTML表单

```html
<!DOCTYPE html>
<html>
<head>
    <title>OSS文件上传</title>
</head>
<body>
    <form id="uploadForm">
        <input type="file" id="fileInput" accept=".md,.txt,.pdf,.doc,.docx" required>
        <button type="submit">上传文件</button>
    </form>
    
    <script>
        document.getElementById('uploadForm').addEventListener('submit', async function(e) {
            e.preventDefault();
            
            const fileInput = document.getElementById('fileInput');
            const file = fileInput.files[0];
            
            if (!file) {
                alert('请选择文件');
                return;
            }
            
            const uploadUrl = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md?Expires=1755523578&OSSAccessKeyId=LTAI5tJX2MNkCtoTGEMM2wSa&Signature=tgfK1xKmZ%2BGcgQrcSfyX9h%2Bdfo4%3D";
            
            try {
                const response = await fetch(uploadUrl, {
                    method: 'PUT',
                    headers: {
                        'Content-Type': 'application/octet-stream'
                    },
                    body: file
                });
                
                if (response.ok) {
                    alert('文件上传成功！');
                } else {
                    alert('上传失败: ' + response.status);
                }
            } catch (error) {
                alert('上传错误: ' + error.message);
            }
        });
    </script>
</body>
</html>
```

## 重要注意事项

### 1. HTTP方法
- **必须使用 PUT 方法**，不是 POST
- 直接将文件内容作为请求体发送

### 2. Content-Type
- 必须设置正确的 Content-Type 头
- 本例中为 `application/octet-stream`
- 如果文件类型已知，可以使用更具体的MIME类型

### 3. URL有效期
- 签名URL有效期为4小时
- 超过有效期需要重新获取签名URL

### 4. 文件大小限制
- 请确保文件大小在允许范围内
- 大文件可能需要分片上传（需要额外实现）

### 5. 错误处理
- 200状态码表示上传成功
- 其他状态码表示上传失败，需要检查错误信息

## 上传成功后

文件上传成功后，您可以：

1. **获取下载URL**: 使用 `/api/v1/oss/download-sign-url` 接口获取下载链接
2. **文档解析**: 如果是文档文件，可以使用 `/api/v1/documents/oss` 接口进行解析
3. **文件管理**: 文件将保存在 `edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md` 路径下

## 相关API接口

- `GET /api/v1/oss/upload-sign-url` - 获取上传签名URL
- `GET /api/v1/oss/download-sign-url` - 获取下载签名URL  
- `POST /api/v1/oss/upload` - 直接上传文件（通过服务器）
- `POST /api/v1/documents/oss` - OSS文档解析

## 示例完整流程

```bash
# 1. 获取上传签名URL
curl -X GET "http://localhost:3000/api/v1/oss/upload-sign-url?file_name=test.md&content_type=text/markdown"

# 2. 使用返回的URL上传文件
curl -X PUT "<返回的upload_url>" \
  -H "Content-Type: text/markdown" \
  --data-binary @test.md

# 3. 上传成功后，可以进行文档解析
curl -X POST "http://localhost:3000/api/v1/documents/oss" \
  -H "Content-Type: application/json" \
  -d '{
    "oss_path": "edu/0198bc58-8e47-7d04-93d4-d0cdc0ae1a28.md",
    "format": "Md",
    "enable_toc": true
  }'
```

这样就完成了整个文件上传和处理流程！