import { files } from "./embedded.js";

const args = process.argv.slice(2);
const argVal = (k, d) => { const i = args.indexOf(k); return i >= 0 && args[i + 1] ? args[i + 1] : d; };
const PORT = Number(argVal("--port", process.env.PORT || 7777));
const SOLO = args.includes("--solo") || process.env.SOLO === "1";

const MIN_PLAYERS = SOLO ? 1 : 2;
const MAX_PLAYERS = 5;
const COLORS = ["#ff5fa2", "#3fa9ff", "#ffd23f", "#4fdc6a", "#a66bff", "#ff8a3d", "#39e0d0", "#ffffff"];
const NON_FINAL_MAPS = ["race1", "jumpclub", "race2"];
const MAP_INFO = {
  race1: { kind: "race", duration: 180 },
  race2: { kind: "race", duration: 180 },
  jumpclub: { kind: "survival", duration: 75 },
  hex: { kind: "final", duration: 400 },
};

const MIME = {
  html: "text/html; charset=utf-8", js: "text/javascript; charset=utf-8", css: "text/css",
  glb: "model/gltf-binary", png: "image/png", svg: "image/svg+xml", json: "application/json", ico: "image/x-icon",
};

const players = new Map();
let nextId = 1;
let hostId = null;
let phase = "lobby";
let show = null;
let round = null;
let nextTimer = null;

const now = () => Date.now();
const shuffle = (a) => { for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; } return a; };

function send(p, msg) { try { p.ws.send(JSON.stringify(msg)); } catch {} }
function broadcast(msg) { const s = JSON.stringify(msg); for (const p of players.values()) { try { p.ws.send(s); } catch {} } }

function lobbyPayload() {
  return {
    t: "lobby", phase, host: hostId, min: MIN_PLAYERS, max: MAX_PLAYERS,
    players: [...players.values()].map((p) => ({
      id: p.id, name: p.name, color: p.color, score: p.score, crowns: p.crowns,
      alive: show ? show.alive.has(p.id) : true, spectator: p.spectator,
    })),
  };
}
const sendLobby = () => broadcast(lobbyPayload());

function freeColor() {
  const used = new Set([...players.values()].map((p) => p.color));
  return COLORS.find((c) => !used.has(c)) || COLORS[0];
}

function activeIds() {
  if (phase === "lobby" || phase === "winner") return [...players.keys()];
  if (!round) return [];
  return round.participants.filter((id) => players.has(id) && !round.finished.includes(id) && !round.out.includes(id));
}

setInterval(() => {
  const l = [];
  for (const id of activeIds()) {
    const p = players.get(id);
    if (p && p.state) l.push([id, ...p.state.p, p.state.r, p.state.a]);
  }
  if (l.length) broadcast({ t: "S", s: now(), l });
}, 50);

function startShow() {
  const ids = [...players.keys()].slice(0, MAX_PLAYERS);
  for (const p of players.values()) { p.score = 0; p.spectator = !ids.includes(p.id); }
  const nonFinal = Math.max(2, ids.length - 2);
  const pool = shuffle([...NON_FINAL_MAPS]);
  const plan = [];
  for (let i = 0; i < nonFinal; i++) plan.push(pool[i % pool.length]);
  plan.push("hex");
  show = { plan, index: 0, alive: new Set(ids), started: ids.length };
  nextRound();
}

function nextRound() {
  clearTimeout(nextTimer);
  const alive = [...show.alive].filter((id) => players.has(id));
  if (alive.length === 0) return backToLobby();
  if (alive.length === 1 && show.started > 1) return declareWinner(alive[0]);
  const map = show.plan[show.index];
  if (!map) return declareWinner(alive[0]);
  const info = MAP_INFO[map];
  const remainingNonFinal = show.plan.length - 1 - show.index;
  let eliminate = 0;
  if (info.kind !== "final") {
    const need = alive.length - 2;
    if (need > 0 && need >= remainingNonFinal) eliminate = Math.ceil(need / remainingNonFinal);
  }
  show.index++;
  phase = "round";
  for (const p of players.values()) p.state = null;
  round = {
    map, kind: info.kind, eliminate, qualify: alive.length - eliminate,
    participants: shuffle([...alive]), finished: [], out: [], tiles: new Map(), doors: new Set(),
    seed: Math.floor(Math.random() * 1e9), startAt: now() + 7000, endAt: 0, over: false,
  };
  round.endAt = round.startAt + info.duration * 1000;
  broadcast({
    t: "round", map, kind: round.kind, eliminate, qualify: round.qualify, participants: round.participants,
    startAt: round.startAt, endAt: round.endAt, seed: round.seed, index: show.index, total: show.plan.length,
  });
  sendLobby();
}

