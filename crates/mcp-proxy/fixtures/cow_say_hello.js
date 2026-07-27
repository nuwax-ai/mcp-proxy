import * as o from 'https://deno.land/x/cowsay/mod.ts'

// 使用类型注解时应确保使用的是支持该特性的环境或编译器（如 TypeScript）
function add(input) {
    console.log("test Add", input.a, "+", input.b);

    let m = o.say({
        text: 'hello every one'
    })
    console.log(m)

    return input.a + input.b;
}


export default async function handler(input) {

    let a = add(input)
    console.log("计算结果=" + a)

    return {
        message: a,
    };
}