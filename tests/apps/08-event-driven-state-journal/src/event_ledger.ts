import mitt, { type Emitter } from "mitt";
import ms from "ms";

export interface LedgerCommandEvent {
    seq: number;
    accountId: string;
    kind: "CREDIT" | "DEBIT" | "RESERVE";
    amountCents: number;
    ttlWindow: string;
}

export interface JournalRecord {
    seq: number;
    accountId: string;
    kind: "CREDIT" | "DEBIT" | "RESERVE";
    amountCents: number;
    resultingBalanceCents: number;
    holdWindowMs: number;
    status: "APPLIED" | "REJECTED_INSUFFICIENT";
}

export type LedgerEventMap = {
    command: LedgerCommandEvent;
    committed: JournalRecord;
};

export class EventSourcedLedger {
    public readonly bus: Emitter<LedgerEventMap>;
    private readonly balances: Record<string, number>;
    private readonly journal: JournalRecord[];

    constructor() {
        this.bus = mitt<LedgerEventMap>();
        this.balances = {};
        this.journal = [];

        this.bus.on("command", (evt: LedgerCommandEvent) => {
            const current = this.balances[evt.accountId] || 0;
            const holdWindowMs = ms(evt.ttlWindow as unknown as number) as unknown as number;
            let nextBalance = current;
            let status: "APPLIED" | "REJECTED_INSUFFICIENT" = "APPLIED";

            if (evt.kind === "CREDIT") {
                nextBalance = current + evt.amountCents;
            } else {
                if (current >= evt.amountCents) {
                    nextBalance = current - evt.amountCents;
                } else {
                    status = "REJECTED_INSUFFICIENT";
                }
            }

            this.balances[evt.accountId] = nextBalance;
            const rec: JournalRecord = {
                seq: evt.seq,
                accountId: evt.accountId,
                kind: evt.kind,
                amountCents: evt.amountCents,
                resultingBalanceCents: nextBalance,
                holdWindowMs,
                status,
            };
            this.journal.push(rec);
            this.bus.emit("committed", rec);
        });
    }

    public getJournal(): JournalRecord[] {
        return this.journal;
    }

    public getBalances(): Record<string, number> {
        return this.balances;
    }
}

