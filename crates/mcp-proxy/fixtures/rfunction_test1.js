// 使用类型注解时应确保使用的是支持该特性的环境或编译器（如 TypeScript）
function add(input) {
    console.log("test Add", input.a, "+", input.b);
    
    // 简单的字符串输出，不使用外部模块
    console.log("Hello from JavaScript!");
    
    return input.a + input.b;
}

function handler(input) {
    console.log("输入参数:", JSON.stringify(input));
    
    let a = add(input);
    console.log("计算结果=" + a);

    return {
        message: a,
    };
}