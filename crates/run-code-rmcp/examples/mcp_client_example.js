// 这是一个使用JavaScript调用MCP服务的示例
// 你可以使用任何支持MCP协议的客户端来调用我们的服务
// 以下是一个简单的示例，展示如何调用我们的服务

// 假设我们已经连接到了MCP服务

// 执行JavaScript代码示例
async function testRunJavaScript() {
  const code = `
    // 一些处理代码
    console.log("Processing JavaScript...");

    function handler(input) {
      // 输入包含通过参数传递的数据
      console.log("Received input:", input);
      
      // 返回最终结果
      return "Hello from JavaScript! Input: " + JSON.stringify(input);
    }
  `;

  const params = { name: "JavaScript User", data: [1, 2, 3] };
  
  // 在实际的MCP客户端中，你会使用类似以下的代码调用服务
  // const result = await client.run_javascript({ code, params });
  // console.log(result);
}

// 执行TypeScript代码示例
async function testRunTypeScript() {
  const code = `
    // 一些处理代码
    console.log("Processing TypeScript...");

    function handler(input: any): string {
      // 输入包含通过参数传递的数据
      console.log("Received input:", input);
      
      // 返回最终结果
      return \`Hello from TypeScript! Input: \${JSON.stringify(input)}\`;
    }
  `;

  const params = { name: "TypeScript User", count: 42 };
  
  // 在实际的MCP客户端中，你会使用类似以下的代码调用服务
  // const result = await client.run_typescript({ code, params });
  // console.log(result);
}

// 执行Python代码示例
async function testRunPython() {
  const code = `
# 一些处理代码
print("Processing Python...")

def handler(args):
    # args 包含通过参数传递的数据
    print(f"Received args: {args}")
    
    # 返回最终结果
    return f"Hello from Python! Args: {args}"
  `;

  const params = { name: "Python User", values: [10, 20, 30] };
  
  // 在实际的MCP客户端中，你会使用类似以下的代码调用服务
  // const result = await client.run_python({ code, params });
  // console.log(result);
}

// 在实际的MCP客户端中，你可以按顺序调用这些函数
// await testRunJavaScript();
// await testRunTypeScript();
// await testRunPython();

// 注意：这个文件只是一个示例，不能直接运行
// 你需要使用支持MCP协议的客户端（如Cursor）来调用我们的服务 