import sys
import json
import os
import logging
from io import StringIO

# 保存原始的stdout
original_stdout = sys.stdout
logs = []

# 创建一个StringIO对象来捕获输出
class LogCapture:
    def __init__(self, show_logs=False):
        self.buffer = StringIO()
        self.show_logs = show_logs
    
    def write(self, text):
        if text.strip():  # 忽略空行
            logs.append(text.rstrip())
            if self.show_logs:
                original_stdout.write(text)
    
    def flush(self):
        self.buffer.flush()
        if self.show_logs:
            original_stdout.flush()

# 替换sys.stdout为我们的捕获器
sys.stdout = LogCapture(show_logs={{SHOW_LOGS}})

# 配置logging，将日志输出重定向到我们的捕获器
class LoggingHandler(logging.Handler):
    def emit(self, record):
        msg = self.format(record)
        print(f"[{record.levelname}] {msg}")

# 配置根日志记录器
root_logger = logging.getLogger()
root_logger.setLevel(logging.INFO)
# 移除所有现有的处理程序
for handler in root_logger.handlers[:]:
    root_logger.removeHandler(handler)
# 添加我们自定义的处理程序
handler = LoggingHandler()
handler.setFormatter(logging.Formatter('%(message)s'))
root_logger.addHandler(handler)

# 从环境变量获取输入参数
args = {}
has_input = False
try:
    # 优先从临时文件读取参数
    input_file = os.environ.get('INPUT_JSON_FILE')
    if input_file:
        with open(input_file, 'r', encoding='utf-8') as f:
            input_json = f.read()
        args = json.loads(input_json)
        has_input = True
        print(f"接收到的参数: {args}")
    else:
        # 兼容旧的环境变量方式
        input_json = os.environ.get('INPUT_JSON')
        if input_json:
            args = json.loads(input_json)
            has_input = True
            print(f"接收到的参数: {args}")

    # 确保参数同时可以通过args直接访问，也可以通过args.get("params")访问
    # 如果args中没有params键，但有其他键，则将整个args作为params的值
    if has_input and "params" not in args and len(args) > 0:
        args["params"] = args.copy()
    # 如果params存在但为None，则初始化为空字典
    elif has_input and args.get("params") is None:
        args["params"] = {}
except Exception as e:
    print(f"解析输入参数失败: {e}")

# 用户代码开始
{{USER_CODE}}

try:
    # 执行handler函数或main函数并获取结果
    result = None
    if 'handler' in globals() and callable(globals()['handler']):
        # 优先使用 handler 函数
        try:
            if has_input:
                result = handler(args)
            else:
                # handler 函数不接受参数，无参数调用
                result = handler()
            # 确保结果不为 None
            if result is None:
                print("警告: handler 函数返回了 None")
        except Exception as e:
            print(f"执行 handler 函数时出错: {e}")
    elif 'main' in globals() and callable(globals()['main']):
        # 如果没有 handler 函数，尝试使用 main 函数
        try:
            result = main(args)
        except Exception as e:
            print(f"执行 main 函数时出错: {e}")
    
    # 打印最终输出为JSON
    sys.stdout = original_stdout
    # 根据结果类型选择合适的处理方式
    result_json = None
    if result is not None:
        if isinstance(result, (dict, list)):
            # 如果是字典或列表，直接使用 json.dumps 序列化
            result_json = json.dumps(result)
        elif isinstance(result, (int, float, bool)) or result is None:
            # 如果是基本类型，直接传递给外层 JSON
            result_json = result
        else:
            # 其他类型（如字符串）转换为字符串
            result_json = str(result)
    
    print(json.dumps({
        'logs': logs,
        'result': result_json,
        'error': None
    }))
except Exception as e:
    # 处理错误
    import traceback
    error_msg = f"{str(e)}\n{traceback.format_exc()}"
    # 处理错误
    sys.stdout = original_stdout
    print(json.dumps({
        'logs': logs,
        'result': None,
        'error': error_msg
    })) 