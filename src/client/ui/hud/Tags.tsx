import { useEffect, useRef } from 'preact/hooks';
import type { Game } from '../../game/game';
import { colorBg } from '../labels';

/**
 * Name tags over the beans as HTML, positioned every frame. Drawn outside the 3D scene, they stay
 * crisp and are untouched by fog, tone mapping and post-processing.
 */
export function Tags({ game }: { game: Game }) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const els = new Map<number, HTMLDivElement>();
    const pos = new Map<number, { x: number; y: number; d: number; name: string; color: string }>();
    let raf = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const box = root.current;
      if (!box) return;
      game.tagPositions(pos);
      for (const [id, p] of pos) {
        let el = els.get(id);
        if (!el) {
          el = document.createElement('div');
          el.className = 'tag';
          el.innerHTML = '<i></i><span></span>';
          box.appendChild(el);
          els.set(id, el);
        }
        const dot = el.firstChild as HTMLElement;
        const label = el.lastChild as HTMLElement;
        if (label.textContent !== p.name) label.textContent = p.name;
        if (el.dataset.color !== p.color) {
          el.dataset.color = p.color;
          dot.style.background = colorBg(p.color);
        }
        const s = Math.max(0.55, Math.min(1, 14 / p.d));
        el.style.transform = `translate(${p.x.toFixed(1)}px, ${p.y.toFixed(1)}px) translate(-50%, -100%) scale(${s.toFixed(3)})`;
        el.style.opacity = p.d > 60 ? '0' : p.d > 40 ? String((60 - p.d) / 20) : '1';
        el.style.zIndex = String(1000 - Math.round(p.d));
        el.hidden = false;
      }
      for (const [id, el] of els) {
        if (pos.has(id)) continue;
        el.hidden = true;
        if (!game.arena) {
          el.remove();
          els.delete(id);
        }
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [game]);
  return <div class="tags" ref={root} />;
}
