const LIMIT = 4 * 1024 * 1024;
const DAILY_BYTES = 64 * 1024 * 1024;
const DAILY_REPORTS = 32;

export class DiagnosticsQuota {
  constructor(ctx) {
    this.sql = ctx.storage.sql;
    this.sql.exec("CREATE TABLE IF NOT EXISTS quota (id INTEGER PRIMARY KEY, day INTEGER NOT NULL, reports INTEGER NOT NULL, bytes INTEGER NOT NULL)");
  }

  async fetch(request) {
    const length = Number(request.headers.get("X-Report-Length"));
    if (!Number.isInteger(length) || length < 4 || length > LIMIT) {
      return new Response(null, {status: 400});
    }
    const day = Math.floor(Date.now() / 86400000);
    // A single SQLite statement atomically reserves space across all Worker instances.
    const admitted = this.sql.exec(
      "INSERT INTO quota VALUES (1, ?, 1, ?) ON CONFLICT(id) DO UPDATE SET " +
      "day=excluded.day, reports=CASE WHEN quota.day=excluded.day THEN reports+1 ELSE 1 END, " +
      "bytes=CASE WHEN quota.day=excluded.day THEN bytes+excluded.bytes ELSE excluded.bytes END " +
      "WHERE quota.day<excluded.day OR (quota.day=excluded.day AND reports<? AND bytes+excluded.bytes<=?) RETURNING id",
      day, length, DAILY_REPORTS, DAILY_BYTES,
    ).toArray().length;
    return new Response(null, {status: admitted ? 204 : 429});
  }
}

export async function uploadDiagnostics(request, env) {
  if (env.DIAGNOSTICS_ENABLED !== "true" || !env.DIAGNOSTICS || !env.DIAGNOSTICS_QUOTA) {
    return Response.json({error: "diagnostics_unavailable"}, {status: 503});
  }
  if (request.method !== "POST") {
    return Response.json({error: "method_not_allowed"}, {status: 405});
  }

  const length = Number(request.headers.get("Content-Length"));
  if (!Number.isInteger(length) || length < 4 || length > LIMIT || !request.body) {
    return Response.json({error: "invalid_length"}, {status: 413});
  }
  const report = new Uint8Array(length);
  const reader = request.body.getReader();
  let offset = 0;
  for (;;) {
    const {done, value} = await reader.read();
    if (done) break;
    if (offset + value.length > length) {
      await reader.cancel();
      return Response.json({error: "invalid_length"}, {status: 413});
    }
    report.set(value, offset);
    offset += value.length;
  }
  if (offset !== length || new DataView(report.buffer).getUint32(0, true) !== 0x184d2204) {
    return Response.json({error: "invalid_report"}, {status: 400});
  }
  const quota = env.DIAGNOSTICS_QUOTA.get(env.DIAGNOSTICS_QUOTA.idFromName("global"));
  const reservation = await quota.fetch("https://quota.internal/reserve", {
    method: "POST", headers: {"X-Report-Length": String(length)},
  });
  if (reservation.status === 429) {
    return Response.json({error: "daily_limit"}, {status: 429});
  }
  if (reservation.status !== 204) {
    return Response.json({error: "diagnostics_unavailable"}, {status: 503});
  }
  const id = crypto.randomUUID().slice(0, 8);
  const timestamp = new Date().toISOString();
  const device = (request.headers.get("X-P2P-Device") ?? "")
    .replace(/[^A-Za-z0-9-]/g, "-").slice(0, 63).replace(/^-+|-+$/g, "") || "unknown-device";
  const keyTime = timestamp.slice(0, 19).replace("T", "_").replaceAll(":", "-") + "Z";
  const key = keyTime + "_" + device + "_" + id + ".jsonl.lz4";
  await env.DIAGNOSTICS.put(key, report, {
    expirationTtl: 7 * 24 * 60 * 60,
  });
  return Response.json({id, timestamp}, {status: 201, headers: {"Cache-Control": "no-store"}});
}
