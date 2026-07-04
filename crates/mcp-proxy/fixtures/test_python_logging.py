#!/usr/bin/env python3
# 测试logging模块的日志捕获功能

import logging
import json

# 配置日志
logging.basicConfig(level=logging.DEBUG)

# 使用不同级别的日志
def log_messages():
    logging.debug("这是一条DEBUG级别的日志")
    logging.info("这是一条INFO级别的日志")
    logging.warning("这是一条WARNING级别的日志")
    logging.error("这是一条ERROR级别的日志")
    logging.critical("这是一条CRITICAL级别的日志")
    
    # 使用格式化
    data = {"name": "测试", "value": 42}
    logging.info(f"格式化的JSON数据: {json.dumps(data, ensure_ascii=False)}")
    
    return "日志测试完成"

def handler(args):
    # 记录输入参数
    logging.info(f"收到参数: {args}")
    
    # 生成一些日志
    result = log_messages()
    
    # 返回结果
    return {
        "message": result,
        "log_count": 6,  # 总共记录了6条日志
        "args": args
    } 