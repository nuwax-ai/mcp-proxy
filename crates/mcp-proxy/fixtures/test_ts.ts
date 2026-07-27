// Sample TypeScript file with handler function

// Log some debug information
console.log("Initializing TypeScript test...");

// Define a simple add function with type annotations
function add(a: number, b: number): number {
    console.log(`Adding ${a} + ${b}`);
    return a + b;
}

// Some processing with type annotations
const numbers: number[] = [1, 2, 3, 4, 5];
console.log("Processing numbers:", numbers);

// Interface for a person object
interface Person {
    name: string;
    age: number;
}

// Create a person object
const person: Person = {
    name: "TypeScript User",
    age: 30
};
console.log("Person object:", person);

/**
 * Handler function that will be called to get the result
 * 注意：这个函数必须存在，并且会被框架调用来获取结果
 */
function handler(): string {
    console.log("Handler function called");
    
    // Calculate sum
    const sum = numbers.reduce((acc, num) => add(acc, num), 0);
    console.log("Final calculation completed");
    
    return `Hello ${person.name}! The sum of [${numbers.join(", ")}] is ${sum}`;
} 