function connectedParticipants() { return round.participants.filter((id) => players.has(id)); }

function checkRound() {
  if (!round || round.over || phase !== "round") return;
  const t = now();
  const parts = connectedParticipants();
  const remaining = parts.filter((id) => !round.finished.includes(id) && !round.out.includes(id));
  if (round.kind === "race") {
    const doneCount = round.finished.filter((id) => players.has(id)).length;
    if (round.eliminate > 0 && doneCount >= Math.min(round.qualify, parts.length)) return endRound();
    if (remaining.length === 0 || t > round.endAt) return endRound();
  } else if (round.kind === "survival") {
    if (round.eliminate > 0 && round.out.length >= round.eliminate) return endRound();
    if (remaining.length === 0 || t > round.endAt) return endRound();
  } else {
    if (show.started > 1 && remaining.length <= 1) return endRound();
    if (remaining.length === 0 || t > round.endAt) return endRound();
  }
}
setInterval(checkRound, 200);

function endRound() {
  round.over = true;
  const parts = connectedParticipants();
  let qualified = [], eliminated = [];
  if (round.kind === "race") {
    const fin = round.finished.filter((id) => players.has(id));
    fin.forEach((id, i) => { players.get(id).score += Math.max(1, 6 - i); });
    if (round.eliminate === 0) {
      qualified = parts;
    } else {
      const rest = parts.filter((id) => !fin.includes(id))
        .sort((a, b) => (players.get(b).best || -1e9) - (players.get(a).best || -1e9));
      qualified = [...fin, ...rest].slice(0, round.qualify);
      eliminated = parts.filter((id) => !qualified.includes(id));
    }
  } else if (round.kind === "survival") {
    const outs = round.out.filter((id) => players.has(id));
    eliminated = round.eliminate > 0 ? outs.slice(0, round.eliminate) : [];
    qualified = parts.filter((id) => !eliminated.includes(id));
    for (const id of parts) if (!outs.includes(id)) players.get(id).score += 3;
  } else {
    const remaining = parts.filter((id) => !round.out.includes(id));
    const winner = remaining[0] ?? round.out[round.out.length - 1];
    return declareWinner(winner);
  }
  for (const id of eliminated) show.alive.delete(id);
  phase = "results";
  broadcast({ t: "roundEnd", qualified, eliminated, map: round.map });
  sendLobby();
  nextTimer = setTimeout(nextRound, 6500);
}

function declareWinner(id) {
  clearTimeout(nextTimer);
  if (round) round.over = true;
  phase = "winner";
  const p = players.get(id);
  if (p) p.crowns++;
  broadcast({ t: "winner", id, name: p?.name || "???" });
  sendLobby();
  nextTimer = setTimeout(backToLobby, 11000);
}

function backToLobby() {
  clearTimeout(nextTimer);
  phase = "lobby"; show = null; round = null;
  for (const p of players.values()) { p.spectator = false; p.best = 0; p.state = null; }
  sendLobby();
}

