import { bytesToHex, hexToBytes } from "./ed25519.js";
import { canonicalRoom, parseInvite, validateServer } from "./protocol.js";
import { RoomClient } from "./rtc.js";

const STORAGE_KEY = "p2pshare.web.v1";

const els = {
  sidebar: document.getElementById("sidebar"),
  sidebarScrim: document.getElementById("sidebar-scrim"),
  menuBtn: document.getElementById("menu-btn"),
  membersHeading: document.getElementById("members-heading"),
  memberList: document.getElementById("member-list"),
  roomChipName: document.getElementById("room-chip-name"),
  roomEditBtn: document.getElementById("room-edit-btn"),
  localAvatar: document.getElementById("local-avatar"),
  localName: document.getElementById("local-name"),
  settingsBtn: document.getElementById("settings-btn"),
  mobileSettingsBtn: document.getElementById("mobile-settings-btn"),
  mobileRoomTitle: document.getElementById("mobile-room-title"),
  mobileDot: document.getElementById("mobile-dot"),
  emptyView: document.getElementById("empty-view"),
  emptyTitle: document.getElementById("empty-title"),
  emptyDesc: document.getElementById("empty-desc"),
  emptyActionBtn: document.getElementById("empty-action-btn"),
  gridView: document.getElementById("grid-view"),
  gridHeading: document.getElementById("grid-heading"),
  streamGrid: document.getElementById("stream-grid"),
  playerView: document.getElementById("player-view"),
  videoStage: document.getElementById("video-stage"),
  player: document.getElementById("player"),
  videoWaiting: document.getElementById("video-waiting"),
  videoWaitingText: document.getElementById("video-waiting-text"),
  unmuteOverlayBtn: document.getElementById("unmute-overlay-btn"),
  watchingAvatar: document.getElementById("watching-avatar"),
  watchingName: document.getElementById("watching-name"),
  watchingStats: document.getElementById("watching-stats"),
  muteStreamBtn: document.getElementById("mute-stream-btn"),
  volumeSlider: document.getElementById("volume-slider"),
  fullscreenBtn: document.getElementById("fullscreen-btn"),
  stopWatchBtn: document.getElementById("stop-watch-btn"),
  allScreensBtn: document.getElementById("all-screens-btn"),
  muteAllBtn: document.getElementById("mute-all-btn"),
  leaveBtn: document.getElementById("leave-btn"),
  statusDot: document.getElementById("status-dot"),
  statusText: document.getElementById("status-text"),
  setupModal: document.getElementById("setup-modal"),
  setupForm: document.getElementById("setup-form"),
  modalCloseBtn: document.getElementById("modal-close-btn"),
  formError: document.getElementById("form-error"),
  serverInput: document.getElementById("server-input"),
  nameInput: document.getElementById("name-input"),
  roomInput: document.getElementById("room-input"),
  recentRooms: document.getElementById("recent-rooms"),
  clearCacheBtn: document.getElementById("clear-cache-btn"),
};

function createSeedHex() {
  const seed = new Uint8Array(32);
  crypto.getRandomValues(seed);
  return bytesToHex(seed);
}

function loadSettings() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : {};
    return {
      server: typeof parsed.server === "string" ? parsed.server : "",
      room: typeof parsed.room === "string" ? parsed.room : "",
      name: typeof parsed.name === "string" && parsed.name.trim() ? parsed.name.trim() : "iPhone Viewer",
      recentRooms: Array.isArray(parsed.recentRooms) ? parsed.recentRooms.slice(0, 6) : [],
      seedHex: typeof parsed.seedHex === "string" && /^[a-f0-9]{64}$/.test(parsed.seedHex) ? parsed.seedHex : createSeedHex(),
      mutedAll: Boolean(parsed.mutedAll),
      volume: typeof parsed.volume === "number" ? Math.min(100, Math.max(0, parsed.volume)) : 100,
    };
  } catch {
    return {
      server: "",
      room: "",
      name: "iPhone Viewer",
      recentRooms: [],
      seedHex: createSeedHex(),
      mutedAll: false,
      volume: 100,
    };
  }
}

function saveSettings() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state.settings));
  } catch {
    // Ignore storage quota errors in private browsing.
  }
}

