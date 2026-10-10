# Fall Beans client: functional UI specification

Fall Beans is a party game: up to 8 jelly-bean characters ("beans") compete in races, survival and points rounds on obstacle-course maps. Native client, players run their own servers and add them to a server list. All player-facing text is Russian (keep strings verbatim). Visual styling is deliberately omitted.

## 0. Global model

### 0.1 Screens
| Screen | When | What is on it |
|---|---|---|
| **Servers** (Серверы) | No server selected | Server list and the Settings tab. |
| **Rooms** (Комнаты) | Connected to a server, not in a room | Room list, own-room form, practice tiles, Settings tab. |
| **Room** | In a room (3D game world behind) | HUD, chat, Esc menu, name tags. |
| **Away** | Connecting, reconnecting, refused, outdated | Only the connection banner. |

Screen selection is automatic from connection state.

### 0.2 Overlay layers (bottom to top)
1. Name tags over beans. 2. HUD (standings, feed, timer, intro, countdown, results, summary, status, prompt). 3. Chat (bottom left). 4. Esc menu. 5. Home layer: server list / room list / settings tab. 6. Connection banner. 7. Loading screen.

### 0.3 Persistence
Every setting saves automatically on change; no Save/Apply for settings. The only explicit Save is "Сохранить" next to a name field.

### 0.4 Input
Mouse: everything clickable. Keyboard: Enter in a text field runs its main action (server address → add; name → save; room title → create; PIN → submit; chat → send). Gamepad: Start toggles the Esc menu in a room. Rebinding: next pressed key becomes the binding, Esc cancels; F-keys, digits 1–5, Esc, Enter reserved.

## 1. Startup and loading

### 1.1 Loading screen ("Загрузка…")
At program start and after a graphics change that needs new shaders. Opaque. Shows logo "Fall Beans", spinner, "Загрузка…", progress bar, one step line: "Модели и текстуры" / "Карты: N из M" / "Шейдеры" / "Шейдеры для новых настроек графики". No cancel.

### 1.2 Startup crash notice (top of server list)
- Normal: "В прошлый раз игра упала. Отчёт: {path}"
- GPU reset: "В прошлый раз видеокарта перестала отвечать (сброс видеодрайвера), и игра закрылась. Обновите драйвер; если повторится — выберите в «Графике» другое масштабирование. Отчёт: {path}"
- Button "Открыть папку с логами".

