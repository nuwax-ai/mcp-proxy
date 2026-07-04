#!/usr/bin/env python3
# Test Python script with large text parameters to verify "Argument list too long" fix

import sys
import json
import time

# Generate a large text parameter (>2MB) to test the temporary file parameter passing solution
def generate_large_text(size_in_mb):
    chunk = "This is a test chunk of text designed to generate large parameters for Python testing. " * 100
    iterations = (size_in_mb * 1024 * 1024) // len(chunk)

    result = ""
    for i in range(iterations):
        result += chunk + f"Iteration {i}\n"

    return result

# Handler function that processes large text parameters
def handler(args):
    print("Handler function called with large parameters")

    # Test that we received the large text parameter
    if not args or 'largeText' not in args:
        print("ERROR: No largeText parameter found")
        return {"success": False, "error": "Missing largeText parameter"}

    large_text = args['largeText']
    print(f"Received large text parameter, length: {len(large_text)}")
    print(f"Large text parameter size (MB): {len(large_text) / 1024 / 1024:.2f}")

    # Verify the text contains expected content
    expected_content = "This is a test chunk of text designed to generate large parameters for Python testing"
    if expected_content in large_text:
        print("SUCCESS: Large text parameter contains expected content")

        # Count occurrences of "Iteration" to verify data integrity
        iteration_count = large_text.count("Iteration")
        print(f"Found {iteration_count} iterations in the text")

        return {
            "success": True,
            "message": "Successfully processed large text parameter",
            "textSize": len(large_text),
            "sizeMB": round(len(large_text) / 1024 / 1024, 2),
            "iterationCount": iteration_count,
            "sampleText": large_text[:100] + "..."
        }
    else:
        print("ERROR: Large text parameter does not contain expected content")
        return {"success": False, "error": "Invalid content in largeText parameter"}

# This part won't be executed when the code is run through the MCP runner
if __name__ == "__main__":
    print("Running large parameter test directly...")

    # Generate test data (3MB to ensure we exceed the 2MB threshold)
    large_text = generate_large_text(3)
    print(f"Generated test data, size: {len(large_text) / 1024 / 1024:.2f} MB")

    # Create test arguments
    test_args = {
        "largeText": large_text,
        "additionalParam": "test value",
        "numberParam": 42
    }

    print("Calling handler with large parameters...")
    result = handler(test_args)
    print("Result:", json.dumps(result, indent=2))