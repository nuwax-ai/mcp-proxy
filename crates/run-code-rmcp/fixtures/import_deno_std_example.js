import { join, basename } from "https://deno.land/std/path/mod.ts";
import { readLines } from "https://deno.land/std/io/mod.ts";
import { format } from "https://deno.land/std/datetime/mod.ts";

// 使用Deno标准库处理路径和文件
async function processPath(path) {
    console.log(`处理路径: ${path}`);
    
    // 使用path模块处理路径
    const fileName = basename(path);
    console.log(`文件名: ${fileName}`);
    
    const fullPath = join("/tmp", fileName);
    console.log(`完整路径: ${fullPath}`);
    
    // 获取当前时间并格式化
    const now = new Date();
    const formattedDate = format(now, "yyyy-MM-dd HH:mm:ss");
    console.log(`当前时间: ${formattedDate}`);
    
    return {
        originalPath: path,
        fileName: fileName,
        fullPath: fullPath,
        timestamp: formattedDate
    };
}

// 模拟读取文件内容
async function simulateFileReading() {
    const text = `这是第一行
这是第二行
这是第三行
这是最后一行`;
    
    // 使用TextEncoder和StringReader代替Deno.Buffer
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    const data = encoder.encode(text);
    
    // 创建一个可读流
    const stream = new ReadableStream({
        start(controller) {
            controller.enqueue(data);
            controller.close();
        }
    });
    
    // 将流转换为Reader
    const reader = stream.getReader();
    
    console.log("开始读取文件内容:");
    
    // 手动处理文本行
    const lines = text.split('\n');
    for (let i = 0; i < lines.length; i++) {
        console.log(`第${i+1}行: ${lines[i]}`);
    }
    
    return lines;
}

export default async function handler(input) {
    console.log("开始处理");
    
    // 处理路径
    const path = input.path || "example.txt";
    const pathInfo = await processPath(path);
    
    // 模拟读取文件
    const fileLines = await simulateFileReading();
    
    return {
        message: '处理完成',
        pathInfo: pathInfo,
        fileContent: fileLines,
        linesCount: fileLines.length
    };
} 