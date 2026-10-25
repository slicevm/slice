import { executeDotenvAutoDlpAudit } from "./auto_dlp_auditor.ts";

const report = executeDotenvAutoDlpAudit();
console.log(JSON.stringify(report));

