import { sha256Hex } from "./ed25519.js";
import { buildSignedRoomUrl, filterIceCandidate, parseControlMessage, sanitizeSdp } from "./protocol.js";

const ICE_SERVERS = [
  { urls: "stun:stun.cloudflare.com:3478" },
  { urls: "stun:stun.l.google.com:19302" },
  { urls: "stun:stun1.l.google.com:19302" },
];

const HEARTBEAT_MS = 10000;
const HEARTBEAT_TIMEOUT_MS = 25000;
const RECONNECT_BACKOFFS = [1000, 2000, 4000, 8000];
const textEncoder = new TextEncoder();

function randomAttemptToken() {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
}

function forceRecvOnly(pc) {
  for (const t of pc.getTransceivers()) {
    try {
      t.direction = "recvonly";
    } catch {
      // Ignore browsers where a transceiver direction cannot be lowered before answer.
    }
  }
}

export class RoomClient {
  constructor(callbacks = {}) {
    this.cb = callbacks;
    this.ws = null;
    this.config = null;
    this.localId = "";
    this.ownSession = "";
    this.inRoom = false;
    this.disposed = false;
    this.failures = 0;
    this.lastRx = 0;
    this.heartbeatTimer = null;
    this.reconnectTimer = null;
    this.connectGeneration = 0;
    this.members = [];
    this.peers = new Map();
    this.avatarCache = new Map();
  }

  emitStatus(text, kind = "info") {
    this.cb.onStatus?.(text, kind);
  }

  async connect({ server, room, name, seed }) {
    this.disconnect(true);
    this.disposed = false;
    this.failures = 0;
    this.config = { server, room, name, seed };
    await this.#openSocket();
  }

  disconnect(silent = false) {
    this.disposed = true;
    this.connectGeneration++;
    clearInterval(this.heartbeatTimer);
    clearTimeout(this.reconnectTimer);
    this.heartbeatTimer = null;
    this.reconnectTimer = null;
    if (this.ws) {
      const sock = this.ws;
      this.ws = null;
      try {
        if (sock.readyState === WebSocket.OPEN) {
          sock.send(JSON.stringify({ type: "leave" }));
        }
      } catch {
        // Ignore send failures on teardown.
      }
      try {
        sock.close(1000, "Left room");
      } catch {
        // Ignore close failures.
      }
    }
    this.#resetAllPeers();
    this.inRoom = false;
    this.members = [];
    this.cb.onMembers?.([]);
    if (!silent) this.emitStatus("Disconnected", "muted");
  }

  setName(name) {
    if (!this.config) return;
    this.config.name = name;
    this.#sendJson({ type: "name", name });
  }

  watch(peerId, enabled) {
    const peer = this.peers.get(peerId);
    if (!peer) return;
    peer.watching = Boolean(enabled);
    if (peer.channel && peer.channel.readyState === "open") {
      peer.channel.send(new Uint8Array([1, peer.watching ? 1 : 0]));
    }
    if (!peer.watching) {
      // Nothing to tear down on WebRTC; sender stops sending video/audio after [1, 0].
    }
  }

  getStream(peerId) {
    return this.peers.get(peerId)?.stream ?? null;
  }

  getPeerState(peerId) {
    return this.peers.get(peerId)?.iceState ?? "new";
  }

  async #openSocket() {
    if (this.disposed || !this.config) return;
    const gen = ++this.connectGeneration;
    this.emitStatus("Connecting to signaling server…", "busy");

    let signed;
    try {
      signed = await buildSignedRoomUrl(this.config.server, this.config.room, this.config.seed);
    } catch (err) {
      this.emitStatus(err.message || "Invalid room configuration", "error");
      return;
    }
    if (gen !== this.connectGeneration || this.disposed) return;

    this.localId = signed.peerId;
    const ws = new WebSocket(signed.url);
    this.ws = ws;
    this.lastRx = Date.now();

    ws.onopen = () => {
      if (gen !== this.connectGeneration) return;
      this.lastRx = Date.now();
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = setInterval(() => {
        if (this.ws !== ws || ws.readyState !== WebSocket.OPEN) return;
        if (Date.now() - this.lastRx > HEARTBEAT_TIMEOUT_MS) {
          ws.close(4000, "Heartbeat timeout");
          return;
        }
        try {
          ws.send("ping");
        } catch {
          // Wait for close handler.
        }
      }, HEARTBEAT_MS);
    };

