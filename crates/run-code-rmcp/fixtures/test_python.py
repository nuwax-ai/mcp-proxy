#!/usr/bin/env python3
# Sample Python file with handler function

import time
time.time()
# Debug print to confirm script execution
print("DEBUGGING: Script is being executed!")

# Log some debug information
print("Initializing Python test...")

# Define a simple function
def multiply(a, b):
    print(f"Multiplying {a} * {b}")
    return a * b

# Some processing
numbers = [1, 2, 3, 4, 5]
print(f"Processing numbers: {numbers}")

# Handler function that will be called to get the result
def handler(args):
    print("Handler function called")
    print(f"Received args: {args}")

    # Calculate product (using the predefined numbers)
    product = 1
    for num in numbers:
        product = multiply(product, num)

    print("Final calculation completed")

    return f"The product of {numbers} is {product}"

# This part won't be executed when the code is run through the MCP runner
if __name__ == "__main__":
    print("Running directly, calling handler...")
    # Create sample args for direct execution
    sample_args = {"test": "direct_execution", "data": [1, 2, 3]}
    result = handler(sample_args)
    print(f"Result: {result}") 