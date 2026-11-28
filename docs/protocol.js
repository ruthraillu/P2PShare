import {
  base64UrlDecode,
  base64UrlEncode,
  bytesToHex,
  publicKeyFromSeed,
  sha256Hex,
  sign,
} from "./ed25519.js";

const textDecoder = new TextDecoder();

export function canonicalRoom(value) {
  if (typeof value !== "string") throw new Error("Enter a room name");
  const name = value.trim().replace(/\s+/g, " ").toLowerCase();
  if (!/^[a-z0-9 _-]{3,64}$/.test(name)) {
    throw new Error("Room name must be 3–64 letters, numbers, spaces, hyphens, or underscores");
  }
  return name;
}

export function validateServer(raw) {
  if (typeof raw !== "string" || !raw.trim()) {
    throw new Error("Enter the signaling server URL");
  }
  const trimmed = raw.trim().replace(/\/+$/, "");
  if (trimmed.length > 2048 || trimmed.includes("@") || trimmed.includes("#")) {
    throw new Error("Use a clean wss:// signaling server URL");
  }
  let url;
  try {
    url = new URL(trimmed);
  } catch {
    throw new Error("Invalid signaling server URL");
  }
  const host = url.hostname.toLowerCase();
  const loopback = host === "127.0.0.1" || host === "localhost" || host === "[::1]";
  const secure = url.protocol === "wss:" || (loopback && url.protocol === "ws:");
  if (!secure || url.pathname !== "/" || url.search) {
    throw new Error("Use a wss:// signaling server URL (ws:// is only allowed on localhost)");
  }
  return `${url.protocol}//${url.host}`;
}

export function parseInvite(input) {
  if (typeof input !== "string") return null;
  const trimmed = input.trim();
  const prefix = "p2pshare://room/";
  if (!trimmed.startsWith(prefix)) return null;
  const encoded = trimmed.slice(prefix.length);
  const jsonBytes = base64UrlDecode(encoded);
  const parsed = JSON.parse(textDecoder.decode(jsonBytes));
  if (!parsed || typeof parsed !== "object") throw new Error("Invalid room invitation");
  if ((parsed.version !== 6 && parsed.version !== 7) || typeof parsed.room !== "string") {
    throw new Error("Only named room invitations are supported in the web viewer");
  }
  return {
    server: validateServer(parsed.server),
    room: canonicalRoom(parsed.room),
  };
}

export async function buildSignedRoomUrl(serverInput, roomInput, seed32, nowSec = Math.floor(Date.now() / 1000)) {
  const server = validateServer(serverInput);
  const room = canonicalRoom(roomInput);
  const roomHash = await sha256Hex(`P2PShare named room v1:${room}`);
  const peerId = bytesToHex(await publicKeyFromSeed(seed32));
  const sessionBytes = new Uint8Array(16);
  crypto.getRandomValues(sessionBytes);
  const session = base64UrlEncode(sessionBytes);
  const ts = String(nowSec).padStart(10, "0");
  const proof = `P2PShare signaling v3\n${roomHash}\n${peerId}\n${session}\n${ts}`;
  const sig = base64UrlEncode(await sign(proof, seed32));

  const wsUrl = new URL(`${server}/v3/rooms/${roomHash}`);
  wsUrl.searchParams.set("room", room);
  wsUrl.searchParams.set("peer", peerId);
  wsUrl.searchParams.set("session", session);
  wsUrl.searchParams.set("ts", ts);
  wsUrl.searchParams.set("sig", sig);
  return { url: wsUrl.toString(), server, room, roomHash, peerId, session };
}

function isValidIpv4(ip) {
  const octets = ip.split(".");
  if (octets.length !== 4) return false;
  for (const part of octets) {
    if (!/^\d{1,3}$/.test(part)) return false;
    const n = Number(part);
    if (n < 0 || n > 255 || (part.length > 1 && part.startsWith("0"))) return false;
  }
  return true;
}

