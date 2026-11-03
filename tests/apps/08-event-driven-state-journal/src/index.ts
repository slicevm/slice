import { executeAndReplayJournal } from "./journal_replayer.ts";

const summary = executeAndReplayJournal(75);
console.log(JSON.stringify(summary));

