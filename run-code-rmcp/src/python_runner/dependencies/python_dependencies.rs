use anyhow::Result;
use log::{debug, info};
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "python_imports.pest"] // 这里是你的pest语法文件
struct ImportParser;

// Python标准库模块列表，使用静态变量定义
static PYTHON_STDLIB_MODULES: &[&str] = &[
    "abc",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "atexit",
    "base64",
    "bdb",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "calendar",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "colorsys",
    "compileall",
    "concurrent",
    "configparser",
    "contextlib",
    "copy",
    "copyreg",
    "cProfile",
    "csv",
    "ctypes",
    "curses",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "doctest",
    "email",
    "encodings",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "idlelib",
    "imaplib",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "msvcrt",
    "multiprocessing",
    "netrc",
    "numbers",
    "operator",
    "optparse",
    "os",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posix",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtplib",
    "socket",
    "socketserver",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "tempfile",
    "termios",
    "test",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "turtledemo",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "winreg",
    "winsound",
    "wsgiref",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

pub fn parse_import(code: &str) -> Result<Vec<String>> {
    let modules = parse_python_imports(code)?;
    // 在非测试环境中，过滤标准库模块
    let mut imports = Vec::new();
    for module in modules {
        if !is_standard_library(&module) {
            imports.push(module);
        }
    }

    Ok(imports)
}

// 解析Python代码中的import语句
fn parse_python_imports(python_code: &str) -> Result<Vec<String>> {
    let input = if python_code.ends_with('\n') {
        python_code.to_string()
    } else {
        format!("{python_code}\n")
    };
    debug!("Processing input: {input:?}");

    let pairs = ImportParser::parse(Rule::file, &input)
        .map_err(|e| anyhow::anyhow!("Failed to parse input: {}", e))?;
    debug!("Initial parse successful");

    let mut imported_modules = Vec::new();

    for pair in pairs {
        debug!("Top level rule: {:?}", pair.as_rule());
        for record in pair.into_inner() {
            debug!(
                "Record rule: {:?}, text: {:?}",
                record.as_rule(),
                record.as_str()
            );
            // 遍历 record 的内部规则
            for inner in record.into_inner() {
                debug!(
                    "Inner rule: {:?}, text: {:?}",
                    inner.as_rule(),
                    inner.as_str()
                );
                match inner.as_rule() {
                    Rule::from_import_statement => {
                        for module in inner.into_inner() {
                            if let Rule::module_name = module.as_rule() {
                                imported_modules.push(module.as_str().to_string());
                            }
                        }
                    }
                    Rule::import_statement => {
                        // 遍历 import_statement 的内部规则找到 module_name
                        for module in inner.into_inner() {
                            if let Rule::module_name = module.as_rule() {
                                imported_modules.push(module.as_str().to_string());
                            }
                        }
                    }
                    Rule::importlib_statement => {
                        // 遍历 importlib_statement 的内部规则找到 module_name
                        for module in inner.into_inner() {
                            if let Rule::module_name = module.as_rule() {
                                imported_modules.push(module.as_str().to_string());
                            }
                        }
                    }
                    _ => {
                        info!("Unknown rule: {:?}", inner.as_rule());
                    }
                }
            }
        }
    }

    debug!("Final result: {imported_modules:?}");
    Ok(imported_modules)
}

// 判断是否为Python标准库模块
fn is_standard_library(module: &str) -> bool {
    // 检查模块名是否在标准库列表中
    PYTHON_STDLIB_MODULES.contains(&module)
}

#[cfg(test)]
mod tests {
    use super::parse_import;
    use anyhow::Result;
    use log::{LevelFilter, info};
    use std::sync::Once;

    // 使用 Once 确保 env_logger 只初始化一次
    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| {
            env_logger::builder()
                .filter_level(LevelFilter::Debug)
                .init();
        });
    }

    #[test]
    fn test_line() -> Result<()> {
        setup();
        let python_code = "
                import pandas as pd\n";
        info!("Testing with input: {python_code:?}");

        let imported_modules = parse_import(python_code)?;
        info!("Parsed modules: {imported_modules:?}");

        assert_eq!(imported_modules, vec!["pandas"]);
        Ok(())
    }

    #[test]
    fn test_line2() -> Result<()> {
        setup();
        let python_code = "
             numpy_alias = importlib.import_module('numpy')";
        info!("Testing with input: {python_code:?}");

        let imported_modules = parse_import(python_code)?;
        info!("Parsed modules: {imported_modules:?}");

        assert_eq!(imported_modules, vec!["numpy"]);
        Ok(())
    }

    #[test]
    fn test_parse_import_bs() -> Result<()> {
        setup();
        let python_code = r#"
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
    print(f"网页标题: {title}")
else:
    print(f"请求失败，状态码: {response.status_code}")

 "#;
        let imported_modules = parse_import(python_code)?;
        info!("Parsed modules: {imported_modules:?}");
        assert_eq!(imported_modules, vec!["requests", "bs4"]);

        Ok(())
    }

    #[test]
    fn test_parse_import() -> Result<()> {
        setup();
        let python_code = r#"
import pandas as pd
import logging

# 配置日志记录
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')

import importlib

# 使用 importlib 动态导入 numpy
numpy = importlib.import_module('numpy')
# 创建一个简单的 numpy 数组
arr = numpy.array([1, 2, 3, 4, 5])
# 记录数组信息
logging.info(f"Created numpy array: {arr}")

# 进行一些简单的操作
sum_arr = numpy.sum(arr)
logging.info(f"Sum of array: {sum_arr}")

# 创建一个简单的 DataFrame
data = {
    'Name': ['Alice', 'Bob', 'Charlie'],
    'Age': [25, 30, 35]
}
df = pd.DataFrame(data)

# 记录 DataFrame 信息
logging.info(f"Created DataFrame:\n{df}")

def main(args: dict) -> dict:
    logging.info(f"input args: {args}")

    params = args.get("params")
    # 构建输出对象
    ret = {
        "key0": params['input'], # 拼接两次入参 input 的值
        "key1": ["hello", "world"],  # 输出一个数组
        "key2": { # 输出一个Object
            "key21": "hi"
        },
    }
    return ret
"#;

        let imported_modules = parse_import(python_code)?;
        info!("Parsed modules: {imported_modules:?}");
        assert_eq!(imported_modules, vec!["pandas", "numpy"]);
        Ok(())
    }
}
