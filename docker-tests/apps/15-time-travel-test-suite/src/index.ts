import { runTimeTravelVerificationSuite } from "./time_travel_harness.ts";

const report = runTimeTravelVerificationSuite();
console.log(JSON.stringify(report));

