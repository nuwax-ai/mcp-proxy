import axios from 'npm:axios';

// 使用axios发起HTTP请求的函数
async function fetchData(url) {
    console.log(`正在请求: ${url}`);
    try {
        const response = await axios.get(url);
        console.log(`请求成功，状态码: ${response.status}`);
        return response.data;
    } catch (error) {
        console.error(`请求失败: ${error.message}`);
        return { error: error.message };
    }
}

export default async function handler(input) {
    // 默认URL或从输入参数获取
    const url = input.url || 'https://jsonplaceholder.typicode.com/todos/1';
    
    console.log(`开始处理请求，URL: ${url}`);
    
    // 发起请求并获取数据
    const data = await fetchData(url);
    
    return {
        message: '请求完成',
        data: data,
        timestamp: new Date().toISOString()
    };
} 