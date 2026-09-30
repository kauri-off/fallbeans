import type { ComponentChildren } from 'preact';

/** The controls line at the bottom of the screen: keyboard and mouse side by side. */
export function Keys({ grab, lead, chat = true }: { grab: string; lead?: ComponentChildren; chat?: boolean }) {
  return (
    <div class="keys">
      {lead}
      <kbd>WASD</kbd> бег · <kbd>Мышь</kbd> камера · <kbd>Пробел</kbd> прыжок · <kbd>E</kbd>/<kbd>ЛКМ</kbd> нырок · <kbd>Q</kbd>/
      <kbd>ПКМ</kbd> {grab} · <kbd>1</kbd>–<kbd>5</kbd> эмоции
      {chat && (
        <>
          {' '}
          · <kbd>Enter</kbd> чат
        </>
      )}
    </div>
  );
}