const state = {
  settings: loadSettings(),
  members: [],
  watchingPeerId: null,
  streamMuted: false,
  previews: new Map(),
  avatars: new Map(),
  streamInfo: new Map(),
  viewers: new Map(),
  peerIce: new Map(),
  statusText: "Not connected",
  statusKind: "muted",
};
saveSettings();

const client = new RoomClient({
  onStatus(text, kind) {
    state.statusText = text;
    state.statusKind = kind;
    renderStatus();
  },
  onMembers(members) {
    state.members = members;
    if (state.watchingPeerId) {
      const target = members.find(m => m.id === state.watchingPeerId);
      if (!target || !target.sharing) {
        stopWatching(false);
      }
    }
    render();
  },
  onPreview(peerId, jpegBytes) {
    const prev = state.previews.get(peerId);
    if (prev) URL.revokeObjectURL(prev);
    const url = URL.createObjectURL(new Blob([jpegBytes], { type: "image/jpeg" }));
    state.previews.set(peerId, url);
    renderStage();
  },
  onAvatar(peerId, bytes) {
    const prev = state.avatars.get(peerId);
    if (prev) URL.revokeObjectURL(prev);
    if (!bytes) {
      state.avatars.delete(peerId);
    } else {
      const url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
      state.avatars.set(peerId, url);
    }
    render();
  },
  onStreamInfo(peerId, info) {
    state.streamInfo.set(peerId, info);
    renderStage();
  },
  onViewers(peerId, list) {
    state.viewers.set(peerId, list);
    renderStage();
  },
  onPeerState(peerId, iceState) {
    state.peerIce.set(peerId, iceState);
    if (peerId === state.watchingPeerId) renderPlayerOverlay();
  },
  onTrackChange(peerId, stream) {
    if (peerId === state.watchingPeerId) {
      attachMediaStream(stream);
    }
  },
  onPeerRemoved(peerId) {
    const previewUrl = state.previews.get(peerId);
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    state.previews.delete(peerId);
    const avatarUrl = state.avatars.get(peerId);
    if (avatarUrl) URL.revokeObjectURL(avatarUrl);
    state.avatars.delete(peerId);
    state.streamInfo.delete(peerId);
    state.viewers.delete(peerId);
    state.peerIce.delete(peerId);
  },
});

function initialOf(name) {
  const trimmed = (name || "?").trim();
  return trimmed ? trimmed[0].toUpperCase() : "?";
}

function formatStats(member) {
  const info = state.streamInfo.get(member.id);
  const res = member.width && member.height ? `${member.width}×${member.height}` : "Live";
  const codec = (info?.codec || member.codec || "h264").toUpperCase();
  const bitrate = info?.bitrate_kbps ? ` · ${(info.bitrate_kbps / 1000).toFixed(info.bitrate_kbps >= 1000 ? 1 : 2)} Mbps` : "";
  const count = state.viewers.get(member.id)?.length;
  const viewers = typeof count === "number" ? ` · ${count} watching` : "";
  return `${res} · ${codec}${bitrate}${viewers}`;
}

function setSidebarOpen(open) {
  els.sidebar.classList.toggle("open", open);
  els.sidebarScrim.classList.toggle("open", open);
}

function renderStatus() {
  els.statusText.textContent = state.statusText;
  els.statusDot.className = `dot ${state.statusKind}`;
  els.mobileDot.className = `dot ${state.statusKind}`;
}

function renderProfileAndRoom() {
  const roomDisplay = state.settings.room ? `# ${state.settings.room}` : "# —";
  els.roomChipName.textContent = roomDisplay;
  els.mobileRoomTitle.textContent = state.settings.room ? `# ${state.settings.room}` : "P2PShare";
  els.localName.textContent = state.settings.name || "Viewer";
  els.localAvatar.textContent = initialOf(state.settings.name);
  els.muteAllBtn.textContent = state.settings.mutedAll ? "Unmute all" : "Mute all";
  els.volumeSlider.value = String(state.settings.volume);
}