    ws.onmessage = async event => {
      if (gen !== this.connectGeneration || typeof event.data !== "string") return;
      this.lastRx = Date.now();
      if (event.data === "pong") return;
      let msg;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }
      await this.#handleMessage(msg);
    };

    ws.onclose = ev => {
      if (gen !== this.connectGeneration) return;
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
      this.ws = null;
      this.inRoom = false;

      if (this.disposed) return;
      if (ev.code === 4001) {
        this.emitStatus("Replaced by another tab or device. Reconnect to take over.", "error");
        this.cb.onError?.("Session replaced (code 4001)");
        return;
      }
      this.#scheduleReconnect();
    };

    ws.onerror = () => {
      // Browser fires onclose right after onerror.
    };
  }

  #scheduleReconnect() {
    if (this.disposed || !this.config) return;
    const delay = RECONNECT_BACKOFFS[Math.min(this.failures, RECONNECT_BACKOFFS.length - 1)];
    this.failures++;
    this.emitStatus(`Signaling disconnected • retrying in ${Math.round(delay / 1000)}s…`, "busy");
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => {
      this.#openSocket();
    }, delay);
  }

  #sendJson(obj) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return false;
    try {
      this.ws.send(JSON.stringify(obj));
      return true;
    } catch {
      return false;
    }
  }

  async #handleMessage(msg) {
    switch (msg.type) {
      case "welcome": {
        this.ownSession = msg.session;
        this.inRoom = true;
        this.failures = 0;
        this.#resetAllPeers();
        this.emitStatus("Room connected • direct P2P ready", "ok");
        this.#sendJson({ type: "name", name: this.config?.name || "Viewer" });
        await this.#applyRoster(msg.members || []);
        break;
      }
      case "members": {
        if (!this.inRoom) return;
        await this.#applyRoster(msg.members || []);
        break;
      }
      case "signal": {
        if (!this.inRoom) return;
        await this.#handleSignal(msg);
        break;
      }
      case "error": {
        if (msg.error && msg.error !== "peer_session_changed") {
          this.emitStatus(`Signaling: ${msg.error}`, "error");
        }
        break;
      }
      default:
        break;
    }
  }

  async #applyRoster(rawMembers) {
    this.members = rawMembers;
    const activeIds = new Set(rawMembers.map(m => m.id));

    for (const [peerId, peer] of this.peers.entries()) {
      if (!activeIds.has(peerId)) {
        this.#destroyPeer(peerId, peer);
        this.peers.delete(peerId);
      }
    }

    for (const member of rawMembers) {
      if (member.id === this.localId) continue;
      const existing = this.peers.get(member.id);
      if (existing && existing.session !== member.session) {
        const keepWatching = existing.watching;
        this.#destroyPeer(member.id, existing);
        this.peers.delete(member.id);
        await this.#initPeer(member, keepWatching);
      } else if (!existing) {
        await this.#initPeer(member, false);
      } else {
        existing.member = member;
      }
    }

    this.cb.onMembers?.(this.members);
  }

  async #initPeer(member, watching = false) {
    const peer = {
      id: member.id,
      session: member.session,
      member,
      pc: null,
      channel: null,
      stream: new MediaStream(),
      token: null,
      negotiated: false,
      watching,
      iceState: "new",
      remoteDescSet: false,
      pendingRemoteCandidates: [],
      pendingLocalCandidates: [],
      sentCandidates: 0,
      announcedAvatarHash: "",
      retryTimer: null,
      disconnectTimer: null,
    };
    this.peers.set(member.id, peer);

    if (this.localId < member.id) {
      await this.#startOffer(peer);
    }
  }

  #createPeerConnection(peer) {
    if (peer.pc) {
      try {
        peer.pc.close();
      } catch {
        // Ignore.
      }
    }
    clearTimeout(peer.retryTimer);
    clearTimeout(peer.disconnectTimer);

    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    peer.pc = pc;
    peer.channel = null;
    peer.remoteDescSet = false;
    peer.pendingRemoteCandidates = [];
    peer.pendingLocalCandidates = [];
    peer.sentCandidates = 0;
    peer.stream = new MediaStream();

    pc.ontrack = ev => {
      if (peer.pc !== pc) return;
      if (ev.track && !peer.stream.getTracks().includes(ev.track)) {
        peer.stream.addTrack(ev.track);
      }
      this.cb.onTrackChange?.(peer.id, peer.stream);
    };

    pc.onicecandidate = ev => {
      if (peer.pc !== pc || !ev.candidate || !ev.candidate.candidate) return;
      const cleaned = filterIceCandidate(ev.candidate.candidate);
      if (!cleaned || peer.sentCandidates >= 64) return;
      if (!peer.negotiated || !peer.token) {
        peer.pendingLocalCandidates.push(cleaned);
        return;
      }
      this.#trickleCandidates(peer, [cleaned]);
    };

    pc.oniceconnectionstatechange = () => {
      if (peer.pc !== pc) return;
      peer.iceState = pc.iceConnectionState;
      this.cb.onPeerState?.(peer.id, peer.iceState);

      if (peer.iceState === "connected" || peer.iceState === "completed") {
        clearTimeout(peer.disconnectTimer);
        peer.disconnectTimer = null;
      } else if (peer.iceState === "disconnected") {
        clearTimeout(peer.disconnectTimer);
        peer.disconnectTimer = setTimeout(() => {
          if (peer.pc === pc && pc.iceConnectionState === "disconnected") {
            this.#requestRecovery(peer);
          }
        }, 6000);
      } else if (peer.iceState === "failed") {
        this.#requestRecovery(peer);
      }
    };

    return pc;
  }

  #attachDataChannel(peer, channel) {
    peer.channel = channel;
    channel.binaryType = "arraybuffer";
    channel.onopen = () => {
      if (peer.channel !== channel) return;
      try {
        channel.send(new Uint8Array([1, peer.watching ? 1 : 0]));
        channel.send(new Uint8Array([6]));
      } catch {
        // Ignore transient close.
      }
    };
    channel.onmessage = async ev => {
      if (peer.channel !== channel || !(ev.data instanceof ArrayBuffer)) return;
      let parsed;
      try {
        parsed = parseControlMessage(new Uint8Array(ev.data));
      } catch {
        return;
      }
      if (!parsed) return;
      await this.#handleControl(peer, parsed);
    };
  }

  async #handleControl(peer, msg) {
    switch (msg.type) {
      case "preview":
        this.cb.onPreview?.(peer.id, msg.jpeg);
        break;
      case "streamInfo":
        this.cb.onStreamInfo?.(peer.id, msg.info);
        break;
      case "viewers":
        this.cb.onViewers?.(peer.id, msg.viewers);
        break;
      case "avatarHash":
        peer.announcedAvatarHash = msg.hash;
        if (!msg.hash) {
          this.cb.onAvatar?.(peer.id, null);
        } else if (this.avatarCache.has(msg.hash)) {
          this.cb.onAvatar?.(peer.id, this.avatarCache.get(msg.hash));
        } else if (peer.channel?.readyState === "open") {
          const req = textEncoder.encode(msg.hash);
          const pkt = new Uint8Array(1 + req.length);
          pkt[0] = 7;
          pkt.set(req, 1);
          peer.channel.send(pkt);
        }
        break;
      case "avatarData": {
        const digest = await sha256Hex(msg.bytes);
        if (peer.announcedAvatarHash && digest === peer.announcedAvatarHash) {
          const copy = new Uint8Array(msg.bytes);
          this.avatarCache.set(digest, copy);
          this.cb.onAvatar?.(peer.id, copy);
        }
        break;
      }
      default:
        break;
    }
  }

  async #startOffer(peer) {
    const pc = this.#createPeerConnection(peer);
    pc.addTransceiver("video", { direction: "recvonly" });
    pc.addTransceiver("audio", { direction: "recvonly" });
    const channel = pc.createDataChannel("p2pshare-preview-v1");
    this.#attachDataChannel(peer, channel);

    const token = randomAttemptToken();
    peer.token = token;
    peer.negotiated = false;

    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    if (peer.pc !== pc) return;

    peer.negotiated = true;
    const payload = JSON.stringify({
      type: "offer",
      sdp: sanitizeSdp(pc.localDescription?.sdp || offer.sdp),
      p2pshare_attempt: token,
    });
    this.#sendJson({
      type: "signal",
      to: peer.id,
      to_session: peer.session,
      kind: "offer",
      payload,
    });
    if (peer.pendingLocalCandidates.length) {
      const queued = peer.pendingLocalCandidates.splice(0, 32);
      this.#trickleCandidates(peer, queued);
    }
  }

  async #handleSignal(msg) {
    const peer = this.peers.get(msg.from);
    if (!peer || msg.to_session !== this.ownSession || msg.from_session !== peer.session) return;

    let payload;
    try {
      payload = JSON.parse(msg.payload);
    } catch {
      return;
    }
    const token = typeof payload.p2pshare_attempt === "string" ? payload.p2pshare_attempt : null;

    if (msg.kind === "offer" && this.localId > peer.id) {
      if (token && peer.negotiated && peer.token === token) return;
      const pc = this.#createPeerConnection(peer);
      peer.token = token;
      pc.ondatachannel = ev => {
        if (ev.channel.label === "p2pshare-preview-v1") {
          this.#attachDataChannel(peer, ev.channel);
        }
      };
      await pc.setRemoteDescription({ type: "offer", sdp: payload.sdp });
      if (peer.pc !== pc) return;
      peer.remoteDescSet = true;
      forceRecvOnly(pc);

      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      if (peer.pc !== pc) return;
      peer.negotiated = true;

      this.#sendJson({
        type: "signal",
        to: peer.id,
        to_session: peer.session,
        kind: "answer",
        payload: JSON.stringify({
          type: "answer",
          sdp: sanitizeSdp(pc.localDescription?.sdp || answer.sdp),
          ...(token ? { p2pshare_attempt: token } : {}),
        }),
      });
      await this.#flushRemoteCandidates(peer);
      if (peer.pendingLocalCandidates.length) {
        const queued = peer.pendingLocalCandidates.splice(0, 32);
        this.#trickleCandidates(peer, queued);
      }
    } else if (msg.kind === "answer" && this.localId < peer.id) {
      if (!peer.pc || peer.remoteDescSet) return;
      if (token !== peer.token) return;
      await peer.pc.setRemoteDescription({ type: "answer", sdp: payload.sdp });
      peer.remoteDescSet = true;
      await this.#flushRemoteCandidates(peer);
    } else if (msg.kind === "ice" && Array.isArray(payload.candidates)) {
      if (!peer.negotiated || (token ?? null) !== (peer.token ?? null)) return;
      for (const cand of payload.candidates) {
        if (typeof cand !== "string") continue;
        const normalized = cand.startsWith("a=") ? cand.slice(2) : cand;
        if (peer.remoteDescSet && peer.pc) {
          try {
            await peer.pc.addIceCandidate({ candidate: normalized, sdpMLineIndex: 0 });
          } catch {
            // Ignore late/unusable candidates from remote interfaces.
          }
        } else {
          peer.pendingRemoteCandidates.push(normalized);
        }
      }
    } else if (msg.kind === "ice" && payload.restart === true && this.localId < peer.id) {
      this.#requestRecovery(peer);
    }
  }

  async #flushRemoteCandidates(peer) {
    if (!peer.pc || !peer.remoteDescSet) return;
    const list = peer.pendingRemoteCandidates.splice(0);
    for (const normalized of list) {
      try {
        await peer.pc.addIceCandidate({ candidate: normalized, sdpMLineIndex: 0 });
      } catch {
        // Ignore invalid remote candidates.
      }
    }
  }

  #trickleCandidates(peer, candidates) {
    const batch = candidates.slice(0, 32);
    if (!batch.length || peer.sentCandidates >= 64) return;
    peer.sentCandidates += batch.length;
    this.#sendJson({
      type: "signal",
      to: peer.id,
      to_session: peer.session,
      kind: "ice",
      payload: JSON.stringify({
        candidates: batch,
        ...(peer.token ? { p2pshare_attempt: peer.token } : {}),
      }),
    });
  }

  #requestRecovery(peer) {
    clearTimeout(peer.retryTimer);
    peer.retryTimer = setTimeout(() => {
      if (this.disposed || !this.inRoom || !this.peers.has(peer.id)) return;
      if (this.localId < peer.id) {
        this.#startOffer(peer);
      } else {
        this.#sendJson({
          type: "signal",
          to: peer.id,
          to_session: peer.session,
          kind: "ice",
          payload: JSON.stringify({ restart: true }),
        });
      }
    }, 1500);
  }

  #destroyPeer(peerId, peer) {
    clearTimeout(peer.retryTimer);
    clearTimeout(peer.disconnectTimer);
    try {
      peer.channel?.close();
    } catch {
      // Ignore.
    }
    try {
      peer.pc?.close();
    } catch {
      // Ignore.
    }
    this.cb.onPeerRemoved?.(peerId);
  }

  #resetAllPeers() {
    for (const [peerId, peer] of this.peers.entries()) {
      this.#destroyPeer(peerId, peer);
    }
    this.peers.clear();
  }
}
