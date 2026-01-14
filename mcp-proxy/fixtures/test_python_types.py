import json
import os

# 打印一些调试信息
print("Python类型测试脚本开始执行...")

# 入口函数，接收参数并返回不同类型的值
def main(args):
    print("接收到的参数:", args)
    
    # 获取要测试的类型
    test_type = args.get("type", "string")
    
    # 根据参数返回不同类型的值
    if test_type == "string":
        print("返回字符串类型")
        return "这是一个字符串"
    
    elif test_type == "number":
        print("返回数字类型")
        return 12345
    
    elif test_type == "boolean":
        print("返回布尔类型")
        return True
    
    elif test_type == "null":
        print("返回None类型")
        return None
    
    elif test_type == "list":
        print("返回列表类型")
        return [1, 2, 3, "四", "五", True]
    
    elif test_type == "dict":
        print("返回字典类型")
        return {
            "name": "测试用户",
            "age": 30,
            "is_active": True,
            "tags": ["python", "rust", "json"]
        }
    
    else:
        print("未知类型，返回默认字符串")
        return "未知类型" 