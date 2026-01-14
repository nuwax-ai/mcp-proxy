// 首先创建一个本地模块，通过相对路径引入
// 注意：这个示例假设有一个名为utils.js的本地模块

// 模拟本地utils.js模块的内容
const utils = {
    formatNumber(num) {
        return num.toLocaleString('zh-CN');
    },
    
    calculateTax(amount, rate = 0.1) {
        return amount * rate;
    },
    
    generateId() {
        return `id-${Date.now()}-${Math.floor(Math.random() * 1000)}`;
    },
    
    formatDate(date = new Date()) {
        return date.toLocaleDateString('zh-CN');
    }
};

// 使用本地模块的函数
function processOrder(order) {
    console.log(`处理订单: ${order.id || utils.generateId()}`);
    
    // 计算订单总额
    const total = order.items.reduce((sum, item) => sum + (item.price * item.quantity), 0);
    console.log(`订单总额: ${utils.formatNumber(total)}`);
    
    // 计算税费
    const taxRate = order.taxRate || 0.13;
    const tax = utils.calculateTax(total, taxRate);
    console.log(`税费(${taxRate * 100}%): ${utils.formatNumber(tax)}`);
    
    // 计算最终金额
    const finalAmount = total + tax;
    console.log(`最终金额: ${utils.formatNumber(finalAmount)}`);
    
    // 生成订单日期
    const orderDate = order.date ? new Date(order.date) : new Date();
    console.log(`订单日期: ${utils.formatDate(orderDate)}`);
    
    return {
        id: order.id || utils.generateId(),
        items: order.items,
        subtotal: total,
        tax: tax,
        total: finalAmount,
        date: utils.formatDate(orderDate)
    };
}

export default async function handler(input) {
    console.log("开始处理订单");
    
    // 默认订单或从输入获取
    const order = input.order || {
        items: [
            { name: "产品A", price: 100, quantity: 2 },
            { name: "产品B", price: 50, quantity: 1 },
            { name: "产品C", price: 200, quantity: 3 }
        ],
        taxRate: 0.13
    };
    
    // 处理订单
    const processedOrder = processOrder(order);
    
    return {
        message: '订单处理完成',
        order: processedOrder
    };
} 