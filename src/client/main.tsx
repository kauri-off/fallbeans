import { render } from 'preact';
import './styles.css';
import { loadModels } from './game/assets';
import { Game } from './game/game';
import { conn, loadProgress, menuOpen } from './state';
import { App } from './ui/App';

const canvas = document.getElementById('scene') as HTMLCanvasElement;
const ui = document.getElementById('ui') as HTMLElement;

async function boot() {
  let game: Game;
  try {
    game = new Game(canvas);
  } catch (e) {
    conn.value = { ...conn.value, status: 'rejected', message: `Не удалось запустить WebGL 2: ${String(e)}` };
    render(<App game={null as unknown as Game} />, ui);
    return;
  }
  render(<App game={game} />, ui);
  // Read-only probe for end-to-end tests and debugging.
  (window as unknown as { __fallbeans: unknown }).__fallbeans = {
    state: () => ({
      id: game.arena?.body?.actor ?? null,
      pos: game.arena?.body?.pos.toArray() ?? null,
      arena: game.arena?.info.game ?? null,
      kind: game.arena?.kind ?? null,
      transport: game.net.kind,
      corrections: game.arena?.corrections ?? 0,
      lead: Math.round(game.arena?.inputLead ?? 0),
      rtt: Math.round(game.net.clock.rtt),
      drawCalls: game.renderer.info.calls,
      input: game.input.enabled,
      menu: menuOpen.value,
    }),
  };
  await loadModels((done, total) => {
    loadProgress.value = done / total;
  });
  conn.value = { ...conn.value, status: 'connecting' };
  game.start();
}

boot().catch((e) => {
  console.error(e);
  conn.value = { ...conn.value, status: 'rejected', message: String(e) };
});
