import _ from 'npm:lodash';

// 使用lodash处理数据的函数
function processData(input) {
    console.log("输入数据:", input);
    
    // 使用lodash的chunk方法将数组分块
    if (Array.isArray(input.data)) {
        const chunks = _.chunk(input.data, input.chunkSize || 2);
        console.log("数据分块结果:", chunks);
        
        // 使用lodash的map方法处理每个块
        const processedChunks = _.map(chunks, (chunk) => {
            return {
                items: chunk,
                sum: _.sum(chunk),
                average: _.mean(chunk)
            };
        });
        
        return processedChunks;
    }
    
    return { error: "输入数据不是数组" };
}

export default async function handler(input) {
    console.log("开始处理数据");
    
    // 从输入获取数据，或使用默认数据
    const data = input.data || [1, 2, 3, 4, 5, 6, 7, 8, 9];
    const chunkSize = input.chunkSize || 3;
    
    // 处理数据
    const result = processData({ data, chunkSize });
    
    return {
        message: '数据处理完成',
        originalData: data,
        processedData: result,
        timestamp: new Date().toISOString()
    };
} 