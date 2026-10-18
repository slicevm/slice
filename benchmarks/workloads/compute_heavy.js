// Identical Compute-Intensive Workload (Fibonacci(40) + 10M Numeric Loop)
// Runs on: Slice (Nexus-JS), Node.js, Deno, and Bun

function fib(n) {
    let a = 0;
    let b = 1;
    for (let i = 0; i < n; i++) {
        let tmp = a + b;
        a = b;
        b = tmp;
    }
    return a;
}

function numericLoop(iterations) {
    let sum = 0;
    for (let i = 0; i < iterations; i++) {
        sum += i;
    }
    return sum;
}

const f = fib(40);
const loop_sum = numericLoop(10000000);
console.log(f);