function isValidIpv6(ip) {
  if (!ip.includes(":") || /[^0-9a-fA-F:]/.test(ip)) return false;
  const parts = ip.split("::");
  if (parts.length > 2) return false;
  const left = parts[0] ? parts[0].split(":") : [];
  const right = parts.length === 2 && parts[1] ? parts[1].split(":") : [];
  const validGroups = left.concat(right);
  if (validGroups.some(g => !/^[0-9a-fA-F]{1,4}$/.test(g))) return false;
  return parts.length === 2 ? left.length + right.length < 8 : left.length === 8;
}

export function isValidIpLiteral(ip) {
  return isValidIpv4(ip) || isValidIpv6(ip);
}

function isDisallowedIp(ip) {
  if (isValidIpv4(ip)) {
    const first = Number(ip.split(".")[0]);
    if (ip === "0.0.0.0") return true;
    if (first >= 224 && first <= 239) return true;
    return false;
  }
  const lower = ip.toLowerCase();
  if (lower.startsWith("ff")) return true;
  if (/^[0:]+$/.test(lower)) return true;
  return false;
}

/**
 * Normalizes a WebRTC ICE candidate string to match P2PShare's strict Rust validator
 * (UDP only, numeric IPv4/IPv6 only, valid port, clean raddr extension).
 */
export function filterIceCandidate(raw) {
  if (typeof raw !== "string") return null;
  let line = raw.trim();
  if (line.startsWith("a=")) line = line.slice(2);
  if (!line.startsWith("candidate:") || line.length > 1024) return null;

  const parts = line.split(/\s+/);
  if (parts.length < 8) return null;
  const [foundationTok, compStr, proto, prioStr, ip, portStr, typKw, kind] = parts;
  if (!/^\d+$/.test(compStr) || !/^\d+$/.test(prioStr)) return null;
  if (proto.toLowerCase() !== "udp") return null;
  if (!isValidIpLiteral(ip) || isDisallowedIp(ip)) return null;

  const port = Number(portStr);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  if (typKw !== "typ") return null;
  if (!["host", "srflx", "prflx", "relay"].includes(kind)) return null;

  const cleaned = [foundationTok, compStr, "udp", prioStr, ip, portStr, "typ", kind];
  for (let i = 8; i + 1 < parts.length; i += 2) {
    const k = parts[i];
    const v = parts[i + 1];
    if (k === "raddr") {
      if (!isValidIpLiteral(v)) {
        if (parts[i + 2] === "rport") i += 2;
        continue;
      }
    }
    cleaned.push(k, v);
  }
  return cleaned.join(" ");
}

export function sanitizeSdp(sdp) {
  if (typeof sdp !== "string") return sdp;
  const eol = sdp.includes("\r\n") ? "\r\n" : "\n";
  const lines = sdp.split(/\r?\n/);
  const out = [];
  for (const line of lines) {
    if (line.startsWith("a=candidate:")) {
      const cleaned = filterIceCandidate(line);
      if (!cleaned) continue;
      out.push(`a=${cleaned}`);
    } else {
      out.push(line);
    }
  }
  return out.join(eol);
}

export function parseControlMessage(data) {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
  if (!bytes.length) return null;
  const tag = bytes[0];
  const body = bytes.subarray(1);
  switch (tag) {
    case 2:
      return body.length ? { type: "preview", jpeg: body } : null;
    case 3: {
      if (body.length > 511) return null;
      const info = JSON.parse(textDecoder.decode(body));
      if (!info || typeof info !== "object" || typeof info.bitrate_kbps !== "number") return null;
      return { type: "streamInfo", info };
    }
    case 5: {
      const viewers = JSON.parse(textDecoder.decode(body));
      if (!Array.isArray(viewers)) return null;
      return { type: "viewers", viewers: viewers.filter(v => typeof v === "string") };
    }
    case 6: {
      const hash = textDecoder.decode(body);
      if (hash !== "" && !/^[a-fA-F0-9]{64}$/.test(hash)) return null;
      return { type: "avatarHash", hash: hash.toLowerCase() };
    }
    case 8:
      return body.length ? { type: "avatarData", bytes: body } : null;
    default:
      return null;
  }
}
