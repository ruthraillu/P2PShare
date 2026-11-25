import {test} from "node:test";
import assert from "node:assert/strict";
import {fileURLToPath} from "node:url";
import {Miniflare} from "miniflare";
import worker from "../src/index.js";

test("uploads default to disabled", async () => {
  const response = await worker.fetch(new Request("https://worker.test/diagnostics", {method: "POST"}), {});
  assert.equal(response.status, 503);
});

test("private KV accepts tokenless uploads within the shared quota", async () => {
  const mf = new Miniflare({
    modules: true, modulesRules: [{type: "ESModule", include: ["**/*.js"]}],
    scriptPath: fileURLToPath(new URL("../src/index.js", import.meta.url)),
    compatibilityDate: "2026-08-01",
    bindings: {DIAGNOSTICS_ENABLED: "true"},
    durableObjects: {DIAGNOSTICS_QUOTA: {className: "DiagnosticsQuota", useSQLite: true}},
    kvNamespaces: ["DIAGNOSTICS"],
  });
  try {
    const report = Uint8Array.of(4, 34, 77, 24, 1);
    const send = body => mf.dispatchFetch("https://worker.test/diagnostics", {method: "POST", headers: {"Content-Length": String(body.length), "X-P2P-Device": "DESKTOP/Test"}, body});
    assert.equal((await send(Uint8Array.of(0, 0, 0, 0))).status, 400);
    assert.equal((await send(new Uint8Array(4194305))).status, 413);
    const accepted = await send(report);
    assert.equal(accepted.status, 201);
    const {id, timestamp} = await accepted.json();
    assert.match(id, /^[a-f0-9]{8}$/);
    const key = timestamp.slice(0, 19).replace("T", "_").replaceAll(":", "-") + "Z_DESKTOP-Test_" + id + ".jsonl.lz4";
    const kv = await mf.getKVNamespace("DIAGNOSTICS");
    assert.deepEqual(new Uint8Array(await kv.get(key, "arrayBuffer")), report);
    const {keys} = await kv.list();
    assert.ok(Math.abs(keys[0].expiration - Date.now() / 1000 - 604800) < 10);
    const statuses = await Promise.all(Array.from({length: 35}, async () => (await send(report)).status));
    assert.equal(statuses.filter(status => status === 201).length, 31);
    assert.equal(statuses.filter(status => status === 429).length, 4);
  } finally {
    await mf.dispose();
  }
});
