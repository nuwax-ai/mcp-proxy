#!/bin/bash

# 测试文档上传功能的脚本
# 确保服务器正在运行在 localhost:8087

echo "开始测试文档上传功能..."
echo "服务器地址: http://localhost:8087"
echo "测试文件: ./fixtures/均线为王之一：均线100分.pdf"
echo ""

# 检查文件是否存在
if [ ! -f "./fixtures/均线为王之一：均线100分.pdf" ]; then
    echo "错误: 测试文件不存在: ./fixtures/均线为王之一：均线100分.pdf"
    exit 1
fi

echo "文件存在，开始上传测试..."
echo ""

# 执行上传测试
curl -X POST "http://localhost:8087/api/v1/documents/upload?format=PDF&enable_toc=true&max_toc_depth=3" \
     -F "file=@./fixtures/均线为王之一：均线100分.pdf" \
     -H "Content-Type: multipart/form-data" \
     -v

echo ""
echo "测试完成。"
echo "如果返回200状态码和任务ID，说明上传成功。"
echo "如果返回400错误，请检查服务器日志获取详细错误信息。"