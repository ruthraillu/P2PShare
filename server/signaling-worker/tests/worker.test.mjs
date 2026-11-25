import {test, after} from "node:test";
import assert from "node:assert/strict";
import {createHash, generateKeyPairSync, randomBytes, sign} from "node:crypto";
import {fileURLToPath} from "node:url";
import {Miniflare} from "miniflare";
import {proofMessage, namedProofMessage, canonicalRoom} from "../src/index.js";
const mf = new Miniflare({modules: true, modulesRules: [{type: "ESModule", include: ["**/*.js"]}], scriptPath: fileURLToPath(new URL("../src/index.js", import.meta.url)), compatibilityDate: "2026-08-01", durableObjects: {ROOMS: {className: "Room", useSQLite: true}}, durableObjectsPersist: fileURLToPath(new URL("../../../.build/worker-tests/state", import.meta.url))});
after(async () => mf.dispose());
function identity() {
  const {privateKey, publicKey} = generateKeyPairSync("ed25519");
  const id = Buffer.from(publicKey.export({format: "jwk"}).x, "base64url").toString("hex");
  return {privateKey, id};
}
function room() {
  const secret = randomBytes(32);
  return {token: secret.toString("base64url"), id: createHash("sha256").update(secret).digest("hex")};
}
function auth(room, peer) {
  const session = randomBytes(16).toString("base64url");
  const timestamp = String(Math.floor(Date.now() / 1000));
  return {session, headers: {Upgrade: "websocket", Authorization: `Bearer ${room.token}`, "X-P2P-Peer": peer.id, "X-P2P-Session": session, "X-P2P-Timestamp": timestamp, "X-P2P-Signature": sign(null, Buffer.from(proofMessage(room.id, peer.id, session, timestamp)), peer.privateKey).toString("base64url")}};
}
async function response(room, auth) {return mf.dispatchFetch(`https://worker.test/v1/rooms/${room.id}`, {headers: auth.headers});}
function collect(ws) {
  const events = [];
  ws.addEventListener("message", e => {try {events.push(JSON.parse(e.data));} catch {events.push(e.data);}});
  return async predicate => {
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline) {
      const index = events.findIndex(predicate);
      if (index >= 0) return events.splice(index, 1)[0];
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    throw new Error("Missing WebSocket event");
  };
}
async function connect(room, peer = identity()) {
  const credentials = auth(room, peer); const res = await response(room, credentials);
  assert.equal(res.status, 101, await res.text());
  const socket = res.webSocket; const next = collect(socket); socket.accept();
  const welcome = await next(m => m.type === "welcome");
  assert.equal(welcome.session, credentials.session);
  return {socket, next, peer, session: credentials.session};
}
const send = (client, message) => client.socket.send(JSON.stringify(message));
test("health and signed room authentication", async () => {
  assert.deepEqual(await (await mf.dispatchFetch("https://worker.test/health")).json(), {service: "p2pshare-signaling", protocol: 1, status: "ok", named_rooms: true, join_only: true});
  const r = room(); const peer = identity(); const credentials = auth(r, peer);
  credentials.headers.Authorization = `Bearer ${randomBytes(32).toString("base64url")}`;
  assert.equal((await response(r, credentials)).status, 401);
  const forged = auth(r, peer); forged.headers["X-P2P-Peer"] = identity().id;
  assert.equal((await response(r, forged)).status, 401);
  const expired = auth(r, peer); expired.headers["X-P2P-Timestamp"] = "1000000000";
  assert.equal((await response(r, expired)).status, 401);
});
test("room membership, share metadata, directed signaling, and creator independence", async () => {
  const r = room(); const a = await connect(r); const b = await connect(r);
  await a.next(m => m.type === "members" && m.members.length === 2);
  send(a, {type: "share", sharing: true, width: 1280, height: 720, codec: "hevc"});
  const roster = await b.next(m => m.type === "members" && m.members.some(p => p.id === a.peer.id && p.sharing));
  assert.equal(roster.members.find(p => p.id === a.peer.id).codec, "hevc");
  send(a, {type: "signal", to: b.peer.id, to_session: b.session, kind: "offer", payload: "test SDP"});
  const signal = await b.next(m => m.type === "signal"); assert.equal(signal.from, a.peer.id); assert.equal(signal.payload, "test SDP");
  a.socket.send("ping"); await a.next(m => m === "pong");
  send(a, {type: "leave"}); await b.next(m => m.type === "members" && m.members.length === 1);
  const c = await connect(r); assert.equal((await c.next(m => m.type === "members")).members.length, 2);
  b.socket.close(1000); c.socket.close(1000);
});
test("restart replaces stale connection and rejects stale-session SDP", async () => {
  const r = room(); const peer = identity(); const old = await connect(r, peer); const b = await connect(r);
  const closed = new Promise(resolve => old.socket.addEventListener("close", resolve));
  const fresh = await connect(r, peer); assert.equal((await closed).code, 4001);
  const current = await b.next(m => m.type === "members" && m.members.find(p => p.id === peer.id)?.session === fresh.session);
  assert.equal(current.members.length, 2);
  send(b, {type: "signal", to: peer.id, to_session: old.session, kind: "answer", payload: "stale"});
  assert.equal((await b.next(m => m.type === "error")).error, "peer_session_changed");
  send(b, {type: "signal", to: peer.id, to_session: fresh.session, kind: "answer", payload: "fresh"});
  assert.equal((await fresh.next(m => m.type === "signal")).payload, "fresh");
  fresh.socket.close(1000); b.socket.close(1000);
});
test("six-member admission and malformed video-shaped payload rejection", async () => {
  const r = room(); const clients = [];
  for (let i = 0; i < 6; i++) clients.push(await connect(r));
  assert.equal((await response(r, auth(r, identity()))).status, 409);
  send(clients[0], {type: "share", sharing: true, width: 3840, height: 2160, codec: "av1"});
  assert.equal((await clients[0].next(m => m.type === "error")).error, "invalid_message");
  const closed = new Promise(resolve => clients[0].socket.addEventListener("close", resolve));
  clients[0].socket.send(new Uint8Array([1,2,3]).buffer); assert.equal((await closed).code, 1009);
  await clients[1].next(m => m.type === "members" && m.members.length === 5);
  const replacement = await connect(r); replacement.socket.close(1000);
  for (const client of clients.slice(1)) client.socket.close(1000);
});


