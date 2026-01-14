// TypeScript测试文件，演示参数传递

// 定义参数接口
interface InputParams {
    a: number;
    b: number;
    name?: string;
}

// 打印一些调试信息
console.log("TypeScript脚本开始执行...");

// 定义一个带类型的加法函数
function add(a: number, b: number): number {
    console.log(`正在计算: ${a} + ${b}`);
    return a + b;
}

// 处理一些数据
const numbers: number[] = [1, 2, 3, 4, 5];
console.log(`处理数字列表: ${numbers}`);

/**
 * 处理函数，接收参数并返回结果
 * 注意：这个函数必须存在，并且会被框架调用来获取结果
 */
function handler(input: InputParams): object {
    console.log(`接收到的参数: ${JSON.stringify(input)}`);
    
    // 从参数中获取值，提供默认值
    const a = input.a || 0;
    const b = input.b || 0;
    const name = input.name || "用户";
    
    // 计算结果
    const result = add(a, b);
    console.log(`计算完成: ${a} + ${b} = ${result}`);
    
    // 返回结果
    return {
        sum: result,
        numbers: numbers,
        greeting: `你好，${name}！`,
        message: `成功计算 ${a} + ${b} 的结果`
    };
} 