function onMessage(p, m) {
  switch (m.t) {
    case "ping": return send(p, { t: "pong", c: m.c, s: now() });
    case "s":
      if (Array.isArray(m.p) && m.p.length === 3) {
        p.state = { p: m.p.map((v) => Math.round(v * 100) / 100), r: Math.round(m.r * 100) / 100, a: m.a | 0 };
        if (round && round.kind === "race") p.best = Math.max(p.best || -1e9, m.p[2]);
      }
      return;
    case "name":
      p.name = String(m.name || "").slice(0, 16) || p.name;
      return sendLobby();
    case "color":
      if (phase === "lobby" && COLORS.includes(m.c) && ![...players.values()].some((o) => o !== p && o.color === m.c)) {
        p.color = m.c; sendLobby();
      }
      return;
    case "start":
      if (p.id === hostId && phase === "lobby" && players.size >= MIN_PLAYERS) startShow();
      return;
    case "finish":
      if (round && !round.over && round.kind === "race" && round.participants.includes(p.id) && !round.finished.includes(p.id) && now() >= round.startAt) {
        round.finished.push(p.id);
        broadcast({ t: "fin", id: p.id, place: round.finished.length });
        checkRound();
      }
      return;
    case "out":
      if (round && !round.over && round.kind !== "race" && round.participants.includes(p.id) && !round.out.includes(p.id)) {
        round.out.push(p.id);
        broadcast({ t: "out", id: p.id });
        checkRound();
      }
      return;
    case "tile":
      if (round && round.map === "hex" && Number.isInteger(m.i) && !round.tiles.has(m.i)) {
        const at = now() + 450;
        round.tiles.set(m.i, at);
        broadcast({ t: "tile", i: m.i, at });
      }
      return;
    case "door":
      if (round && Number.isInteger(m.i) && !round.doors.has(m.i)) {
        round.doors.add(m.i);
        broadcast({ t: "door", i: m.i, by: p.id });
      }
      return;
    case "bump":
    case "grab": {
      const o = players.get(m.to);
      if (o && Array.isArray(m.v || [0, 0, 0])) send(o, { t: m.t, from: p.id, v: m.v });
      return;
    }
    case "emote":
      return broadcast({ t: "emote", id: p.id, e: m.e | 0 });
  }
}

const server = Bun.serve({
  port: PORT,
  hostname: "0.0.0.0",
  fetch(req, srv) {
    const url = new URL(req.url);
    if (url.pathname === "/ws") {
      if (srv.upgrade(req)) return;
      return new Response("upgrade failed", { status: 400 });
    }
    const path = url.pathname === "/" ? "/index.html" : decodeURIComponent(url.pathname);
    const f = files[path];
    if (!f) return new Response("not found", { status: 404 });
    const ext = path.split(".").pop();
    return new Response(Bun.file(f), { headers: { "Content-Type": MIME[ext] || "application/octet-stream", "Cache-Control": "no-cache" } });
  },
  websocket: {
    idleTimeout: 60,
    open(ws) {
      if (players.size >= MAX_PLAYERS) {
        ws.send(JSON.stringify({ t: "full", max: MAX_PLAYERS }));
        ws.close();
        return;
      }
      const p = { id: nextId++, ws, name: `Боб ${nextId - 1}`, color: freeColor(), score: 0, crowns: 0, state: null, spectator: phase !== "lobby", best: 0 };
      ws.data = p;
      players.set(p.id, p);
      if (hostId === null || !players.has(hostId)) hostId = p.id;
      send(p, { t: "welcome", id: p.id, solo: SOLO });
      sendLobby();
      if (round && phase !== "lobby") {
        send(p, {
          t: "round", map: round.map, kind: round.kind, eliminate: round.eliminate, qualify: round.qualify,
          participants: round.participants, startAt: round.startAt, endAt: round.endAt, seed: round.seed,
          index: show.index, total: show.plan.length, late: true,
          tiles: [...round.tiles], doors: [...round.doors], finished: round.finished, out: round.out,
        });
      }
    },
    message(ws, data) {
      const p = ws.data;
      if (!p) return;
      let m;
      try { m = JSON.parse(data); } catch { return; }
      if (m && typeof m.t === "string") onMessage(p, m);
    },
    close(ws) {
      const p = ws.data;
      if (!p || !players.has(p.id)) return;
      players.delete(p.id);
      if (hostId === p.id) hostId = players.size ? [...players.keys()][0] : null;
      broadcast({ t: "left", id: p.id });
      if (players.size === 0) { backToLobby(); return; }
      if (show) {
        show.alive.delete(p.id);
        if (phase === "results" && show.alive.size <= 1) nextRound();
      }
      sendLobby();
    },
  },
});

console.log(`Fall Beans: порт ${server.port}, игроков ${MIN_PLAYERS}-${MAX_PLAYERS}${SOLO ? " (--solo)" : ""}`);
