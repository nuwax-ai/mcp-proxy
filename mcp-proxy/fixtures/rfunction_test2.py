#!/usr/bin/env python3
import pandas as pd

import logging
import json

# 创建一个简单的数据结构，不使用pandas
data = {
    'Name': ['Alice', 'Bob', 'Charlie'],
    'Age': [25, 30, 35]
}
df = pd.DataFrame(data)
# 记录数据信息
logging.info(f"Created data structure:\n{json.dumps(data, indent=2)}")

def main(args: dict) -> dict:
    logging.info(f"input args: {args}")

    params = args.get("params")
    
    # 处理params为None的情况
    if params is None:
        logging.warning("params is None, using default values")
        params = {"input": "default_value"}
    elif not isinstance(params, dict):
        logging.warning(f"params is not a dictionary: {type(params)}, using default values")
        params = {"input": str(params)}
    elif "input" not in params:
        logging.warning("input key not found in params, using default value")
        params["input"] = "default_value"
    
    # 构建输出对象
    ret = {
        "key0": params['input'],  # 使用input参数值
        "key1": ["hello", "world"],  # 输出一个数组
        "key2": {  # 输出一个Object
            "key21": "hi"
        },
    }
    return ret