//引入js组件,必须是http(s)形式引入,可以去网站: https://deno.land/x  搜索自己所需组件
//下面是一个引入示例
//import * as o from https://deno.land/x/cowsay/mod.ts
//网络请求,可以直接使用fetch ,具体自行查阅 fetch 文档
//打印日志使用console,比如:console.log

// 入口函数不可修改，否则无法执行，args 为配置的入参
export default async function main(args) {
    // 构建输出对象，出参中的key需与配置的出参保持一致
    return {
        key: 'value',
    };
}
