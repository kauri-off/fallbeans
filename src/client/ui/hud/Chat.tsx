import { useEffect, useRef } from 'preact/hooks';
import { CHAT_MAX } from '../../../shared/consts';
import type { Game } from '../../game/game';
import { chatFresh, chatLog, chatOpen, menuOpen } from '../../state';

/**
 * The room's text chat, bottom left. Normally invisible; a new line shows it half transparent for a
 * few seconds; Enter opens the line to type in (Enter again sends, Esc closes).
 */
export function Chat({ game }: { game: Game }) {
  const open = chatOpen.value;
  const lines = chatLog.value;
  const field = useRef<HTMLInputElement>(null);
  const log = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (open) field.current?.focus();
  }, [open]);
  // Keep the newest line in view (when a line arrives, and when the full log opens).
  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines, open]);
  if (!open && !lines.length) return null;
  const close = () => {
    chatOpen.value = false;
  };
  return (
    <div class={`chat${open ? ' open' : chatFresh.value ? ' fresh' : ''}${menuOpen.value ? ' aside' : ''}`}>
      <div class="chat-log" ref={log}>
        {(open ? lines : lines.slice(-6)).map((l) => (
          <div key={l.n} class={l.mine ? 'me' : ''}>
            <b style={{ color: l.color }}>{l.name}:</b> {l.text}
          </div>
        ))}
      </div>
      {open && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            game.sendChat(field.current?.value ?? '');
            close();
          }}
        >
          <input
            ref={field}
            class="chat-input"
            maxLength={CHAT_MAX}
            autocomplete="off"
            placeholder="Сообщение: Enter — отправить, Esc — закрыть"
            onKeyDown={(e) => {
              if (e.key === 'Escape') close();
            }}
            onBlur={close}
          />
        </form>
      )}
    </div>
  );
}
