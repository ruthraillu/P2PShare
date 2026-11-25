import { uploadDiagnostics } from "./diagnostics.js";
export { DiagnosticsQuota } from "./diagnostics.js";
// P2PShare signaling only: no video, audio, TURN or file transfers.
const VERSION = 1;
const LEGACY_ROOM_LIMIT = 6;
const MAX_MESSAGE = 65536;
const HEARTBEAT_TIMEOUT = 30000;
const SWEEP_INTERVAL = 15000;
const text = new TextEncoder();
function json(data, status = 200) {
  return Response.json(data, {status, headers: {"Cache-Control": "no-store"}});
}
function fromBase64url(value, length) {
  if (typeof value !== "string" || !/^[A-Za-z0-9_-]+$/.test(value)) throw new Error("encoding");
  const raw = atob(value.replaceAll("-", "+").replaceAll("_", "/"));
  if (raw.length !== length) throw new Error("length");
  return Uint8Array.from(raw, c => c.charCodeAt(0));
}
export function proofMessage(room, peer, session, timestamp) {
  return `P2PShare signaling v1\n${room}\n${peer}\n${session}\n${timestamp}`;
}
export function namedProofMessage(room,peer,session,timestamp) {
  return `P2PShare signaling v3\n${room}\n${peer}\n${session}\n${timestamp}`;
}
export function canonicalRoom(value) {
  if(typeof value!=="string")throw new Error("name");
  const name=value.trim().replace(/\s+/g," ").toLowerCase();
  if(!/^[a-z0-9 _-]{3,64}$/.test(name))throw new Error("name");
  return name;
}
function hex(raw){return Array.from(new Uint8Array(raw),b=>b.toString(16).padStart(2,"0")).join("");}
export default {
  async fetch(request,env) {
    const url=new URL(request.url);
    if (url.pathname === "/diagnostics") return uploadDiagnostics(request, env);
    if(request.method==="GET"&&["/","/health"].includes(url.pathname))return json({service:"p2pshare-signaling",protocol:VERSION,status:"ok",named_rooms:true,join_only:true});
    if(url.pathname.startsWith("/v2/rooms/"))return json({error:"password_rooms_retired_update_app"},410);
    const match=/^\/v([13])\/rooms\/([a-f0-9]{64})$/.exec(url.pathname);
    if(!match)return json({error:"not_found"},404);
    if(request.method!=="GET")return json({error:"method_not_allowed"},405);
    if(request.headers.get("Upgrade")?.toLowerCase()!=="websocket")return json({error:"websocket_required"},426);
    const named=match[1]==="3",room=match[2];
    const peer=request.headers.get("X-P2P-Peer")??"",session=request.headers.get("X-P2P-Session")??"",timestamp=request.headers.get("X-P2P-Timestamp")??"";
    try {
      if(named) {
        const name=canonicalRoom(request.headers.get("X-P2P-Room"));
        const hash=hex(await crypto.subtle.digest("SHA-256",text.encode(`P2PShare named room v1:${name}`)));
        if(hash!==room)throw new Error("room");
      }else{
        const token=request.headers.get("Authorization")?.match(/^Bearer ([A-Za-z0-9_-]{43})$/)?.[1];
        if(hex(await crypto.subtle.digest("SHA-256",fromBase64url(token,32)))!==room)throw new Error("token");
      }
      if(!/^[a-f0-9]{64}$/.test(peer)||!/^\d{10}$/.test(timestamp))throw new Error("auth");
      fromBase64url(session,16);
      if(Math.abs(Date.now()/1000-Number(timestamp))>30)throw new Error("expired");
      const publicKey=Uint8Array.from(peer.match(/../g),b=>parseInt(b,16));
      const key=await crypto.subtle.importKey("raw",publicKey,{name:"Ed25519"},false,["verify"]);
      if(!await crypto.subtle.verify("Ed25519",key,fromBase64url(request.headers.get("X-P2P-Signature"),64),text.encode(named?namedProofMessage(room,peer,session,timestamp):proofMessage(room,peer,session,timestamp))))throw new Error("signature");
    }catch{return json({error:"unauthorized"},401);}
    // Public named rooms are independent of retired password-bound rooms.
    return env.ROOMS.get(env.ROOMS.idFromName(named?`public:${room}`:room)).fetch(request);
  }
};
export class Room {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
    this.members = new Map();
    for (const socket of ctx.getWebSockets()) {
      const state = socket.deserializeAttachment();
      if (state && !state.retired) this.members.set(state.id, {socket, state});
    }
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }
  roster() {
    return Array.from(this.members.values(), ({state}) => {
      const {id, session, name, sharing, width, height, codec} = state;
      return {id, session, name, sharing, width, height, codec};
    });
  }
  send(socket, data) {
    try { socket.send(JSON.stringify(data)); return true; } catch { return false; }
  }
  broadcast() {
    const data = {type: "members", members: this.roster()};
    for (const {socket} of this.members.values()) this.send(socket, data);
  }
  async fetch(request) {
    const id = request.headers.get("X-P2P-Peer");
    const session = request.headers.get("X-P2P-Session");
    this.sweep();
    const previous = this.members.get(id);
    if (previous?.state.session === session) return json({error: "session_replay"}, 422);
    const named = new URL(request.url).pathname.startsWith("/v3/rooms/");
    if (!named) {
      if (!previous && this.members.size >= LEGACY_ROOM_LIMIT) return json({error: "room_full"}, 409);
      if (this.ctx.getWebSockets().length >= 24) return json({error: "room_busy"}, 429);
    }
    const pair = new WebSocketPair();
    const [client, socket] = Object.values(pair);
    const state = {id, session, name: "Guest", sharing: false, width: 0, height: 0, codec: "h264", lastActive: Date.now(), window: Date.now(), messages: 0};
    this.ctx.acceptWebSocket(socket);
    socket.serializeAttachment(state);
    this.members.set(id, {socket, state});
    // Install the new connection first; the old close callback cannot remove it.
    if (previous) {
      previous.socket.serializeAttachment({...previous.state, retired: true});
      previous.socket.close(4001, "Replaced by a fresh connection");
    }
    this.send(socket, {type: "welcome", protocol: VERSION, id, session, members: this.roster()});
    this.broadcast();
    await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL);
    return new Response(null, {status: 101, webSocket: client});
  }
  sweep() {
    let changed = false;
    for (const [id, member] of this.members) {
      const pong = this.ctx.getWebSocketAutoResponseTimestamp(member.socket);
      const lastActive = Math.max(member.state.lastActive, pong?.getTime() ?? 0);
      if (Date.now() - lastActive > HEARTBEAT_TIMEOUT) {
        this.members.delete(id);
        member.socket.serializeAttachment({...member.state, retired: true});
        member.socket.close(4000, "Heartbeat expired");
        changed = true;
      }
    }
    if (changed) this.broadcast();
  }
  async alarm() {
    this.sweep();
    if (this.members.size) await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL);
  }
  webSocketMessage(socket, raw) {
    const state = socket.deserializeAttachment();
    if (!state || this.members.get(state.id)?.socket !== socket) return;
    if (typeof raw !== "string" || text.encode(raw).length > MAX_MESSAGE) {
      this.remove(socket, 1009, "Signaling text only, up to 64 KiB"); return;
    }
    // Control messages are sparse. Bound bursts without persisting to SQL per message.
    const now = Date.now();
    if (now - state.window > 1000) {state.window = now; state.messages = 0;}
    if (++state.messages > 60) {this.remove(socket, 1008, "Message rate exceeded"); return;}
    state.lastActive = now;
    socket.serializeAttachment(state);
    this.members.get(state.id).state = state;
    let data;
    try {data = JSON.parse(raw);} catch {this.send(socket, {type: "error", error: "invalid_json"}); return;}
    if (!data || typeof data !== "object" || Array.isArray(data)) {this.send(socket, {type: "error", error: "invalid_message"}); return;}
    switch (data.type) {
      case "name":
        if (typeof data.name !== "string") break;
        state.name = Array.from(data.name.replace(/[\u0000-\u001f\u007f]/g, "").trim()).slice(0, 32).join("") || "Guest";
        socket.serializeAttachment(state); this.broadcast(); return;
      case "share":
        if (typeof data.sharing !== "boolean") break;
        if (data.sharing && (!Number.isInteger(data.width) || !Number.isInteger(data.height) || data.width < 64 || data.height < 64 || data.width > 1920 || data.height > 1920 || data.width % 2 || data.height % 2 || data.width * data.height > 1920 * 1080 || !["h264", "hevc", "av1"].includes(data.codec))) break;
        Object.assign(state, {sharing: data.sharing, width: data.sharing ? data.width : 0, height: data.sharing ? data.height : 0, codec: data.sharing ? data.codec : "h264"});
        socket.serializeAttachment(state); this.broadcast(); return;
      case "signal": {
        const target = this.members.get(data.to);
        if (!target || target.state.session !== data.to_session) {this.send(socket, {type: "error", error: "peer_session_changed"}); return;}
        if (!["offer", "answer", "ice"].includes(data.kind) || typeof data.payload !== "string" || text.encode(data.payload).length > 49152) break;
        this.send(target.socket, {type: "signal", from: state.id, from_session: state.session, to_session: target.state.session, kind: data.kind, payload: data.payload}); return;
      }
      case "leave": this.remove(socket, 1000, "Left room"); return;
      default: break;
    }
    this.send(socket, {type: "error", error: "invalid_message"});
  }
  remove(socket, code = 1000, reason = "Disconnected") {
    const state = socket.deserializeAttachment();
    if (state && this.members.get(state.id)?.socket === socket) {
      this.members.delete(state.id);
      socket.serializeAttachment({...state, retired: true});
      this.broadcast();
    }
    try {socket.close(code, reason);} catch {}
  }
  webSocketClose(socket, code) {this.remove(socket, code === 1005 || code === 1006 ? 1000 : code);}
  webSocketError(socket) {this.remove(socket, 1011, "Connection error");}
}
