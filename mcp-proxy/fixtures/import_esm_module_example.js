// 使用ES模块导入方式
import { createHash } from "node:crypto";
import { Buffer } from "node:buffer";

// 使用Node.js内置模块处理数据
function hashData(data, algorithm = 'sha256') {
    console.log(`使用 ${algorithm} 算法处理数据`);
    
    // 将数据转换为Buffer
    const buffer = typeof data === 'string' 
        ? Buffer.from(data) 
        : Buffer.from(JSON.stringify(data));
    
    // 创建哈希
    const hash = createHash(algorithm);
    hash.update(buffer);
    
    // 获取哈希结果
    const digest = hash.digest('hex');
    console.log(`哈希结果: ${digest}`);
    
    return digest;
}

// 加密数据
function encryptData(data, salt = 'default-salt') {
    console.log(`加密数据，使用盐值: ${salt}`);
    
    // 组合数据和盐值
    const combined = `${data}:${salt}:${Date.now()}`;
    
    // 生成多种哈希
    const sha256Hash = hashData(combined, 'sha256');
    const md5Hash = hashData(combined, 'md5');
    
    return {
        original: data,
        salt,
        sha256: sha256Hash,
        md5: md5Hash,
        timestamp: new Date().toISOString()
    };
}

export default async function handler(input) {
    console.log("开始处理加密任务");
    
    // 获取输入数据
    const data = input.data || "这是需要加密的默认数据";
    const salt = input.salt || "custom-salt-" + Math.floor(Math.random() * 1000);
    
    // 加密数据
    const encryptedResult = encryptData(data, salt);
    
    return {
        message: '加密处理完成',
        result: encryptedResult
    };
} 