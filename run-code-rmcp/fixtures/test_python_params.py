import json
import os

# 打印一些调试信息
print("Python脚本开始执行...")

# 定义一个简单的加法函数
def add(a, b):
    print("正在计算: " + str(a) + " + " + str(b))
    return a + b

# 处理一些数据
numbers = [1, 2, 3, 4, 5]
print("处理数字列表: " + str(numbers))

# 入口函数，接收参数
def main(args):
    print("接收到的参数: " + str(args))
    
    # 从参数中获取值
    a = args.get("a", 0)
    b = args.get("b", 0)
    
    # 计算结果
    result = add(a, b)
    print("计算完成: " + str(a) + " + " + str(b) + " = " + str(result))
    
    # 返回结果
    return {
        "sum": result,
        "numbers": numbers,
        "message": "成功计算 " + str(a) + " + " + str(b) + " 的结果"
    } 