// Identical Cold-Start + TypeScript Execution Workload
// Runs on: Slice (Nexus-JS), Bun, Deno, and Node.js (--experimental-strip-types)

interface EdgePayload {
    id: number;
    service: string;
}

const a: number = 40;
const b: number = 2;
const result: number = a + b;
console.log(result);

