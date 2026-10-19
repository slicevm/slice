import { runTelemetryPipeline } from "./histogram_wal.ts";

const report = runTelemetryPipeline(20);
console.log(JSON.stringify(report));