function namedRoom(name){name=canonicalRoom(name);return{name,id:createHash('sha256').update(`P2PShare named room v1:${name}`).digest('hex')};}
function namedAuth(room,peer){
 const session=randomBytes(16).toString('base64url'),timestamp=String(Math.floor(Date.now()/1000));
 return{Upgrade:'websocket','X-P2P-Room':room.name,'X-P2P-Peer':peer.id,'X-P2P-Session':session,'X-P2P-Timestamp':timestamp,'X-P2P-Signature':sign(null,Buffer.from(namedProofMessage(room.id,peer.id,session,timestamp)),peer.privateKey).toString('base64url')};
}
async function namedConnect(room,peer=identity()){
 const headers=namedAuth(room,peer),response=await mf.dispatchFetch(`https://worker.test/v3/rooms/${room.id}`,{headers});assert.equal(response.status,101,await response.text());
 const socket=response.webSocket,next=collect(socket);socket.accept();await next(m=>m.type==='welcome');return{socket,next,peer};
}
test('room-name-only joins register automatically, isolate rooms and survive creator exit',async()=>{
 const name='auto-'+randomBytes(8).toString('hex'),r=namedRoom(name);
 const a=await namedConnect(r),b=await namedConnect(namedRoom(`  ${name.toUpperCase()}  `));
 await a.next(m=>m.type==='members'&&m.members.length===2);
 const other=await namedConnect(namedRoom(name+'-different'));assert.equal((await other.next(m=>m.type==='members')).members.length,1);
 send(a,{type:'leave'});await b.next(m=>m.type==='members'&&m.members.length===1);
 b.socket.close(1000);other.socket.close(1000);
 const later=await namedConnect(r);later.socket.close(1000);
});
test('signed identity and room name bind public joins; password protocol is retired',async()=>{
 const r=namedRoom('auth-'+randomBytes(8).toString('hex')),peer=identity();
 let headers=namedAuth(r,peer);headers['X-P2P-Room']='some-other-room';assert.equal((await mf.dispatchFetch(`https://worker.test/v3/rooms/${r.id}`,{headers})).status,401);
 headers=namedAuth(r,peer);headers['X-P2P-Peer']=identity().id;assert.equal((await mf.dispatchFetch(`https://worker.test/v3/rooms/${r.id}`,{headers})).status,401);
 assert.equal((await mf.dispatchFetch(`https://worker.test/v2/rooms/${r.id}`,{headers})).status,410);
});


test("named rooms admit more devices than the former room and socket limits", {timeout: 30000}, async () => {
  const room = namedRoom(`capacity-${randomBytes(8).toString("hex")}`);
  const observer = await namedConnect(room);
  const occupants = [];
  try {
    for (let i = 0; i < 25; i++) occupants.push(await namedConnect(room));
    await observer.next(m => m.type === "members" && m.members.length === 26);
  } finally {
    while (occupants.length) {
      const client = occupants.pop();
      send(client, {type: "leave"});
      await observer.next(m => m.type === "members" && m.members.length === occupants.length + 1 &&
        !m.members.some(member => member.id === client.peer.id));
    }
    observer.socket.close(1000);
  }
});
