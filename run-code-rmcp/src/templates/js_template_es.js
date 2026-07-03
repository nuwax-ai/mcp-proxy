// @ts-nocheck
// ES模块格式，支持import/export语句

// 保存原始console.log
const originalConsoleLog = console.log;
let logs = [];

// 替换console.log以捕获日志
console.log = function() {
    // 将参数转换为字符串并连接它们
    const message = Array.from(arguments).map(arg =>
        typeof arg === 'object' && arg !== null ? JSON.stringify(arg) : String(arg)
    ).join(' ');

    // 存储日志
    logs.push(message);

    // 如果显示日志，也输出到原始控制台
    if ({{SHOW_LOGS}}) {
        originalConsoleLog.apply(console, arguments);
    }
};

// 异步读取输入参数的函数
async function readInputParams() {
    let input = {};
    try {
        const inputFile = Deno.env.get("INPUT_JSON_FILE");
        if (inputFile) {
            const inputJson = await Deno.readTextFile(inputFile);
            input = JSON.parse(inputJson);
            console.log("接收到的参数:", JSON.stringify(input));
        } else {
            // 兼容旧的环境变量方式
            const inputJson = Deno.env.get("INPUT_JSON");
            if (inputJson) {
                input = JSON.parse(inputJson);
                console.log("接收到的参数:", JSON.stringify(input));
            }
        }
    } catch (error) {
        console.error("解析输入参数失败:", error);
    }
    return input;
}

// 用户代码
{{USER_CODE}}

// 执行handler函数并获取结果
let result = null;

// 异步立即执行函数
(async () => {
    // 读取输入参数
    const input = await readInputParams();

    try {
        // 优先检查main函数
        if (typeof main === 'function') {
            // 检查main是否是异步函数
            if (main.constructor.name === 'AsyncFunction') {
                result = await main(input);
            } else {
                result = main(input);
            }
        } else if (typeof handler === 'function') {
            // 如果没有main函数，检查handler
            if (handler.constructor.name === 'AsyncFunction') {
                result = await handler(input);
            } else {
                result = handler(input);
            }
        }else{
            throw new Error("没有找到main或handler函数");
        }

        // 打印最终输出为JSON
        originalConsoleLog(JSON.stringify({
            logs: logs,
            result: result !== undefined ? (typeof result === 'object' ? JSON.stringify(result) : String(result)) : null,
            error: null
        }));
    } catch (error) {
        // 处理错误
        originalConsoleLog(JSON.stringify({
            logs: logs,
            result: null,
            error: error.toString()
        }));
    }
})(); 