## 2. Server list (tab "Серверы"; tab bar has "Серверы" and "Настройки")
- Update box at top (see 7).
- Heading "Серверы".
- Server cards (last-played first, then in order added). Each: round status icon (lit when online & playable); title (server's advertised name or the address); status badge: "проверяем…" (spinner) / "не отвечает" / "в сети" / "другая версия игры"; detail line: "игроков: N · комнат: M" or "другая версия игры (build)", plus " · address" if title isn't the address; button "Войти" (disabled on version mismatch); button "×" removes (no confirm).
- Empty state: globe icon + "Добавьте сервер: его адрес скажет тот, кто его запустил."
- Section "Добавить сервер": text field placeholder "IP или домен сервера" (max 120), button "Добавить", validation error "Не похоже на адрес: IP, домен или хост:порт.", always-visible hint "Порт нужен, только если сервер слушает HTTP не на 5887: 192.168.1.10:7000."
- Servers are health-checked on open and every 10 s. "Войти" → Away screen with "Подключение…".

## 3. Room list (tab "Комнаты", connected to a server)
- Top bar: button "‹ К серверам", muted server address.
- Alert line (conditional): server's refusal message.
- Caption: "Комнаты" or "Комнаты: N".
- States: loading (spinner, "Загружаем список…"); empty (door icon, "Пока нет ни одной комнаты — создайте свою."); list of room cards: host avatar (initials + colour); title (prefix "🔒 " if private); subline "хост ⭐ {host} · {в лобби | идёт игра}" (+ " · ваша комната"); count "players/max" (+ " +N🤖" for bots); fill meter (red when full); button "Войти" or disabled "Мест нет". Own room highlighted.
- PIN box (when a private room asks): heading "🔒 Комната закрыта PIN-кодом «{room title}»"; field placeholder "PIN-код" (exactly 4 digits); "Войти"; "Отмена"; text "PIN-код спросите у хоста комнаты." or the server's error in red.
- Own room: if player has one → button "Вернуться в свою комнату" + note "Своя комната у каждого одна. Пока вас нет, хостом в ней кто-то из оставшихся; вернётесь — роль снова ваша." Otherwise create form: title field (max 24, placeholder "Комната {name}" or "Название комнаты"), toggle "Приватная: вход по PIN-коду (вы увидите его в меню комнаты)", button "Создать комнату" (enters immediately).
- Practice: foldable "Тренировка одной карты с ботами" with 17 map tiles (title + genre, coloured by genre). Click → practice round vs 3 bots.
- Name: heading "Ваше имя", field (max 16), button "Сохранить".

## 4. Settings (tab on home screens and in Esc menu; identical; all instant + autosaved)

### 4.1 "Управление"
- Slider "Чувствительность мыши" 0.20–3.00 step 0.05 (default 1.00)
- Toggle "Инвертировать мышь по вертикали"
- Slider "Чувствительность стика" 0.20–3.00 (default 1.00)
- Toggle "Инвертировать правый стик по вертикали"
- Toggle "Тряска камеры от ударов и приземлений"

### 4.2 "Экран и звук"
- Slider "Угол обзора" 55–100 (default 70)
- Slider "Громкость звуков" 0–1 step 0.05
- Slider "Размер интерфейса" 0.75–1.50 step 0.05 (applied on release)
- Toggle "Показывать частоту кадров"
- Toggle "Полный экран (F11)"

### 4.3 "Графика" (foldable)
- "Качество графики": chips "Низкое", "Высокое"
- Info: "Видеокарта: {name} · уровень {tier}"
- "Масштабирование (включено всегда)": chips "Авто", "DLSS 4.5", "FSR 3.1", "FSR 1" (unsupported ones disabled)
- Status: "Сейчас: {name}" or "Сейчас: {name} ({chosen} дал сбой, до перезапуска)"
- Note: "DLSS — видеокарты NVIDIA RTX, DLSS и FSR 3.1 — только на Vulkan"
- "Режим масштабирования": "Ультра-качество", "Качество", "Баланс", "Производительность"
- Toggle "Вертикальная синхронизация"
- "Ограничение кадров в секунду": "Нет", "30", "60", "120", "144"
- "Графический API (после перезапуска)": "Авто", "DirectX 12" (Windows only), "Vulkan"

### 4.4 "Клавиши" (foldable)
Rows: "Вперёд" (W, ↑), "Назад" (S, ↓), "Влево" (A, ←), "Вправо" (D, →), "Прыжок" (Space), "Нырок" (E, Shift, Ctrl), "Захват" (Q). Each: keycaps + button "Изменить" → "Нажмите клавишу… (Esc — отмена)". Note: "ЛКМ — тоже нырок, ПКМ — тоже захват; клавиши по месту на клавиатуре, раскладка не важна." Button "Вернуть как было".

### 4.5 "Если что-то сломалось" (foldable)
Note "F8 в игре сохраняет отчёт: что было с сетью и вашим бобом в последние секунды. Нажмите сразу, как заметили странное, и пришлите отчёт с логом из этой папки." Button "Открыть папку с логами".

## 5. Esc menu (in a room)
Opens with Esc / gamepad Start; closes the same way or by clicking the game. Frees the mouse; game takes no input; round is NOT paused (server-run). Opens automatically on entering a room; closes when a round/results/podium begins.

Tabs: "Игра", "Настройки", "Dev" (dev server only).

### 5.1 Game tab — practice
"Тренировка: «{map title}». Раунд повторяется с ботами, пока вы не вернётесь." Button "‹ К списку комнат" or "‹ Вернуться в комнату".

### 5.2 Game tab — in a room
- Room section "Комната": title (🔒 if private); button "Выйти из комнаты" (danger); host-only toggle "Приватная комната (вход по PIN-коду)" → when on "Приватная комната — PIN-код для входа: {PIN}"; non-host note "Приватная комната: PIN-код для друзей знает хост ⭐".
- Bean section "Ваш боб": name field (max 16) + "Сохранить"; colour swatches (lobby only; own marked; taken ones disabled); foldable outfit "Шапка, очки и цвета": 14 hat chips, "Цвет шапки" swatches (first = "as designed"), 6 glasses chips, "Цвет живота", "Цвет ботинок", buttons "🎲 Случайно", "Сбросить".
- Game phase line (not lobby): badge "идёт игра"; text "Идёт раунд {i} из {n}: «{map}»" / "Итоги раунда." / "Игра окончена — награждение."; host-only "Прервать игру" (danger).
- Lobby only:
  - Players: "Игроки: {n} из {max}" + fill meter. Rows: avatar, name (+ " (вы)"), second line "бот" / "{ping} мс" / "нет связи" (red); badges "⭐" host, "👑 N" crowns. Host-only: "×" on bot (hidden while auto-fill on), "Сделать хостом" on other humans.
  - Host setup (non-hosts see spinner + "Игру запускает хост ⭐"): heading "Игра"; mode chips "Микс", "Гонки", "Выживание", "Свой список"; non-custom: "Раундов:" chips 3/5/7; custom: 17 map chips, chosen ones show order "1. Title", up to 12; toggle "Заполнять свободные места ботами"; button "+ Добавить бота" (only when fill off and not full); button "Начать игру" or disabled "Нужно хотя бы N игроков".

### 5.3 Dev tab (dev server only)
"Время:" chips ⏸, ×0.1, ×0.25, ×0.5, ×1, ×2, ×4, "+1 тик", "+0,5 с"; row "Пропустить заставку", "+10 с", "Завершить раунд", "В лобби"; "Телепорт:" "старт", "КТ {i}"…, "финиш"; row "+ бот рядом", "Заморозить ботов", "Разморозить ботов", "Сбить меня", "Выбыть"; foldable "Запустить карту" (map chips).

## 6. Connection banner (mutually exclusive)
| State | Content | Buttons |
|---|---|---|
| Connecting | spinner, "Подключение…" | "Отмена" |
| Reconnecting | spinner, "Связь потеряна — переподключаемся…" | "Отмена" |
| Refused | logo + server's message | "‹ К серверам", "Выйти из игры" (primary) |
| Outdated | logo, "Эта версия игры устарела: скачайте новую." + update state | update buttons + "‹ К серверам" |

## 7. Updates (box at top of server list, and in Outdated banner)
| State | Text | Buttons |
|---|---|---|
| Idle | "Версия {build}" | — |
| Available | "Вышла версия {version}" | "Обновить" or "Скачать" |
| Downloading | "Скачиваем обновление… {n}%" | — |
| Failed | "Не удалось обновиться: {reason}" (red) | "Попробовать снова", "Страница релиза" |
| Restarting | "Обновлено — перезапускаем…" | — |

## 9. In-room HUD

### 9.1 Standings panel (top left)
Heading: round → genre tag "{i}/{n}" (or "Тренировка") + genre name ("Гонка"/"Выживание"/"Очки") + map title; lobby → "Лобби" + room title; podium → "Итоги игры".
Rows (round: by round score; lobby: by crowns): place badge (gold/silver/bronze top 3 in a round), name (own highlighted; dimmed if out/disconnected), "⭐" host; lobby: "👑N", "🔔N"; round: status icon 👁 (not in round) / 🏁N (finished Nth) / ✖ (out); points genre: bells as green number; round/podium: score; ping ("бот", "—" if disconnected).

### 9.2 Net line (under standings)
"WebSocket" or "UDP" · "{n} мс" · "{n} к/с" (if FPS setting on). Fallback explanation: "Игра идёт через WebSocket: UDP до сервера не доходит" (+ longer VPN advice: route server address around VPN, or use Hysteria2/TUIC).

### 9.3 Timer (top middle, during round)
"m:ss", red below 10 s, pulse each of last 10 s; bar of time left. Map pill (map-specific live status). Bonus pill "{icon} {title} · {N} с": "🍄 Великан", "🦘 Мега-прыжок", "⚡ Ускорение".

### 9.4 Feed (top right)
Newest at bottom, each line 6 s. Knock-outs: "[attacker] {icon} victim — {cause}"; out → "выбывает ({cause})" red; shortcut → "срезка пути: −2". Causes: 🔨 молот, 🌀 вертушка, 🎳 шар, 💥 отбойник, 🧱 стена, 🧱 толкатель, 🥁 барабан, 📍 штырь, 🧊 блок, 🥊 перчатка, 🚪 ворота, 🕳 падение, 🤸 сбит нырком, ✊ захват, 🚫 срезка пути. Notes: "🏁 {name}: финиш, {n} место ({time})", "{icon} {name|Вы}: {bonus}!", "🔔 Вы звоните в колокол на башне" / "🔔 {name} звонит в колокол на башне (в N-й раз)", "Отчёт сохранён: {path}".

### 9.5 Prompt (in play, mouse free)
"🖱 Щёлкните по полю, чтобы вернуть управление" / "или нажмите Esc для меню".

### 9.6 Name tags
Over other beans: suit-colour dot + name + badge. Fade 40–60 m, hidden behind walls, not for own bean.

## 10. Round flow

### 10.1 Intro (pre-round, camera flies over course)
Card: genre tag + "Раунд {i} из {n}"; "Старт через {N} с"; big map title; "🎯 {goal}."; map description.

### 10.2 Countdown
Big "3", "2", "1"; then "ВПЕРЁД!" for 1.2 s.

### 10.3 Round results (between rounds)
Caption "итоги раунда {i} из {n}" (or "тренировка"); map title; "Следующий раунд через {N} с" / "Заново через {N} с" / "Итоги игры через {N} с". Table: place badge; name + note ("финиш за {time}" / "без финиша" / "в игре {N} с" / "до конца раунда" / "очки: {N}"); "+N" green; "−N" red (if non-zero); delta badge; total.

### 10.4 Status line (bottom centre, when not in play)
"🏁 Финиш: {N}-е место!" / "🏁 Финиш!" / "Вы выбыли" (red) / "👁 Вы зритель"; "Камера: {name}" or "Камера: обзор арены"; hint "ЛКМ/ПКМ или A/D — сменить игрока".

### 10.5 Game summary (beside 3D podium)
"Итоги игры"; "👑 Вы победили!" / "👑 {name} побеждает!" / "Игра окончена"; "Возврат в лобби через {N} с"; podium places 2-1-3 (name, total, place); table "Итоговая таблица": place, name, "🏆{wins}", total; "Награды": ⚡ Молния (best places in races), 🛡️ Несокрушимость ("{N} с в игре"), 💥 Задира ("{N} сбитых соперников"), 🤲 Цепкие руки ("{N} захватов"), 🍌 Неваляшка ("{N} падений"), 🦊 Хитрая лиса ("{N} срезок пути (и штрафы за них)").

## 11. Spectating
Camera follows a player still in play or arena overview. Switch: A/←/Q prev, D/→/E next, RMB prev / LMB next, gamepad D-pad/triggers.

## 12. Chat
Hidden by default; lines show 8 s after a new message (last six). Enter opens (in room, not practice, not menu); input at bottom left with placeholder "Сообщение: Enter — отправить, Esc — закрыть"; whole history visible while open; max 160 chars; sender name in suit colour; server notices from "Сервер". Not in practice.

## 13. Keys
WASD/arrows move; Space jump; E/Shift/Ctrl dive; Q grab; LMB dive; RMB grab; 1–5 emotes; Enter chat; Esc menu; F4 perf overlay; F8 report; F9 perf recording; F11 / Alt+Enter fullscreen.

## 14. Constraints
Name 16 chars; room title 24; PIN 4 digits; chat 160; address 120; up to 8 players per room (server sets max/min); custom list ≤12 maps; rounds 3/5/7.

## Appendix A. Maps (17)
| Title | Genre | Goal |
|---|---|---|
| Дверной переполох | Гонка | Добегите до финиша |
| Молоты и качели | Гонка | Доберитесь до финиша |
| Скользкий склон | Гонка | Доберитесь до финиша |
| Невидимый мост | Гонка | Найдите путь и добегите до финиша |
| Барабаны | Гонка | Доберитесь до финиша |
| Прыг-клуб | Выживание | Не упадите |
| Перекати-поле | Выживание | Не упадите |
| Скалолазы | Гонка | Заберитесь на вершину |
| Стенобой | Выживание | Не упадите |
| Хекс-а-гон | Выживание | Продержитесь дольше всех |
| Гора короны | Гонка | Доберитесь до короны |
| Падающие плиты | Выживание | Продержитесь дольше всех |
| Портальный переполох | Гонка | Добегите до финиша |
| Прыг-скок | Гонка | Допрыгайте до финиша |
| Ледяные небеса | Гонка | Доскользите до финиша |
| Звездопад | Очки | Соберите больше всех звёзд |
| Хвостики | Очки | Держите хвост как можно дольше |

## Appendix D. Outfit
Hats: Без шапки, 🧢 Кепка, 🧶 Шапка, 🥳 Колпак, 🎩 Цилиндр, 🤠 Ковбойская шляпа, ⚔ Викинг, 🚁 Пропеллер, 🐰 Ушки зайки, 🐱 Ушки кошки, 😈 Рожки, 😇 Нимб, 🌸 Цветок, 👽 Антенны.
Glasses: Без очков, 👓 Круглые, 🕶 Тёмные, 💕 Сердечки, 🧐 Монокль, 🥽 Визор.

## Notes
No confirmations on destructive actions. Menu doesn't pause. Settings autosave. "Выйти из игры" only in the refused card.