function renderMembers() {
  const sorted = [...state.members].sort((a, b) => {
    if (a.sharing !== b.sharing) return a.sharing ? -1 : 1;
    return (a.name || "").localeCompare(b.name || "");
  });
  els.membersHeading.textContent = `Members / ${sorted.length}`;
  els.memberList.innerHTML = "";

  for (const m of sorted) {
    const btn = document.createElement("button");
    btn.type = "button";
    const isMe = m.id === client.localId;
    const isWatching = m.id === state.watchingPeerId;
    btn.className = `member-row${isWatching ? " active" : ""}${!m.sharing ? " static" : ""}`;

    const avatar = document.createElement("div");
    avatar.className = "avatar";
    const avatarUrl = state.avatars.get(m.id);
    if (avatarUrl) {
      const img = document.createElement("img");
      img.src = avatarUrl;
      img.alt = "";
      avatar.appendChild(img);
    } else {
      avatar.textContent = initialOf(m.name);
    }

    const meta = document.createElement("div");
    meta.className = "member-meta";
    const nameEl = document.createElement("div");
    nameEl.className = "member-name";
    nameEl.textContent = isMe ? `${m.name} (you)` : m.name || "Guest";
    const subEl = document.createElement("div");
    subEl.className = `member-sub${m.sharing ? " live" : ""}`;
    subEl.textContent = m.sharing ? (isWatching ? "Watching" : "Streaming") : "In room";
    meta.append(nameEl, subEl);
    btn.append(avatar, meta);

    if (m.sharing && !isMe) {
      btn.addEventListener("click", () => {
        setSidebarOpen(false);
        startWatching(m.id);
      });
    }
    els.memberList.appendChild(btn);
  }
}

function renderPlayerOverlay() {
  if (!state.watchingPeerId) return;
  const stream = client.getStream(state.watchingPeerId);
  const hasVideoTrack = Boolean(stream && stream.getVideoTracks().length > 0);
  const ice = state.peerIce.get(state.watchingPeerId) || client.getPeerState(state.watchingPeerId);
  if (hasVideoTrack && (ice === "connected" || ice === "completed")) {
    els.videoWaiting.classList.add("hidden");
  } else {
    els.videoWaiting.classList.remove("hidden");
    els.videoWaitingText.textContent =
      ice === "failed"
        ? "Direct P2P connection failed (NAT/firewall may block UDP). Retrying…"
        : ice === "connected" || ice === "completed"
          ? "Waiting for video keyframe…"
          : `Establishing direct P2P stream (${ice})…`;
  }
}

