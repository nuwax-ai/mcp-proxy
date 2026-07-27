import { assertEquals } from "jsr:@std/assert";
import { delay } from "jsr:@std/async";
import { bold, red, green } from "jsr:@std/fmt/colors";

// 使用JSR标准库中的工具函数
async function runTests(input) {
    console.log(bold("开始运行测试..."));
    
    // 使用delay函数模拟异步操作
    console.log("等待1秒...");
    await delay(1000);
    
    // 测试结果数组
    const results = [];
    
    // 测试1: 加法运算
    try {
        const a = input.a || 5;
        const b = input.b || 3;
        const expected = input.expected || 8;
        
        console.log(`测试加法: ${a} + ${b} = ${expected}`);
        assertEquals(a + b, expected);
        
        console.log(green("✓ 加法测试通过"));
        results.push({ name: "加法测试", success: true });
    } catch (error) {
        console.log(red(`✗ 加法测试失败: ${error.message}`));
        results.push({ name: "加法测试", success: false, error: error.message });
    }
    
    // 测试2: 字符串操作
    try {
        const str1 = input.str1 || "hello";
        const str2 = input.str2 || "world";
        const expectedStr = input.expectedStr || "hello world";
        
        console.log(`测试字符串连接: "${str1}" + " " + "${str2}" = "${expectedStr}"`);
        assertEquals(`${str1} ${str2}`, expectedStr);
        
        console.log(green("✓ 字符串测试通过"));
        results.push({ name: "字符串测试", success: true });
    } catch (error) {
        console.log(red(`✗ 字符串测试失败: ${error.message}`));
        results.push({ name: "字符串测试", success: false, error: error.message });
    }
    
    // 等待再次展示结果
    console.log("处理结果中...");
    await delay(500);
    
    return results;
}

export default async function handler(input) {
    console.log(bold("JSR依赖示例"));
    
    // 运行测试
    const testResults = await runTests(input);
    
    // 统计结果
    const passed = testResults.filter(r => r.success).length;
    const failed = testResults.filter(r => !r.success).length;
    
    // 使用彩色输出显示结果
    console.log(bold("\n测试结果摘要:"));
    console.log(green(`通过: ${passed}`));
    console.log(failed > 0 ? red(`失败: ${failed}`) : green(`失败: ${failed}`));
    
    return {
        message: '测试完成',
        results: testResults,
        summary: {
            total: testResults.length,
            passed,
            failed
        }
    };
} 