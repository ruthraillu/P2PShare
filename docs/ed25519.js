// Minimal RFC 8032 Ed25519 + WebCrypto hashing helpers for browsers without native WebCrypto Ed25519.
const P = (1n << 255n) - 19n;
const L = (1n << 252n) + 27742317777372353535851937790883648493n;
const D = 37095705934669439343138083508754565189542113879843219016388785533085940283555n;
const D2 = (2n * D) % P;
const BX = 15112221349535400772501151409588531511454012693041857206046113283949847762202n;
const BY = 46316835694926478169428394003475163141307993866256225615783033603165251855960n;
const IDENTITY = [0n, 1n, 1n, 0n];
const BASE = [BX, BY, 1n, (BX * BY) % P];

const textEncoder = new TextEncoder();

function mod(a, m) {
  const res = a % m;
  return res >= 0n ? res : res + m;
}

function modPow(base, exp, m) {
  let result = 1n;
  let b = mod(base, m);
  let e = exp;
  while (e > 0n) {
    if (e & 1n) result = (result * b) % m;
    b = (b * b) % m;
    e >>= 1n;
  }
  return result;
}

function modInverse(a, m) {
  return modPow(a, m - 2n, m);
}

function pointAdd(p1, p2) {
  const [x1, y1, z1, t1] = p1;
  const [x2, y2, z2, t2] = p2;
  const a = mod((y1 - x1) * (y2 - x2), P);
  const b = mod((y1 + x1) * (y2 + x2), P);
  const c = mod(t1 * D2 * t2, P);
  const d = mod(2n * z1 * z2, P);
  const e = mod(b - a, P);
  const f = mod(d - c, P);
  const g = mod(d + c, P);
  const h = mod(b + a, P);
  return [
    mod(e * f, P),
    mod(g * h, P),
    mod(f * g, P),
    mod(e * h, P),
  ];
}

function scalarMult(point, scalar) {
  let result = IDENTITY;
  let addend = point;
  let k = scalar;
  while (k > 0n) {
    if (k & 1n) result = pointAdd(result, addend);
    addend = pointAdd(addend, addend);
    k >>= 1n;
  }
  return result;
}

function leBytesToBigInt(bytes) {
  let res = 0n;
  for (let i = bytes.length - 1; i >= 0; i--) {
    res = (res << 8n) | BigInt(bytes[i]);
  }
  return res;
}

function bigIntToLeBytes32(n) {
  const out = new Uint8Array(32);
  let v = n;
  for (let i = 0; i < 32; i++) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

function encodePoint(point) {
  const [X, Y, Z] = point;
  const zi = modInverse(Z, P);
  const x = mod(X * zi, P);
  const y = mod(Y * zi, P);
  const encoded = bigIntToLeBytes32(y);
  if ((x & 1n) === 1n) encoded[31] |= 0x80;
  return encoded;
}

function concatBytes(...chunks) {
  const total = chunks.reduce((sum, c) => sum + c.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

async function sha512(bytes) {
  const digest = await crypto.subtle.digest("SHA-512", bytes);
  return new Uint8Array(digest);
}

async function expandSeed(seed32) {
  if (!(seed32 instanceof Uint8Array) || seed32.length !== 32) {
    throw new Error("Ed25519 seed must be 32 bytes");
  }
  const h = await sha512(seed32);
  const scalarBytes = h.slice(0, 32);
  scalarBytes[0] &= 248;
  scalarBytes[31] &= 127;
  scalarBytes[31] |= 64;
  return {
    scalar: leBytesToBigInt(scalarBytes),
    prefix: h.slice(32, 64),
  };
}

export function bytesToHex(bytes) {
  return Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
}

export function hexToBytes(hex) {
  if (typeof hex !== "string" || hex.length % 2 !== 0 || !/^[a-fA-F0-9]*$/.test(hex)) {
    throw new Error("Invalid hex string");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function base64UrlEncode(bytes) {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function base64UrlDecode(str) {
  if (typeof str !== "string" || !/^[A-Za-z0-9_-]+$/.test(str)) {
    throw new Error("Invalid base64url encoding");
  }
  const padded = str.replaceAll("-", "+").replaceAll("_", "/").padEnd(Math.ceil(str.length / 4) * 4, "=");
  const binary = atob(padded);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

export async function sha256Hex(input) {
  const bytes = typeof input === "string" ? textEncoder.encode(input) : input;
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return bytesToHex(new Uint8Array(digest));
}

export async function publicKeyFromSeed(seed32) {
  const { scalar } = await expandSeed(seed32);
  return encodePoint(scalarMult(BASE, scalar));
}

export async function sign(message, seed32) {
  const msgBytes = typeof message === "string" ? textEncoder.encode(message) : message;
  const { scalar, prefix } = await expandSeed(seed32);
  const pub = encodePoint(scalarMult(BASE, scalar));
  const rHash = await sha512(concatBytes(prefix, msgBytes));
  const r = mod(leBytesToBigInt(rHash), L);
  const R = encodePoint(scalarMult(BASE, r));
  const kHash = await sha512(concatBytes(R, pub, msgBytes));
  const k = mod(leBytesToBigInt(kHash), L);
  const S = mod(r + k * scalar, L);
  return concatBytes(R, bigIntToLeBytes32(S));
}
