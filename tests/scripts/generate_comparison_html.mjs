#!/usr/bin/env node
/**
 * Generates a single self-contained HTML comparison dashboard (`tests/reports/comparison.html`)
 * across all historical `tests/reports/bench_v*.json` snapshots (always overwritten).
 *
 * Usage:
 *   node tests/scripts/generate_comparison_html.mjs
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.resolve(__dirname, "..", "..");
const REPORTS_DIR = path.join(ROOT_DIR, "tests", "reports");
const OUT_HTML = path.join(REPORTS_DIR, "comparison.html");

function parseSemver(v) {
  const m = String(v || "0.0.0").match(/(\d+)\.(\d+)\.(\d+)/);
  if (!m) return [0, 0, 0];
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

function compareSemver(a, b) {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] - pb[i];
  }
  return 0;
}

function loadReports() {
  if (!fs.existsSync(REPORTS_DIR)) {
    throw new Error(`Reports directory not found: ${REPORTS_DIR}`);
  }
  const files = fs
    .readdirSync(REPORTS_DIR)
    .filter((f) => /^bench_v.+\.json$/.test(f));

  const parsed = files.map((file) => {
    const raw = JSON.parse(fs.readFileSync(path.join(REPORTS_DIR, file), "utf8"));
    const workloads = raw.workloads || [];
    const rss = raw.rss || [];
    const coldTotMs = workloads.reduce((s, w) => s + (Number(w.sliceCasAvgMs) || 0), 0);
    const warmTotMs = workloads.reduce((s, w) => s + (Number(w.sliceWarmMs) || 0), 0);
    const binTotMs = workloads.reduce((s, w) => s + (Number(w.sliceBinAvgMs) || 0), 0);
    const bunTotMs = workloads.reduce((s, w) => s + (Number(w.bunAvgMs) || 0), 0);
    const denoTotMs = workloads.reduce((s, w) => s + (Number(w.denoAvgMs) || 0), 0);
    const nodeTotMs = workloads.reduce((s, w) => s + (Number(w.nodeAvgMs) || 0), 0);

    const sliceRssMb = rss.map((r) => (Number(r.sliceKb) || 0) / 1024).filter((x) => x > 0);
    const rssMinMb = sliceRssMb.length ? Math.min(...sliceRssMb) : 0;
    const rssMaxMb = sliceRssMb.length ? Math.max(...sliceRssMb) : 0;
    const rssAvgMb = sliceRssMb.length
      ? sliceRssMb.reduce((a, b) => a + b, 0) / sliceRssMb.length
      : 0;

    return {
      file,
      version: raw.metadata?.version || "0.0.0",
      commit: raw.metadata?.commit || "unknown",
      timestampUtc: raw.metadata?.timestampUtc || "",
      runtimes: raw.metadata?.runtimes || {},
      workloadCount: workloads.length,
      coldTotMs: Number(coldTotMs.toFixed(1)),
      warmTotMs: Number(warmTotMs.toFixed(1)),
      binTotMs: Number(binTotMs.toFixed(1)),
      bunTotMs: Number(bunTotMs.toFixed(1)),
      denoTotMs: Number(denoTotMs.toFixed(1)),
      nodeTotMs: Number(nodeTotMs.toFixed(1)),
      rssMinMb: Number(rssMinMb.toFixed(2)),
      rssAvgMb: Number(rssAvgMb.toFixed(2)),
      rssMaxMb: Number(rssMaxMb.toFixed(2)),
      tier0Rps: Number(raw.httpKeepAlive?.sliceTier0?.rps || 0),
      tier1Rps: Number(raw.httpKeepAlive?.sliceTier1?.rps || 0),
      bunRps: Number(raw.httpKeepAlive?.bun?.rps || 0),
      denoRps: Number(raw.httpKeepAlive?.deno?.rps || 0),
      nodeRps: Number(raw.httpKeepAlive?.node?.rps || 0),
      coldP50Us: Number(raw.microvm?.linux?.coldP50Us || 0),
      coldP99Us: Number(raw.microvm?.linux?.coldP99Us || 0),
      swarm1000Ms: Number(raw.microvm?.linux?.swarm1000Ms || 0),
      swarm1000RssMb: Number(raw.microvm?.linux?.swarm1000RssMb || 0),
      cas500ColdMs: Number(raw.microvm?.linux?.cas500ColdMs || 0),
      cas500MemUs: Number(raw.microvm?.linux?.cas500MemUs || 0),
      kv1001Us: Number(raw.primitives?.kv1001Us || 0),
      tot3SandboxesUs: Number(raw.primitives?.tot3SandboxesUs || 0),
      workloads: workloads.map((w) => ({
        id: w.id,
        category: w.category || "other",
        coldMs: Number(w.sliceCasAvgMs || 0),
        warmMs: Number(w.sliceWarmMs || 0),
        bunMs: Number(w.bunAvgMs || 0),
        denoMs: Number(w.denoAvgMs || 0),
        nodeMs: Number(w.nodeAvgMs || 0),
      })),
      rss: rss.map((r) => ({
        id: r.id,
        sliceMb: Number(((Number(r.sliceKb) || 0) / 1024).toFixed(2)),
        bunMb: Number(((Number(r.bunKb) || 0) / 1024).toFixed(2)),
        denoMb: Number(((Number(r.denoKb) || 0) / 1024).toFixed(2)),
        nodeMb: Number(((Number(r.nodeKb) || 0) / 1024).toFixed(2)),
      })),
    };
  });

  parsed.sort((a, b) => {
    const c = compareSemver(a.version, b.version);
    if (c !== 0) return c;
    return a.timestampUtc.localeCompare(b.timestampUtc);
  });

  return parsed;
}

function buildHtml(reports) {
  const generatedUtc = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const jsonBlob = JSON.stringify(reports).replace(/<\//g, "<\\/");

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SliceVM Runtime - Cross-Release Benchmark Comparison</title>
<style>
  :root {
    --bg: #0b0f17;
    --panel: #131a26;
    --panel-alt: #182132;
    --border: #243147;
    --text: #e6edf7;
    --muted: #8fa3bf;
    --accent: #38bdf8;
    --accent-2: #818cf8;
    --good: #34d399;
    --warn: #fbbf24;
    --bad: #f87171;
    --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    --sans: system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    background: var(--bg);
    color: var(--text);
    font-family: var(--sans);
    line-height: 1.5;
  }
  .wrap { max-width: 1360px; margin: 0 auto; padding: 28px 20px 64px; }
  header {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: flex-end;
    gap: 16px;
    padding-bottom: 20px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 24px;
  }
  h1 { margin: 0; font-size: 26px; letter-spacing: 0.02em; }
  .subtitle { color: var(--muted); font-size: 14px; margin-top: 4px; }
  .meta-pill {
    font-family: var(--mono);
    font-size: 12px;
    background: var(--panel);
    border: 1px solid var(--border);
    padding: 6px 12px;
    border-radius: 6px;
    color: var(--muted);
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
    gap: 14px;
    margin-bottom: 26px;
  }
  .kpi {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 16px;
  }
  .kpi-label { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .kpi-val { font-family: var(--mono); font-size: 24px; font-weight: 700; margin: 6px 0 4px; }
  .kpi-sub { font-family: var(--mono); font-size: 12px; color: var(--muted); display: flex; flex-direction: column; gap: 2px; }
  .badge { display: inline-block; padding: 1px 6px; border-radius: 4px; font-family: var(--mono); font-size: 11px; font-weight: 600; }
  .good { color: var(--good); background: rgba(52, 211, 153, 0.12); }
  .warn { color: var(--warn); background: rgba(251, 191, 36, 0.14); }
  .bad { color: var(--bad); background: rgba(248, 113, 113, 0.14); }
  .neutral { color: var(--muted); background: rgba(143, 163, 191, 0.12); }
  .charts-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(560px, 1fr));
    gap: 18px;
    margin-bottom: 28px;
  }
  .card {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px;
    margin-bottom: 22px;
  }
  .charts-grid .card { margin-bottom: 0; }
  .card h2 { margin: 0 0 4px; font-size: 16px; }
  .card .desc { margin: 0 0 12px; font-size: 13px; color: var(--muted); }
  .legend { display: flex; flex-wrap: wrap; gap: 14px; font-size: 12px; color: var(--muted); margin-bottom: 8px; }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
  svg.chart { width: 100%; height: auto; display: block; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; font-family: var(--mono); }
  th, td { padding: 8px 10px; border-bottom: 1px solid var(--border); text-align: right; }
  th:first-child, td:first-child { text-align: left; }
  th { color: var(--muted); font-weight: 600; background: var(--panel-alt); position: sticky; top: 0; }
  tr:hover td { background: rgba(56, 189, 248, 0.04); }
  .controls { display: flex; flex-wrap: wrap; gap: 10px; margin-bottom: 12px; align-items: center; }
  input, select {
    background: var(--panel-alt);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    font-family: var(--sans);
  }
  .table-scroll { overflow-x: auto; max-height: 520px; border: 1px solid var(--border); border-radius: 8px; }
  .two-col { display: grid; grid-template-columns: repeat(auto-fit, minmax(460px, 1fr)); gap: 18px; margin-bottom: 22px; }
  .two-col .card { margin-bottom: 0; }
</style>
</head>
<body>
<div class="wrap">
  <header>
    <div>
      <h1>SliceVM Runtime -- Benchmark History &amp; Regression Dashboard</h1>
      <div class="subtitle" id="headerSub">Comparing historical Linux x86_64 Docker + MicroVM benchmark snapshots</div>
    </div>
    <div class="meta-pill">Generated: ${generatedUtc}</div>
  </header>

  <section class="kpis" id="kpiGrid"></section>

  <section class="charts-grid">
    <div class="card">
      <h2>33-Workload Total Latency (ms, lower is better)</h2>
      <p class="desc">Cold .cas one-shot total, --warm pooled total, and standalone compiled binary total across releases.</p>
      <div class="legend" id="legLatency"></div>
      <div id="chartLatency"></div>
    </div>
    <div class="card">
      <h2>HTTP/1.1 Keep-Alive Throughput (req/s, higher is better)</h2>
      <p class="desc">16,000 requests on slice serve (Tier-0 CAS &amp; Tier-1 WinterTC) vs Bun, Deno, and Node.js.</p>
      <div class="legend" id="legHttp"></div>
      <div id="chartHttp"></div>
    </div>
    <div class="card">
      <h2>Memory Footprint -- Peak RSS (MB, lower is better)</h2>
      <p class="desc">Per-workload Peak RSS (Min / Avg / Max) and 1,000-Tenant MicroVM Swarm RSS across releases.</p>
      <div class="legend" id="legRss"></div>
      <div id="chartRss"></div>
    </div>
    <div class="card">
      <h2>MicroVM Cold-Start &amp; 1,000-Tenant Swarm Latency</h2>
      <p class="desc">Single isolate P50/P99 cold-start (us) and 1,000-tenant boot+exec swarm (ms).</p>
      <div class="legend" id="legMicrovm"></div>
      <div id="chartMicrovm"></div>
    </div>
  </section>

  <section class="two-col">
    <div class="card">
      <h2>Latency Regression &amp; Speedup Watchdog (Latest vs Previous)</h2>
      <p class="desc" id="watchdogLatDesc"></p>
      <div class="table-scroll"><table id="tblWatchdogLat"></table></div>
    </div>
    <div class="card">
      <h2>Memory (RSS MB) Delta Watchdog (Latest vs Previous)</h2>
      <p class="desc" id="watchdogRssDesc"></p>
      <div class="table-scroll"><table id="tblWatchdogRss"></table></div>
    </div>
  </section>

  <section class="card">
    <h2>Release-by-Release Summary Matrix</h2>
    <p class="desc">All canonical benchmark snapshots in tests/reports/ sorted by SemVer.</p>
    <div class="table-scroll"><table id="tblSummary"></table></div>
  </section>

  <section class="card">
    <h2>Per-Workload Historical Explorer</h2>
    <div class="controls">
      <input id="searchBox" type="search" placeholder="Filter workload id (e.g. hono, sveltekit, wasm, tot)...">
      <select id="metricMode">
        <option value="coldMs">Cold .cas Avg Latency (ms)</option>
        <option value="warmMs">--warm Pooled Latency (ms)</option>
      </select>
      <select id="catFilter"><option value="all">All Categories</option></select>
    </div>
    <div class="table-scroll"><table id="tblWorkloads"></table></div>
  </section>
</div>

<script>
const REPORTS = ${jsonBlob};

function fmtNum(n, d = 1) {
  if (n === null || n === undefined || Number.isNaN(n)) return "-";
  return Number(n).toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
}

function deltaBadge(curr, prev, lowerIsBetter, unit = "") {
  if (prev === undefined || prev === null || prev === 0) return '<span class="badge neutral">baseline</span>';
  const diff = curr - prev;
  const pct = (diff / prev) * 100;
  const sign = diff > 0 ? "+" : "";
  const isGood = lowerIsBetter ? diff <= 0 : diff >= 0;
  const isFlat = Math.abs(pct) < 1.5;
  const cls = isFlat ? "neutral" : (isGood ? "good" : (Math.abs(pct) < 5 ? "warn" : "bad"));
  const diffStr = Math.abs(diff) >= 100 ? Math.round(diff).toLocaleString("en-US") : diff.toFixed(2);
  return '<span class="badge ' + cls + '">' + sign + diffStr + unit + ' (' + sign + pct.toFixed(1) + '%)</span>';
}

function renderLegend(elId, series) {
  document.getElementById(elId).innerHTML = series
    .map((s) => '<span><i class="dot" style="background:' + s.color + '"></i>' + s.label + '</span>')
    .join("");
}

function renderLineChart(elId, labels, series, unit) {
  const W = 580, H = 240, PL = 58, PR = 20, PT = 16, PB = 34;
  const plotW = W - PL - PR, plotH = H - PT - PB;
  const allVals = series.flatMap((s) => s.values).filter((v) => typeof v === "number" && !Number.isNaN(v));
  const minV = 0;
  const maxV = Math.max(...allVals, 1) * 1.12;
  const xStep = labels.length > 1 ? plotW / (labels.length - 1) : 0;
  const xAt = (i) => (labels.length > 1 ? PL + i * xStep : PL + plotW / 2);
  const yAt = (v) => PT + plotH - ((v - minV) / (maxV - minV)) * plotH;

  let svg = '<svg class="chart" viewBox="0 0 ' + W + ' ' + H + '">';
  for (let g = 0; g <= 4; g++) {
    const val = minV + ((maxV - minV) * g) / 4;
    const y = yAt(val);
    svg += '<line x1="' + PL + '" y1="' + y.toFixed(1) + '" x2="' + (W - PR) + '" y2="' + y.toFixed(1) + '" stroke="#243147" stroke-dasharray="3 3"/>';
    const lbl = maxV >= 1000 ? Math.round(val).toLocaleString("en-US") : val.toFixed(maxV < 10 ? 2 : 0);
    svg += '<text x="' + (PL - 8) + '" y="' + (y + 4).toFixed(1) + '" fill="#8fa3bf" font-size="11" font-family="monospace" text-anchor="end">' + lbl + '</text>';
  }
  labels.forEach((lbl, i) => {
    svg += '<text x="' + xAt(i).toFixed(1) + '" y="' + (H - 10) + '" fill="#8fa3bf" font-size="11" font-family="monospace" text-anchor="middle">' + lbl + '</text>';
  });
  for (const s of series) {
    const pts = s.values.map((v, i) => xAt(i).toFixed(1) + "," + yAt(v).toFixed(1)).join(" ");
    svg += '<polyline fill="none" stroke="' + s.color + '" stroke-width="2.5" stroke-dasharray="' + (s.dash || "") + '" points="' + pts + '"/>';
    s.values.forEach((v, i) => {
      const cx = xAt(i).toFixed(1), cy = yAt(v).toFixed(1);
      svg += '<circle cx="' + cx + '" cy="' + cy + '" r="4" fill="' + s.color + '"><title>' + s.label + ' (' + labels[i] + '): ' + fmtNum(v, maxV < 20 ? 2 : 1) + unit + '</title></circle>';
    });
  }
  svg += "</svg>";
  document.getElementById(elId).innerHTML = svg;
}

(function init() {
  if (!REPORTS.length) return;
  const first = REPORTS[0];
  const latest = REPORTS[REPORTS.length - 1];
  const prev = REPORTS.length >= 2 ? REPORTS[REPORTS.length - 2] : null;
  const labels = REPORTS.map((r) => "v" + r.version);

  document.getElementById("headerSub").textContent =
    "Tracking " + REPORTS.length + " releases (v" + first.version + " -> v" + latest.version + ") across " + latest.workloadCount + " real-world workloads";

  const kpis = [
    { label: "Cold .cas Total (33 Apps)", val: fmtNum(latest.coldTotMs, 1) + " ms", prev: prev?.coldTotMs, first: first.coldTotMs, lower: true, unit: " ms" },
    { label: "--warm Pooled Total", val: fmtNum(latest.warmTotMs, 1) + " ms", prev: prev?.warmTotMs, first: first.warmTotMs, lower: true, unit: " ms" },
    { label: "HTTP Keep-Alive Tier-0", val: fmtNum(latest.tier0Rps, 0) + " req/s", prev: prev?.tier0Rps, first: first.tier0Rps, lower: false, unit: "" },
    { label: "HTTP Keep-Alive Tier-1", val: fmtNum(latest.tier1Rps, 0) + " req/s", prev: prev?.tier1Rps, first: first.tier1Rps, lower: false, unit: "" },
    { label: "Isolate Cold-Start P50", val: fmtNum(latest.coldP50Us, 2) + " us", prev: prev?.coldP50Us, first: first.coldP50Us, lower: true, unit: " us" },
    { label: "1,000-Tenant Swarm", val: fmtNum(latest.swarm1000Ms, 2) + " ms", prev: prev?.swarm1000Ms, first: first.swarm1000Ms, lower: true, unit: " ms" },
    { label: "Avg Peak RSS (33 Apps)", val: fmtNum(latest.rssAvgMb, 2) + " MB", prev: prev?.rssAvgMb, first: first.rssAvgMb, lower: true, unit: " MB" },
    { label: "Max Peak RSS (33 Apps)", val: fmtNum(latest.rssMaxMb, 2) + " MB", prev: prev?.rssMaxMb, first: first.rssMaxMb, lower: true, unit: " MB" }
  ];

  document.getElementById("kpiGrid").innerHTML = kpis.map((k) =>
    '<div class="kpi"><div class="kpi-label">' + k.label + '</div>' +
    '<div class="kpi-val">' + k.val + '</div>' +
    '<div class="kpi-sub">' +
      '<div>vs prev' + (prev ? " (v" + prev.version + ")" : "") + ": " + deltaBadge(parseFloat(k.val.replace(/,/g, "")), k.prev, k.lower, k.unit) + '</div>' +
      '<div>vs v' + first.version + ": " + deltaBadge(parseFloat(k.val.replace(/,/g, "")), k.first, k.lower, k.unit) + '</div>' +
    '</div></div>'
  ).join("");

  const latSeries = [
    { label: "Cold .cas (ms)", color: "#38bdf8", values: REPORTS.map((r) => r.coldTotMs) },
    { label: "--warm Pooled (ms)", color: "#34d399", values: REPORTS.map((r) => r.warmTotMs) },
    { label: "Standalone Bin (ms)", color: "#818cf8", dash: "5 4", values: REPORTS.map((r) => r.binTotMs) }
  ];
  renderLegend("legLatency", latSeries);
  renderLineChart("chartLatency", labels, latSeries, " ms");

  const httpSeries = [
    { label: "Slice Tier-0 CAS", color: "#38bdf8", values: REPORTS.map((r) => r.tier0Rps) },
    { label: "Slice Tier-1 WinterTC", color: "#34d399", values: REPORTS.map((r) => r.tier1Rps) },
    { label: "Bun", color: "#fbbf24", dash: "4 4", values: REPORTS.map((r) => r.bunRps) },
    { label: "Deno", color: "#818cf8", dash: "4 4", values: REPORTS.map((r) => r.denoRps) },
    { label: "Node.js", color: "#f87171", dash: "2 4", values: REPORTS.map((r) => r.nodeRps) }
  ];
  renderLegend("legHttp", httpSeries);
  renderLineChart("chartHttp", labels, httpSeries, " req/s");

  const rssSeries = [
    { label: "Max Peak RSS (MB)", color: "#f87171", values: REPORTS.map((r) => r.rssMaxMb) },
    { label: "Avg Peak RSS (MB)", color: "#38bdf8", values: REPORTS.map((r) => r.rssAvgMb) },
    { label: "Min Peak RSS (MB)", color: "#34d399", values: REPORTS.map((r) => r.rssMinMb) },
    { label: "1k-Tenant Swarm RSS (MB)", color: "#fbbf24", dash: "5 4", values: REPORTS.map((r) => r.swarm1000RssMb) }
  ];
  renderLegend("legRss", rssSeries);
  renderLineChart("chartRss", labels, rssSeries, " MB");

  const microSeries = [
    { label: "Cold-Start P50 (us)", color: "#34d399", values: REPORTS.map((r) => r.coldP50Us) },
    { label: "Cold-Start P99 (us)", color: "#38bdf8", values: REPORTS.map((r) => r.coldP99Us) },
    { label: "1k-Tenant Swarm (ms)", color: "#fbbf24", values: REPORTS.map((r) => r.swarm1000Ms) }
  ];
  renderLegend("legMicrovm", microSeries);
  renderLineChart("chartMicrovm", labels, microSeries, "");

  // Watchdog tables
  if (prev) {
    document.getElementById("watchdogLatDesc").textContent =
      "Comparing v" + prev.version + " -> v" + latest.version + " sorted by largest Cold .cas delta.";
    const pMap = Object.fromEntries(prev.workloads.map((w) => [w.id, w]));
    const latRows = latest.workloads
      .map((w) => {
        const pw = pMap[w.id];
        return {
          id: w.id,
          prevCold: pw ? pw.coldMs : null,
          currCold: w.coldMs,
          dCold: pw ? w.coldMs - pw.coldMs : 0,
          prevWarm: pw ? pw.warmMs : null,
          currWarm: w.warmMs,
          dWarm: pw ? w.warmMs - pw.warmMs : 0
        };
      })
      .sort((a, b) => b.dCold - a.dCold);
    document.getElementById("tblWatchdogLat").innerHTML =
      "<thead><tr><th>Workload</th><th>Cold Prev</th><th>Cold Now</th><th>Cold Delta</th><th>Warm Delta</th></tr></thead><tbody>" +
      latRows.map((r) =>
        "<tr><td>" + r.id + "</td><td>" + fmtNum(r.prevCold, 2) + "</td><td>" + fmtNum(r.currCold, 2) + "</td><td>" +
        deltaBadge(r.currCold, r.prevCold, true, " ms") + "</td><td>" + deltaBadge(r.currWarm, r.prevWarm, true, " ms") + "</td></tr>"
      ).join("") + "</tbody>";

    document.getElementById("watchdogRssDesc").textContent =
      "Comparing v" + prev.version + " -> v" + latest.version + " sorted by largest Peak RSS (MB) increase.";
    const prMap = Object.fromEntries(prev.rss.map((r) => [r.id, r]));
    const rssRows = latest.rss
      .map((r) => {
        const pr = prMap[r.id];
        return { id: r.id, prevMb: pr ? pr.sliceMb : null, currMb: r.sliceMb, dMb: pr ? r.sliceMb - pr.sliceMb : 0 };
      })
      .sort((a, b) => b.dMb - a.dMb);
    document.getElementById("tblWatchdogRss").innerHTML =
      "<thead><tr><th>Workload</th><th>RSS Prev (MB)</th><th>RSS Now (MB)</th><th>Delta</th></tr></thead><tbody>" +
      rssRows.map((r) =>
        "<tr><td>" + r.id + "</td><td>" + fmtNum(r.prevMb, 2) + "</td><td>" + fmtNum(r.currMb, 2) + "</td><td>" +
        deltaBadge(r.currMb, r.prevMb, true, " MB") + "</td></tr>"
      ).join("") + "</tbody>";
  }

  // Summary matrix
  document.getElementById("tblSummary").innerHTML =
    "<thead><tr><th>Version</th><th>Commit</th><th>Cold .cas</th><th>--warm</th><th>Tier-0 RPS</th><th>Tier-1 RPS</th><th>P50 (us)</th><th>Swarm (ms)</th><th>Swarm RSS</th><th>Avg RSS</th><th>Max RSS</th><th>500-Mod</th></tr></thead><tbody>" +
    REPORTS.map((r) =>
      "<tr><td>v" + r.version + "</td><td>" + r.commit + "</td><td>" + fmtNum(r.coldTotMs, 1) + " ms</td><td>" + fmtNum(r.warmTotMs, 1) + " ms</td><td>" +
      fmtNum(r.tier0Rps, 0) + "</td><td>" + fmtNum(r.tier1Rps, 0) + "</td><td>" + fmtNum(r.coldP50Us, 2) + "</td><td>" + fmtNum(r.swarm1000Ms, 2) + "</td><td>" +
      fmtNum(r.swarm1000RssMb, 2) + " MB</td><td>" + fmtNum(r.rssAvgMb, 2) + " MB</td><td>" + fmtNum(r.rssMaxMb, 2) + " MB</td><td>" + fmtNum(r.cas500ColdMs, 3) + " ms</td></tr>"
    ).join("") + "</tbody>";

  // Interactive workload explorer
  const cats = [...new Set(latest.workloads.map((w) => w.category))].sort();
  const catSel = document.getElementById("catFilter");
  cats.forEach((c) => {
    const opt = document.createElement("option");
    opt.value = c;
    opt.textContent = c;
    catSel.appendChild(opt);
  });

  function renderWorkloadsTable() {
    const q = document.getElementById("searchBox").value.trim().toLowerCase();
    const mode = document.getElementById("metricMode").value;
    const cat = catSel.value;
    const wMaps = REPORTS.map((r) => Object.fromEntries(r.workloads.map((w) => [w.id, w])));
    const rows = latest.workloads.filter((w) =>
      (cat === "all" || w.category === cat) && (!q || w.id.toLowerCase().includes(q))
    );
    document.getElementById("tblWorkloads").innerHTML =
      "<thead><tr><th>Workload</th>" + REPORTS.map((r) => "<th>v" + r.version + "</th>").join("") + "<th>Bun (latest)</th><th>Node (latest)</th></tr></thead><tbody>" +
      rows.map((w) => {
        const cols = wMaps.map((m) => "<td>" + (m[w.id] ? fmtNum(m[w.id][mode], 2) : "-") + "</td>").join("");
        return "<tr><td>" + w.id + "</td>" + cols + "<td>" + fmtNum(w.bunMs, 2) + "</td><td>" + fmtNum(w.nodeMs, 2) + "</td></tr>";
      }).join("") + "</tbody>";
  }

  document.getElementById("searchBox").addEventListener("input", renderWorkloadsTable);
  document.getElementById("metricMode").addEventListener("change", renderWorkloadsTable);
  catSel.addEventListener("change", renderWorkloadsTable);
  renderWorkloadsTable();
})();
</script>
</body>
</html>
`;
}

const reports = loadReports();
if (reports.length === 0) {
  console.error("No bench_v*.json reports found in tests/reports/");
  process.exit(1);
}
const html = buildHtml(reports);
fs.writeFileSync(OUT_HTML, html, "utf8");
console.log(
  `Generated ${path.relative(ROOT_DIR, OUT_HTML)} comparing ${reports.length} releases (${reports.map((r) => "v" + r.version).join(", ")}).`
);
