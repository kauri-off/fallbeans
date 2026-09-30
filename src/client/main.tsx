// Imported for its side effect too: capture.ts starts catching errors as soon as it loads.

import { render } from 'preact';
import { report, setReportContext } from './debug/capture';
import './styles.css';
import { createProbe } from './debug/probe';
import { loadModels } from './game/assets';
import { Game } from './game/game';
import { settings } from './settings';
import { arenaInfo, conn, loadProgress, lobby, myId } from './state';
import { App } from './ui/App';

const canvas = document.getElementById('scene') as HTMLCanvasElement;
const ui = document.getElementById('ui') as HTMLElement;

async function boot() {
  let game: Game;
  try {
    game = new Game(canvas);
  } catch (e) {
    report('boot', `WebGL 2 unavailable: ${String(e)}`);
    conn.value = { ...conn.value, status: 'rejected', message: `Не удалось запустить WebGL 2: ${String(e)}` };
    render(<App game={null as unknown as Game} />, ui);
    return;
  }
  setReportContext(() => ({
    me: myId.value,
    phase: lobby.value?.phase,
    arena: arenaInfo.value?.game,
    kind: arenaInfo.value?.kind,
    status: conn.value.status,
    transport: game.net.kind,
    rtt: game.net.clock.rtt,
    quality: settings.value.quality,
    size: `${innerWidth}x${innerHeight}@${devicePixelRatio}`,
  }));
  render(<App game={game} />, ui);
  // Debug probe (window.__fallbeans) for tests, automation and the console.
  window.__fallbeans = createProbe(game);
  await loadModels((done, total) => {
    loadProgress.value = done / total;
  });
  conn.value = { ...conn.value, status: 'connecting' };
  game.start();
}

boot().catch((e) => {
  console.error(e);
  report('boot', String(e), e instanceof Error ? e.stack : undefined);
  conn.value = { ...conn.value, status: 'rejected', message: String(e) };
});
