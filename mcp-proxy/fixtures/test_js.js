// Sample JavaScript file with handler function

// Log some debug information
console.log("Initializing JavaScript test...");

// Define a simple add function
function add(a, b) {
    console.log(`Adding ${a} + ${b}`);
    return a + b;
}

// Some processing
const numbers = [1, 2, 3, 4, 5];
console.log("Processing numbers:", numbers);

// Handler function that will be called to get the result
function handler() {
    console.log("Handler function called");
    
    // Calculate sum
    const sum = numbers.reduce((acc, num) => add(acc, num), 0);
    console.log("Final calculation completed");
    
    return `The sum of [${numbers.join(", ")}] is ${sum}`;
} 