function renderStage() {
  const sharers = state.members.filter(m => m.sharing && m.id !== client.localId);
  const active = state.watchingPeerId
    ? state.members.find(m => m.id === state.watchingPeerId && m.sharing)
    : null;

  if (active) {
    els.emptyView.classList.add("hidden");
    els.gridView.classList.add("hidden");
    els.playerView.classList.remove("hidden");

    const avatarUrl = state.avatars.get(active.id);
    els.watchingAvatar.innerHTML = "";
    if (avatarUrl) {
      const img = document.createElement("img");
      img.src = avatarUrl;
      img.alt = "";
      els.watchingAvatar.appendChild(img);
    } else {
      els.watchingAvatar.textContent = initialOf(active.name);
    }
    els.watchingName.textContent = active.name || "Streamer";
    els.watchingStats.textContent = formatStats(active);
    const effectivelyMuted = state.settings.mutedAll || state.streamMuted;
    els.muteStreamBtn.textContent = effectivelyMuted ? "Unmute" : "Mute";
    els.player.muted = effectivelyMuted;
    els.player.volume = state.settings.volume / 100;
    renderPlayerOverlay();
    return;
  }

  els.playerView.classList.add("hidden");
  if (!sharers.length) {
    els.gridView.classList.add("hidden");
    els.emptyView.classList.remove("hidden");
    if (client.inRoom) {
      els.emptyTitle.textContent = "Nobody is sharing right now";
      els.emptyDesc.textContent = `Connected to # ${state.settings.room}. Waiting for someone to start a screen share.`;
      els.emptyActionBtn.textContent = "Room Settings";
    } else if (!state.settings.server || !state.settings.room) {
      els.emptyTitle.textContent = "Set up your signaling server";
      els.emptyDesc.textContent = "Enter your group's signaling URL and room name once. This browser caches them locally.";
      els.emptyActionBtn.textContent = "Configure Server";
    } else {
      els.emptyTitle.textContent = "Connecting to room…";
      els.emptyDesc.textContent = state.statusText;
      els.emptyActionBtn.textContent = "Room Settings";
    }
    return;
  }

  els.emptyView.classList.add("hidden");
  els.gridView.classList.remove("hidden");
  els.gridHeading.textContent = `Live streams (${sharers.length})`;
  els.streamGrid.innerHTML = "";

  for (const m of sharers) {
    const card = document.createElement("article");
    card.className = "stream-card";

    const thumb = document.createElement("div");
    thumb.className = "stream-thumb";
    thumb.addEventListener("click", () => startWatching(m.id));

    const previewUrl = state.previews.get(m.id);
    if (previewUrl) {
      const img = document.createElement("img");
      img.src = previewUrl;
      img.alt = `${m.name} preview`;
      thumb.appendChild(img);
    } else {
      const ph = document.createElement("div");
      ph.className = "thumb-placeholder";
      ph.textContent = "Preview loading…";
      thumb.appendChild(ph);
    }

    const livePill = document.createElement("span");
    livePill.className = "live-pill";
    livePill.textContent = "LIVE";
    thumb.appendChild(livePill);

    const viewerCount = state.viewers.get(m.id)?.length;
    if (typeof viewerCount === "number" && viewerCount > 0) {
      const vPill = document.createElement("span");
      vPill.className = "viewer-pill";
      vPill.textContent = `${viewerCount} watching`;
      thumb.appendChild(vPill);
    }

    const body = document.createElement("div");
    body.className = "stream-card-body";
    const info = document.createElement("div");
    info.className = "stream-card-info";
    const nameEl = document.createElement("div");
    nameEl.className = "stream-card-name";
    nameEl.textContent = m.name || "Streamer";
    const statsEl = document.createElement("div");
    statsEl.className = "stream-card-stats";
    statsEl.textContent = formatStats(m);
    info.append(nameEl, statsEl);

    const watchBtn = document.createElement("button");
    watchBtn.type = "button";
    watchBtn.className = "primary";
    watchBtn.textContent = "Watch";
    watchBtn.addEventListener("click", () => startWatching(m.id));

    body.append(info, watchBtn);
    card.append(thumb, body);
    els.streamGrid.appendChild(card);
  }
}

function render() {
  renderStatus();
  renderProfileAndRoom();
  renderMembers();
  renderStage();
}

function attachMediaStream(stream) {
  if (!stream) return;
  if (els.player.srcObject !== stream) {
    els.player.srcObject = stream;
  }
  const wantMuted = state.settings.mutedAll || state.streamMuted;
  els.player.muted = wantMuted;
  els.player.volume = state.settings.volume / 100;
  els.unmuteOverlayBtn.classList.add("hidden");

  const playPromise = els.player.play();
  if (playPromise && typeof playPromise.catch === "function") {
    playPromise.catch(() => {
      // iOS Safari blocks unmuted playback if outside user gesture; fall back to muted + prompt.
      if (!els.player.muted) {
        els.player.muted = true;
        els.unmuteOverlayBtn.classList.remove("hidden");
        els.player.play().catch(() => {});
      }
    });
  }
  renderPlayerOverlay();
}

function startWatching(peerId) {
  if (state.watchingPeerId && state.watchingPeerId !== peerId) {
    client.watch(state.watchingPeerId, false);
  }
  state.watchingPeerId = peerId;
  state.streamMuted = false;
  client.watch(peerId, true);
  renderStage();
  attachMediaStream(client.getStream(peerId));
}

function stopWatching(notifyClient = true) {
  if (notifyClient && state.watchingPeerId) {
    client.watch(state.watchingPeerId, false);
  }
  state.watchingPeerId = null;
  els.player.pause();
  els.player.srcObject = null;
  els.unmuteOverlayBtn.classList.add("hidden");
  render();
}

function openSetupModal(errorMessage = "") {
  els.serverInput.value = state.settings.server;
  els.nameInput.value = state.settings.name;
  els.roomInput.value = state.settings.room;
  if (errorMessage) {
    els.formError.textContent = errorMessage;
    els.formError.classList.remove("hidden");
  } else {
    els.formError.classList.add("hidden");
  }
  renderRecentRooms();
  els.setupModal.classList.remove("hidden");
}

function closeSetupModal() {
  els.setupModal.classList.add("hidden");
}

