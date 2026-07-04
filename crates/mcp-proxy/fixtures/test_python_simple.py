#!/usr/bin/env python3
# 简单的Python测试脚本，用于测试参数传递机制

def main(args: dict) -> dict:
    print(f"接收到的参数: {args}")
    
    # 尝试通过两种方式获取参数
    direct_params = args.get("input", "direct_default")
    nested_params = None
    
    params = args.get("params")
    if params:
        if isinstance(params, dict):
            nested_params = params.get("input", "nested_default")
        else:
            nested_params = str(params)
    
    # 返回结果，展示两种方式获取的参数
    return {
        "direct_access": direct_params,
        "nested_access": nested_params,
        "args_structure": args
    } 