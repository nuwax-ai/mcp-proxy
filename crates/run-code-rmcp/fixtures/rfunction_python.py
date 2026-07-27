import requests
from bs4 import BeautifulSoup

# 发送请求到百度
url = 'https://www.baidu.com'
response = requests.get(url)

# 检查请求是否成功
if response.status_code == 200:
    # 解析网页内容
    soup = BeautifulSoup(response.text, 'html.parser')

    # 提取你想要的信息，例如标题
    title = soup.title.string
else:
    print(f"请求失败，状态码:")


# 入口函数不可修改，否则无法执行，args 为配置的入参
def main(args: dict) -> dict:
    params = args.get("params")
    # 构建输出对象，出参中的key需与配置的出参保持一致
    ret = {
        "key": "value"
    }
    return ret
