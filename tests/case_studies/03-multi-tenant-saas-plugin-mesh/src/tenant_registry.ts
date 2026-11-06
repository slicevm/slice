import bytes from "bytes";
import ms from "ms";
import type { TenantPluginManifest } from "./types.ts";

export function buildTenantPluginCatalog(): TenantPluginManifest[] {
    const defaultMem = Number(bytes.parse("2MB"));
    const _ttlWindow = Number(ms("15m"));
    if (_ttlWindow <= 0) {
        throw new Error("Invalid TTL window");
    }

    return [
        {
            tenantId: "tenant-01-nordic-retail",
            pluginName: "vat-nordic-calc",
            version: "1.0.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 2500,
            loyaltyMultiplier: 2,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-02-tokyo-luxury",
            pluginName: "consumption-tax-jp",
            version: "1.2.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1000,
            loyaltyMultiplier: 3,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-03-berlin-b2b",
            pluginName: "eu-reverse-charge",
            version: "1.0.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1900,
            loyaltyMultiplier: 1,
            faultMode: "NONE",
            hotSwapTargetTaxRateBps: 1600, // Hot-swaps v1.0.0 -> v2.0.0 after event #5!
        },
        {
            tenantId: "tenant-04-austin-saas",
            pluginName: "tx-digital-tax",
            version: "1.1.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 825,
            loyaltyMultiplier: 2,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-05-noisy-cpu-miner",
            pluginName: "buggy-regex-Loop",
            version: "0.9.1",
            fuelLimit: 18_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 500,
            loyaltyMultiplier: 1,
            faultMode: "CPU_RUNAWAY_LOOP",
        },
        {
            tenantId: "tenant-06-london-fintech",
            pluginName: "uk-stamp-duty",
            version: "2.0.1",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 2000,
            loyaltyMultiplier: 4,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-07-malicious-fs-probe",
            pluginName: "rogue-key-stealer",
            version: "0.1.0",
            fuelLimit: 25_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1000,
            loyaltyMultiplier: 1,
            faultMode: "UNAUTHORIZED_FS_READ",
        },
        {
            tenantId: "tenant-08-toronto-market",
            pluginName: "hst-ontario-split",
            version: "1.4.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1300,
            loyaltyMultiplier: 2,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-09-ses-poisoner",
            pluginName: "prototype-hijack",
            version: "0.2.0",
            fuelLimit: 25_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1000,
            loyaltyMultiplier: 1,
            faultMode: "SES_PROTOTYPE_TAMPER",
        },
        {
            tenantId: "tenant-10-sydney-express",
            pluginName: "gst-au-calculator",
            version: "1.3.2",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 1000,
            loyaltyMultiplier: 2,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-11-zurich-wealth",
            pluginName: "swiss-mwst-tier",
            version: "3.1.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 810,
            loyaltyMultiplier: 5,
            faultMode: "NONE",
        },
        {
            tenantId: "tenant-12-singapore-hub",
            pluginName: "apac-gst-settlement",
            version: "2.2.0",
            fuelLimit: 35_000,
            memoryLimitBytes: defaultMem,
            taxRateBps: 900,
            loyaltyMultiplier: 3,
            faultMode: "NONE",
        },
    ];
}

