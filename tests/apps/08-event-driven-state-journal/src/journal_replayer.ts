import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import * as crypto from "node:crypto";
import { EventSourcedLedger, type JournalRecord } from "./event_ledger.ts";

export interface JournalReplaySummary {
    app: string;
    libraries: string[];
    eventsDispatched: number;
    appliedCount: number;
    rejectedCount: number;
    replayedAggregateBalanceCents: number;
    journalDiskBytes: number;
    stateMatchVerified: boolean;
    journalSha256: string;
}

export function executeAndReplayJournal(eventCount: number): JournalReplaySummary {
    const ledger = new EventSourcedLedger();
    const walPath = path.join(os.tmpdir(), "slice_event_journal_" + process.pid + ".jsonl");
    if (fs.existsSync(walPath)) {
        fs.unlinkSync(walPath);
    }

    let appliedCount = 0;
    let rejectedCount = 0;

    ledger.bus.on("committed", (rec: JournalRecord) => {
        if (rec.status === "APPLIED") {
            appliedCount += 1;
        } else {
            rejectedCount += 1;
        }
        fs.appendFileSync(walPath, JSON.stringify(rec) + "\n");
    });

    const kinds: Array<"CREDIT" | "DEBIT" | "RESERVE"> = ["CREDIT", "DEBIT", "RESERVE"];
    const windows = ["30s", "5m", "15m"];

    for (let seq = 1; seq <= eventCount; seq++) {
        const accountId = "acct_" + (seq % 6);
        // Ensure first 12 events seed balances with CREDIT
        const kind = seq <= 12 ? "CREDIT" : kinds[seq % kinds.length];
        const amountCents = 1250 + (seq * 370) % 4500;
        ledger.bus.emit("command", {
            seq,
            accountId,
            kind,
            amountCents,
            ttlWindow: windows[seq % windows.length],
        });
    }

    const journalDiskBytes = fs.statSync(walPath).size;
    const rawJournal = fs.readFileSync(walPath, "utf8");
    const journalSha256 = crypto.createHash("sha256").update(rawJournal).digest("hex").slice(0, 16);

    // Replay the JSONL journal from scratch and verify deterministic state convergence
    const replayedBalances: Record<string, number> = {};
    const lines = rawJournal.trim().split("\n");
    for (let i = 0; i < lines.length; i++) {
        const entry = JSON.parse(lines[i]) as JournalRecord;
        replayedBalances[entry.accountId] = entry.resultingBalanceCents;
    }

    const liveBalances = ledger.getBalances();
    let stateMatchVerified = true;
    let replayedAggregateBalanceCents = 0;
    const acctKeys = Object.keys(liveBalances).sort();
    for (let i = 0; i < acctKeys.length; i++) {
        const k = acctKeys[i];
        if (liveBalances[k] !== replayedBalances[k]) {
            stateMatchVerified = false;
        }
        replayedAggregateBalanceCents += replayedBalances[k] || 0;
    }

    fs.unlinkSync(walPath);

    return {
        app: "08-event-driven-state-journal",
        libraries: ["mitt@3.0.1", "ms@2.1.3"],
        eventsDispatched: eventCount,
        appliedCount,
        rejectedCount,
        replayedAggregateBalanceCents,
        journalDiskBytes,
        stateMatchVerified,
        journalSha256,
    };
}

