import type { StreamsAndCloneResult } from "./types.ts";

export async function runWinterTcStreamsAndCloningPipeline(): Promise<StreamsAndCloneResult> {
  const enc = new TextEncoder();

  // 1. EventTarget + CustomEvent + AbortSignal.any cancellation orchestration
  const bus = new EventTarget();
  const dispatchedEvents: string[] = [];
  const subController = new AbortController();

  bus.addEventListener(
    "stage",
    (ev: any) => {
      dispatchedEvents.push("once:" + ev.detail.step);
    },
    { once: true }
  );
  bus.addEventListener(
    "stage",
    (ev: any) => {
      dispatchedEvents.push("live:" + ev.detail.step);
    },
    { signal: subController.signal }
  );

  bus.dispatchEvent(new CustomEvent("stage", { detail: { step: "init" } }));
  bus.dispatchEvent(new CustomEvent("stage", { detail: { step: "transform" } }));
  subController.abort("listener-detached");
  bus.dispatchEvent(new CustomEvent("stage", { detail: { step: "ignored" } }));

  const primaryController = new AbortController();
  const secondaryController = new AbortController();
  const compositeSignal = AbortSignal.any([primaryController.signal, secondaryController.signal]);
  let anySignalAbortedBy = "none";
  compositeSignal.addEventListener("abort", () => {
    anySignalAbortedBy = String(compositeSignal.reason);
  });
  secondaryController.abort("secondary-deadline-exceeded");

  // 2. Blob, File, FormData & Request/Response binary body round-trip
  const headerPart = new Blob(["SLICE-EDGE::"], { type: "text/plain" });
  const bodyPart = enc.encode("wintertc-binary-payload-v3");
  const artifactFile = new File([headerPart, bodyPart], "bundle-manifest.bin", {
    type: "application/octet-stream",
    lastModified: 1700000005000
  });
  const slicedHeaderText = await artifactFile.slice(0, 10).text();

  const form = new FormData();
  form.append("tenant", "acme-edge");
  form.append("region", "iad-01");
  form.append("bundle", artifactFile);

  const formKeys = Array.from(form.keys());
  const uploadedFile = form.get("bundle") as File;

  // 3. structuredClone with cyclic reference graph, Map, Set, and Uint8Array isolation
  const stateGraph: any = {
    id: "graph-01",
    regions: new Set(["iad", "fra", "nrt"]),
    quotas: new Map([
      ["cpuMs", 50],
      ["memKb", 4096]
    ]),
    bytes: new Uint8Array([11, 22, 33, 44])
  };
  stateGraph.selfRef = stateGraph;
  const clonedGraph = structuredClone(stateGraph);
  clonedGraph.bytes[0] = 99;
  const cyclicCloneVerified =
    clonedGraph !== stateGraph &&
    clonedGraph.selfRef === clonedGraph &&
    stateGraph.bytes[0] === 11 &&
    clonedGraph.bytes[0] === 99 &&
    clonedGraph.quotas.get("memKb") === 4096 &&
    clonedGraph.regions.has("fra");

  // 4. WhatWG ReadableStream -> TransformStream -> TextEncoderStream -> TextDecoderStream -> WritableStream
  const sanitizeTransform = new TransformStream({
    transform(chunk: string, controller: any) {
      controller.enqueue(String(chunk).trim().toUpperCase());
    },
    flush(controller: any) {
      controller.enqueue("::EOF");
    }
  });

  const collectedStrings: string[] = [];
  let totalTransformedBytes = 0;
  const byteCounterTransform = new TransformStream({
    transform(u8Chunk: Uint8Array, controller: any) {
      totalTransformedBytes += u8Chunk.byteLength;
      controller.enqueue(u8Chunk);
    }
  });

  const sink = new WritableStream({
    write(chunk: string) {
      collectedStrings.push(chunk);
    }
  });

  const sourceStream = new ReadableStream({
    start(controller: any) {
      controller.enqueue("  route:api  |");
      controller.enqueue("  auth:jwt  |");
      controller.enqueue("  stream:ok  ");
      controller.close();
    }
  });

  await sourceStream
    .pipeThrough(sanitizeTransform)
    .pipeThrough(new TextEncoderStream())
    .pipeThrough(byteCounterTransform)
    .pipeThrough(new TextDecoderStream())
    .pipeTo(sink);

  return {
    transformedPayload: collectedStrings.join(""),
    totalTransformedBytes,
    formFieldsCount: formKeys.length,
    uploadedFileName: uploadedFile.name,
    uploadedFileSize: uploadedFile.size,
    slicedHeaderText,
    cyclicCloneVerified,
    dispatchedEvents,
    anySignalAbortedBy
  };
}