function renderRecentRooms() {
  els.recentRooms.innerHTML = "";
  for (const r of state.settings.recentRooms) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "chip";
    chip.textContent = `# ${r}`;
    chip.addEventListener("click", () => {
      els.roomInput.value = r;
    });
    els.recentRooms.appendChild(chip);
  }
}

async function connectWithSettings() {
  stopWatching(true);
  render();
  await client.connect({
    server: state.settings.server,
    room: state.settings.room,
    name: state.settings.name,
    seed: hexToBytes(state.settings.seedHex),
  });
}

// Event bindings
els.menuBtn.addEventListener("click", () => setSidebarOpen(true));
els.sidebarScrim.addEventListener("click", () => setSidebarOpen(false));
els.settingsBtn.addEventListener("click", () => openSetupModal());
els.mobileSettingsBtn.addEventListener("click", () => openSetupModal());
els.roomEditBtn.addEventListener("click", () => openSetupModal());
els.emptyActionBtn.addEventListener("click", () => openSetupModal());
els.modalCloseBtn.addEventListener("click", () => closeSetupModal());

els.allScreensBtn.addEventListener("click", () => stopWatching(true));
els.stopWatchBtn.addEventListener("click", () => stopWatching(true));

els.muteAllBtn.addEventListener("click", () => {
  state.settings.mutedAll = !state.settings.mutedAll;
  saveSettings();
  renderProfileAndRoom();
  renderStage();
});

els.muteStreamBtn.addEventListener("click", () => {
  if (state.settings.mutedAll) {
    state.settings.mutedAll = false;
    state.streamMuted = false;
  } else {
    state.streamMuted = !state.streamMuted;
  }
  saveSettings();
  render();
});

els.volumeSlider.addEventListener("input", () => {
  state.settings.volume = Number(els.volumeSlider.value);
  saveSettings();
  els.player.volume = state.settings.volume / 100;
});

els.unmuteOverlayBtn.addEventListener("click", () => {
  state.settings.mutedAll = false;
  state.streamMuted = false;
  els.player.muted = false;
  els.player.play().catch(() => {});
  els.unmuteOverlayBtn.classList.add("hidden");
  saveSettings();
  render();
});

els.fullscreenBtn.addEventListener("click", () => {
  if (typeof els.player.webkitEnterFullscreen === "function") {
    els.player.webkitEnterFullscreen();
  } else if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  } else if (typeof els.videoStage.requestFullscreen === "function") {
    els.videoStage.requestFullscreen().catch(() => {});
  }
});

els.leaveBtn.addEventListener("click", () => {
  stopWatching(false);
  client.disconnect();
  render();
  openSetupModal();
});

els.serverInput.addEventListener("input", () => {
  try {
    const invite = parseInvite(els.serverInput.value);
    if (invite) {
      els.serverInput.value = invite.server;
      els.roomInput.value = invite.room;
      els.formError.classList.add("hidden");
    }
  } catch (err) {
    els.formError.textContent = err.message;
    els.formError.classList.remove("hidden");
  }
});

els.clearCacheBtn.addEventListener("click", () => {
  localStorage.removeItem(STORAGE_KEY);
  state.settings = loadSettings();
  saveSettings();
  client.disconnect();
  render();
  openSetupModal();
});

els.setupForm.addEventListener("submit", async ev => {
  ev.preventDefault();
  try {
    const invite = parseInvite(els.serverInput.value);
    const server = invite ? invite.server : validateServer(els.serverInput.value);
    const room = canonicalRoom(invite ? invite.room : els.roomInput.value);
    const name = els.nameInput.value.replace(/[\u0000-\u001f\u007f]/g, "").trim().slice(0, 32) || "Viewer";

    state.settings.server = server;
    state.settings.room = room;
    state.settings.name = name;
    state.settings.recentRooms = [room, ...state.settings.recentRooms.filter(r => r !== room)].slice(0, 6);
    saveSettings();
    closeSetupModal();
    await connectWithSettings();
  } catch (err) {
    els.formError.textContent = err.message || "Invalid configuration";
    els.formError.classList.remove("hidden");
  }
});

render();
if (state.settings.server && state.settings.room) {
  connectWithSettings();
} else {
  openSetupModal();
}
