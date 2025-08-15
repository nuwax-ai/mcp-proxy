#!/bin/bash

# Document Parser API 快速测试脚本
# 使用curl命令测试核心API功能

BASE_URL="http://localhost:8087"
COLOR_GREEN="\033[0;32m"
COLOR_RED="\033[0;31m"
COLOR_YELLOW="\033[1;33m"
COLOR_BLUE="\033[0;34m"
COLOR_RESET="\033[0m"

echo -e "${COLOR_BLUE}🧪 Document Parser API 测试脚本${COLOR_RESET}"
echo -e "${COLOR_BLUE}📍 服务地址: ${BASE_URL}${COLOR_RESET}"
echo ""

# 测试函数
test_api() {
    local name="$1"
    local method="$2"
    local endpoint="$3"
    local data="$4"
    local content_type="$5"
    
    echo -e "${COLOR_YELLOW}🔍 测试: ${name}${COLOR_RESET}"
    echo -e "${COLOR_BLUE}   ${method} ${endpoint}${COLOR_RESET}"
    
    if [ "$method" = "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "${BASE_URL}${endpoint}")
    else
        if [ -n "$content_type" ]; then
            response=$(curl -s -w "\n%{http_code}" -X "$method" "${BASE_URL}${endpoint}" \
                -H "Content-Type: $content_type" \
                -d "$data")
        else
            response=$(curl -s -w "\n%{http_code}" -X "$method" "${BASE_URL}${endpoint}" \
                -d "$data")
        fi
    fi
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n -1)
    
    if [ "$http_code" -ge 200 ] && [ "$http_code" -lt 300 ]; then
        echo -e "   ${COLOR_GREEN}✅ 成功 (${http_code})${COLOR_RESET}"
        echo "   响应: $(echo "$body" | head -c 100)..."
    else
        echo -e "   ${COLOR_RED}❌ 失败 (${http_code})${COLOR_RESET}"
        echo "   错误: $(echo "$body" | head -c 100)..."
    fi
    echo ""
}

# 检查服务器是否运行
echo -e "${COLOR_YELLOW}🔍 检查服务器状态...${COLOR_RESET}"
if ! curl -s "${BASE_URL}/health" > /dev/null; then
    echo -e "${COLOR_RED}❌ 服务器未运行或无法连接到 ${BASE_URL}${COLOR_RESET}"
    echo -e "${COLOR_YELLOW}💡 请先启动服务器: ./test_server.sh${COLOR_RESET}"
    exit 1
fi
echo -e "${COLOR_GREEN}✅ 服务器运行正常${COLOR_RESET}"
echo ""

# 1. 基础健康检查
echo -e "${COLOR_BLUE}📋 1. 基础健康检查${COLOR_RESET}"
test_api "健康检查" "GET" "/health"
test_api "就绪检查" "GET" "/ready"
test_api "支持格式" "GET" "/api/v1/documents/formats"
test_api "解析器健康" "GET" "/api/v1/documents/parser/health"

# 2. Markdown结构化解析测试
echo -e "${COLOR_BLUE}📋 2. Markdown结构化解析测试${COLOR_RESET}"
markdown_data='{
  "markdown_content": "# 测试标题\n\n这是测试内容\n\n## 子标题\n\n更多内容",
  "enable_toc": true,
  "max_toc_depth": 3,
  "enable_anchors": true
}'
test_api "结构化文档生成" "POST" "/api/v1/documents/structured" "$markdown_data" "application/json"

# 3. Markdown章节解析测试
echo -e "${COLOR_BLUE}📋 3. Markdown章节解析测试${COLOR_RESET}"
sections_data='{
  "markdown_content": "# 第一章\n\n第一章内容\n\n## 1.1 小节\n\n小节内容\n\n# 第二章\n\n第二章内容",
  "enable_toc": true,
  "max_toc_depth": 3
}'
test_api "章节解析" "POST" "/api/v1/documents/markdown/sections" "$sections_data" "application/json"

# 4. 任务管理测试
echo -e "${COLOR_BLUE}📋 4. 任务管理测试${COLOR_RESET}"
test_api "任务列表" "GET" "/api/v1/tasks"
test_api "任务统计" "GET" "/api/v1/tasks/stats"

