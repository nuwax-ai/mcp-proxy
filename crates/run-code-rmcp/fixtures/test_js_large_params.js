// Test JavaScript script with large text parameters to verify "Argument list too long" fix

// Generate a large text parameter (>2MB) to test the temporary file parameter passing solution
function generateLargeText(sizeInMB) {
    const chunk = "This is a test chunk of text designed to generate large parameters. ".repeat(100);
    const iterations = Math.ceil((sizeInMB * 1024 * 1024) / chunk.length);

    let result = "";
    for (let i = 0; i < iterations; i++) {
        result += chunk + `Iteration ${i}\n`;
    }

    return result;
}

// Handler function that processes large text parameters
function handler(args) {
    console.log("Handler function called with large parameters");

    // Test that we received the large text parameter
    if (!args || !args.largeText) {
        console.log("ERROR: No largeText parameter found");
        return { success: false, error: "Missing largeText parameter" };
    }

    const largeText = args.largeText;
    console.log("Received large text parameter, length:", largeText.length);
    console.log("Large text parameter size (MB):", (largeText.length / 1024 / 1024).toFixed(2));

    // Verify the text contains expected content
    if (largeText.includes("This is a test chunk of text designed to generate large parameters")) {
        console.log("SUCCESS: Large text parameter contains expected content");

        // Count occurrences of "Iteration" to verify data integrity
        const iterationCount = (largeText.match(/Iteration \d+/g) || []).length;
        console.log("Found", iterationCount, "iterations in the text");

        // Return some statistics about the processed data
        return {
            success: true,
            message: "Successfully processed large text parameter",
            textSize: largeText.length,
            sizeMB: (largeText.length / 1024 / 1024).toFixed(2),
            iterationCount: iterationCount,
            sampleText: largeText.substring(0, 100) + "..."
        };
    } else {
        console.log("ERROR: Large text parameter does not contain expected content");
        return { success: false, error: "Invalid content in largeText parameter" };
    }
}

// This part won't be executed when the code is run through the MCP runner
if (typeof require !== 'undefined' && require.main === module) {
    console.log("Running large parameter test directly...");

    // Generate test data (3MB to ensure we exceed the 2MB threshold)
    const largeText = generateLargeText(3);
    console.log("Generated test data, size:", (largeText.length / 1024 / 1024).toFixed(2), "MB");

    // Create test arguments
    const testArgs = {
        largeText: largeText,
        additionalParam: "test value",
        numberParam: 42
    };

    console.log("Calling handler with large parameters...");
    const result = handler(testArgs);
    console.log("Result:", JSON.stringify(result, null, 2));
}

if (typeof module !== 'undefined' && typeof module.exports !== 'undefined') {
    module.exports = { handler, generateLargeText };
}