# 5. 文件上传测试（如果test_sample.md存在）
echo -e "${COLOR_BLUE}📋 5. 文件上传测试${COLOR_RESET}"
if [ -f "test_sample.md" ]; then
    echo -e "${COLOR_YELLOW}🔍 测试: 文件上传解析${COLOR_RESET}"
    echo -e "${COLOR_BLUE}   POST /api/v1/documents/upload${COLOR_RESET}"
    
    response=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/api/v1/documents/upload" \
        -F "file=@test_sample.md" \
        -F "format=Md" \
        -F "enable_toc=true" \
        -F "max_toc_depth=3")
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n -1)
    
    if [ "$http_code" -ge 200 ] && [ "$http_code" -lt 300 ]; then
        echo -e "   ${COLOR_GREEN}✅ 成功 (${http_code})${COLOR_RESET}"
        echo "   响应: $(echo "$body" | head -c 100)..."
        
        # 尝试提取task_id
        task_id=$(echo "$body" | grep -o '"task_id":"[^"]*"' | cut -d'"' -f4)
        if [ -n "$task_id" ]; then
            echo -e "   ${COLOR_GREEN}📝 任务ID: ${task_id}${COLOR_RESET}"
            
            # 测试任务查询
            echo ""
            test_api "查询上传任务" "GET" "/api/v1/tasks/${task_id}"
            
            # 等待一下再测试结果获取
            echo -e "${COLOR_YELLOW}⏳ 等待2秒后测试结果获取...${COLOR_RESET}"
            sleep 2
            
            test_api "获取任务目录" "GET" "/api/v1/tasks/${task_id}/toc"
            test_api "获取所有章节" "GET" "/api/v1/tasks/${task_id}/sections"
        fi
    else
        echo -e "   ${COLOR_RED}❌ 失败 (${http_code})${COLOR_RESET}"
        echo "   错误: $(echo "$body" | head -c 100)..."
    fi
    echo ""
else
    echo -e "   ${COLOR_YELLOW}⚠️  test_sample.md 文件不存在，跳过文件上传测试${COLOR_RESET}"
    echo ""
fi

# 6. URL下载测试（使用一个公开的测试URL）
echo -e "${COLOR_BLUE}📋 6. URL下载测试${COLOR_RESET}"
url_data='{
  "url": "https://httpbin.org/robots.txt",
  "format": "Text",
  "enable_toc": false
}'
test_api "URL文档下载" "POST" "/api/v1/documents/download" "$url_data" "application/json"

# 7. 错误处理测试
# 7. PDF文件上传测试
echo -e "${COLOR_BLUE}📋 7. PDF文件上传测试${COLOR_RESET}"
test_pdf_upload() {
    local pdf_file="fixtures/均线为王之一：均线100分.pdf"
    
    echo -e "${COLOR_YELLOW}🔍 测试: PDF文件上传解析${COLOR_RESET}"
    echo -e "${COLOR_BLUE}   POST /api/v1/documents/upload${COLOR_RESET}"
    echo -e "${COLOR_BLUE}   文件: ${pdf_file}${COLOR_RESET}"
    
    if [ ! -f "$pdf_file" ]; then
        echo -e "${COLOR_RED}❌ 错误: 测试文件不存在: $pdf_file${COLOR_RESET}"
        return 1
    fi
    
    response=$(curl -s -w "\n%{http_code}" \
        -X POST "${BASE_URL}/api/v1/documents/upload" \
        -F "file=@${pdf_file}" \
        -F "format=PDF" \
        -F "enable_toc=true" \
        -F "max_toc_depth=3")
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n -1)
    
    if [ "$http_code" -eq 200 ] || [ "$http_code" -eq 201 ]; then
        echo -e "${COLOR_GREEN}✅ 成功 (HTTP $http_code)${COLOR_RESET}"
        # 提取task_id用于后续测试
        task_id=$(echo "$body" | grep -o '"task_id":"[^"]*"' | cut -d'"' -f4)
        if [ -n "$task_id" ]; then
            echo -e "${COLOR_GREEN}📝 任务ID: $task_id${COLOR_RESET}"
            export TEST_TASK_ID="$task_id"
        fi
    else
        echo -e "${COLOR_RED}❌ 失败 (HTTP $http_code)${COLOR_RESET}"
        echo -e "${COLOR_RED}响应: $body${COLOR_RESET}"
    fi
    echo ""
}

test_pdf_upload

echo -e "${COLOR_BLUE}📋 8. 错误处理测试${COLOR_RESET}"
invalid_url_data='{
  "url": "invalid-url",
  "format": "PDF"
}'
test_api "无效URL测试" "POST" "/api/v1/documents/download" "$invalid_url_data" "application/json"

invalid_format_data='{
  "url": "https://httpbin.org/robots.txt",
  "format": "UNSUPPORTED"
}'
test_api "不支持格式测试" "POST" "/api/v1/documents/download" "$invalid_format_data" "application/json"

test_api "无效任务ID测试" "GET" "/api/v1/tasks/invalid-task-id"

# 9. 缓存和统计测试
echo -e "${COLOR_BLUE}📋 9. 缓存和统计测试${COLOR_RESET}"
test_api "解析器统计" "GET" "/api/v1/documents/parser/stats"
test_api "缓存统计" "GET" "/api/v1/documents/processor/cache/stats"

echo -e "${COLOR_GREEN}🎉 测试完成！${COLOR_RESET}"
echo -e "${COLOR_YELLOW}💡 提示:${COLOR_RESET}"
echo -e "   - 使用 test_api.rest 文件进行更详细的测试"
echo -e "   - 查看 TEST_README.md 了解完整测试指南"
echo -e "   - 检查 logs/ 目录查看详细日志"
echo ""