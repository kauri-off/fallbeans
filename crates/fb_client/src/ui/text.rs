//! Every line the player reads (port of `labels.ts` and the strings of `client/ui`), in Russian.
use fb_shared::outfit::{Glasses, Hat};

pub const LOGO: &str = "Fall Beans";
pub const TAB_ROOMS: &str = "Комнаты";
pub const TAB_SERVERS: &str = "Серверы";
pub const TAB_SETTINGS: &str = "Настройки";
pub const TAB_GAME: &str = "Игра";
pub const TAB_DEV: &str = "Dev";

pub const CONNECTING: &str = "Подключение…";
pub const RECONNECTING: &str = "Связь потеряна — переподключаемся…";
pub const UPDATING: &str = "Игра обновляется…";
pub const UPDATING_MORE: &str = "Подключимся сами, как только новая версия запустится.";
pub const OUTDATED: &str = "Эта версия игры устарела: скачайте новую.";
pub const QUIT: &str = "Выйти из игры";
pub const UPDATE: &str = "Обновить";
pub const DOWNLOAD: &str = "Скачать";
pub const RELEASE_PAGE: &str = "Страница релиза";
pub const RESTARTING: &str = "Обновлено — перезапускаем…";

pub fn version(build: &str) -> String {
    format!("Версия {build}")
}
pub fn update_out(v: &str) -> String {
    format!("Вышла версия {v}")
}
pub fn downloading(got: u64, total: u64) -> String {
    format!("Скачиваем обновление… {}%", (got * 100).checked_div(total).unwrap_or(0))
}
pub fn update_failed(e: &str) -> String {
    format!("Не удалось обновиться: {e}")
}
pub fn crashed(report: &str) -> String {
    format!("В прошлый раз игра упала. Отчёт: {report}")
}
pub const OPEN_LOGS: &str = "Открыть папку с логами";
pub const PROBLEMS: &str = "Если что-то сломалось";
pub const PROBLEMS_NOTE: &str = "F8 в игре сохраняет отчёт: что было с сетью и вашим бобом в последние секунды. \
Нажмите сразу, как заметили странное, и пришлите отчёт с логом из этой папки.";
pub fn report_saved(path: &str) -> String {
    format!("Отчёт сохранён: {path}")
}
pub const TO_SERVERS: &str = "‹ К серверам";
pub const SERVERS: &str = "Серверы";
pub const NO_SERVERS: &str = "Добавьте сервер: его адрес скажет тот, кто его запустил.";
pub const SERVER_PLACEHOLDER: &str = "IP или домен сервера";
pub const SERVER_HINT: &str = "Порт нужен, только если сервер слушает HTTP не на 5887: 192.168.1.10:7000.";
pub const SERVER_BAD: &str = "Не похоже на адрес: IP, домен или хост:порт.";
pub const ADD: &str = "Добавить";
pub const CHECKING: &str = "проверяем…";
pub const SERVER_DOWN: &str = "не отвечает";
pub const SERVER_UPDATING: &str = "обновляется";
pub const SERVER_OTHER_VERSION: &str = "другая версия игры";

pub fn server_line(players: u64, rooms: u64) -> String {
    format!("игроков: {players} · комнат: {rooms}")
}

pub const NAME_PLACEHOLDER: &str = "Ваше имя";
pub const SAVE: &str = "Сохранить";
pub const LOADING_ROOMS: &str = "Загружаем список…";
pub const NO_ROOMS: &str = "Пока нет ни одной комнаты — создайте свою.";
pub const ENTER: &str = "Войти";
pub const NO_PLACES: &str = "Мест нет";
pub const CANCEL: &str = "Отмена";
pub const PIN_PLACEHOLDER: &str = "PIN-код";
pub const PIN_ASK: &str = "PIN-код спросите у хоста комнаты.";
pub const PIN_LOCKED: &str = "🔒 Комната закрыта PIN-кодом";
pub const OWN_ROOM: &str = "Своя комната";
pub const ROOM_TITLE_PLACEHOLDER: &str = "Название комнаты";
pub const PRIVATE_CREATE: &str = "Приватная: вход по PIN-коду (вы увидите его в меню комнаты)";
pub const CREATE_ROOM: &str = "Создать комнату";
pub const BACK_TO_OWN: &str = "Вернуться в свою комнату";
pub const OWN_ROOM_NOTE: &str =
    "Своя комната у каждого одна. Пока вас нет, хостом в ней кто-то из оставшихся; вернётесь — роль снова ваша.";
pub const PRACTICE: &str = "Тренировка одной карты с ботами";
pub const IN_LOBBY: &str = "в лобби";
pub const IN_GAME: &str = "идёт игра";
pub const YOUR_ROOM: &str = "ваша комната";

pub const RESUME: &str = "Продолжить";
pub const RESUME_HINT: &str = "· Esc или щелчок по полю";
pub const LEAVE_ROOM: &str = "Выйти из комнаты";
pub const PRIVATE_ROOM: &str = "Приватная комната";
pub const PIN_FOR_ENTRY: &str = "(вход по PIN-коду)";
pub const PRIVATE_NOTE: &str = "Приватная комната: PIN-код для друзей знает хост ⭐";
pub const YOU: &str = " (вы)";
pub const NO_LINK: &str = "нет связи";
pub const GIVE_HOST: &str = "Отдать хоста";
pub const HOST_STARTS: &str = "Игру запускает хост ⭐";
pub const ABORT: &str = "Прервать игру";
pub const PODIUM_NOW: &str = "Игра окончена — награждение.";
pub const RESULTS_NOW: &str = "Итоги раунда.";
pub const ROUNDS: &str = "Раундов:";
pub const FILL_BOTS: &str = "Заполнять свободные места ботами";
pub const ADD_BOT: &str = "+ Добавить бота";
pub const START_GAME: &str = "Начать игру";
pub const PRACTICE_BACK: &str = "‹ К списку комнат";
pub const BACK_TO_ROOM: &str = "‹ Вернуться в комнату";
pub const OUTFIT: &str = "Шапка, очки и цвета";
pub const HAT_COLOR: &str = "Цвет шапки";
pub const BELLY: &str = "Цвет живота";
pub const SHOES: &str = "Цвет ботинок";
pub const RANDOM: &str = "🎲 Случайно";
pub const RESET: &str = "Сбросить";

pub const MODE_MIX: &str = "Микс";
pub const MODE_RACES: &str = "Гонки";
pub const MODE_SURVIVAL: &str = "Выживание";
pub const MODE_CUSTOM: &str = "Свой список";

pub const MOUSE_SENS: &str = "Чувствительность мыши";
pub const INVERT_MOUSE: &str = "Инвертировать мышь по вертикали";
pub const STICK_SENS: &str = "Чувствительность стика";
pub const INVERT_STICK: &str = "Инвертировать правый стик по вертикали";
pub const SHAKE: &str = "Качание камеры от ударов и приземлений";
pub const FOV: &str = "Угол обзора";
pub const VOLUME: &str = "Громкость звуков";
pub const UI_SCALE: &str = "Размер интерфейса";
pub const SHOW_FPS: &str = "Показывать частоту кадров";
pub const KEYS: &str = "Клавиши";
pub const GRAPHICS: &str = "Графика";
pub const PRESET: &str = "Качество графики";
pub const PRESET_AUTO: &str = "Авто";
pub const PRESET_LOW: &str = "Низкое";
pub const PRESET_MEDIUM: &str = "Среднее";
pub const PRESET_HIGH: &str = "Высокое";
pub const UPSCALE: &str = "Масштабирование AMD FSR";
pub const UPSCALES: [(&str, &str); 5] = [
    ("off", "Выкл"),
    ("ultra", "Ультра"),
    ("quality", "Качество"),
    ("balanced", "Баланс"),
    ("performance", "Скорость"),
];
pub const SHADOWS: &str = "Тени";
pub const AO: &str = "Фоновое затенение (AO, только «Высокое»)";
pub const AA: &str = "Сглаживание";
pub const GRADE: &str = "Цветокоррекция и виньетка";
pub const MOTES: &str = "Частицы в воздухе";
pub const VSYNC: &str = "Вертикальная синхронизация";
pub const FPS_LIMIT: &str = "Ограничение кадров в секунду";
pub const NO_LIMIT: &str = "Нет";
pub const BACKEND: &str = "Графический API (после перезапуска)";
pub const BACKEND_AUTO: &str = "Авто";

pub fn adapter(name: &str, tier: &str) -> String {
    format!("Видеокарта: {name} · уровень {tier}")
}
pub const PRESS_KEY: &str = "Нажмите клавишу… (Esc — отмена)";
pub const CHANGE: &str = "Изменить";
pub const RESET_KEYS: &str = "Вернуть как было";
pub const KEYS_NOTE: &str = "ЛКМ — тоже нырок, ПКМ — тоже захват; клавиши по месту на клавиатуре, раскладка не важна.";

pub fn bind(b: crate::keys::Bind) -> &'static str {
    use crate::keys::Bind;
    match b {
        Bind::Forward => "Вперёд",
        Bind::Back => "Назад",
        Bind::Left => "Влево",
        Bind::Right => "Вправо",
        Bind::Jump => "Прыжок",
        Bind::Dive => "Нырок",
        Bind::Grab => "Захват",
    }
}

pub const TO_START: &str = "Старт через";
pub const GO: &str = "ВПЕРЁД!";
pub const NEXT_ROUND: &str = "Следующий раунд через";
pub const AGAIN: &str = "Заново через";
pub const GAME_RESULTS_IN: &str = "Итоги игры через";
pub const BACK_TO_LOBBY: &str = "Возврат в лобби через";
pub const PRACTICE_SMALL: &str = "тренировка";
pub const GAME_OVER: &str = "Игра окончена";
pub const YOU_WON: &str = "Вы победили!";
pub const GAME_SUMMARY: &str = "Итоги игры";
pub const LOBBY: &str = "Лобби";
pub const PRACTICE_TITLE: &str = "Тренировка";
pub const YOU_ARE_OUT: &str = "Вы выбыли";
pub const SPECTATOR: &str = "👁 Вы зритель";
pub const CAMERA_OVERVIEW: &str = "Камера: обзор арены";
pub const SPECTATE_HINT: &str = "ЛКМ/ПКМ или A/D — сменить игрока";
pub const CLICK_FIELD: &str = "🖱 Щёлкните по полю, чтобы вернуть управление";
pub const OR_ESC_MENU: &str = "или нажмите Esc для меню";
pub const BOT: &str = "бот";
pub const AFK: &str = "AFK";

pub const VPN_SHORT: &str = "Игра идёт через WebSocket: UDP до сервера не доходит";
pub const VPN_SILENT: &str = "Игра идёт через WebSocket: UDP до сервера не доходит, задержка может быть больше.\n\
     Если включён VPN, пустите адрес сервера мимо него (direct):\n\
     v2rayN — правило «IP сервера: direct» выше правила, блокирующего udp443;\n\
     sing-box, Hiddify, NekoBox — правило ip_cidr или domain сервера.\n\
     Для игры лучше протокол с родным UDP: Hysteria2 или TUIC.";
pub const VPN_LOST: &str = "UDP пропал посреди игры (переподключился VPN или поменялись его правила): игра идёт через WebSocket.\n\
     Раз в минуту игра проверяет UDP и вернётся на него между раундами.\n\
     Чтобы UDP не пропадал, пустите адрес сервера мимо VPN (direct).";

pub const QUALITY_LOWERED: &str = "Качество графики снижено, чтобы игра шла плавнее";

pub const CHAT_PLACEHOLDER: &str = "Сообщение: Enter — отправить, Esc — закрыть";

pub fn genre(g: fb_shared::game::Genre) -> &'static str {
    g.label()
}

/// "1:05".
pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    format!("{}:{:02}", (s / 60.0).floor() as u32, (s % 60.0).floor() as u32)
}

/// "+3", "−2", "0".
pub fn signed(n: i64) -> String {
    match n {
        0 => "0".into(),
        n if n > 0 => format!("+{n}"),
        n => format!("−{}", -n),
    }
}

/// "1-е", "2-е"… (for «место»).
pub fn ordinal(n: usize) -> String {
    format!("{n}-е")
}

/// Russian plural: `plural(5, "игрок", "игрока", "игроков")` → «5 игроков».
pub fn plural(n: u32, one: &str, few: &str, many: &str) -> String {
    let (m10, m100) = (n % 10, n % 100);
    let w = if m10 == 1 && m100 != 11 {
        one
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        few
    } else {
        many
    };
    format!("{n} {w}")
}

pub fn need_players(min: u32) -> String {
    format!("Нужно хотя бы {}", plural(min, "игрок", "игрока", "игроков"))
}

pub fn players_of(n: usize, max: u32) -> String {
    format!("Игроки: {n} из {max}")
}

pub fn rooms_count(n: usize) -> String {
    if n == 0 {
        TAB_ROOMS.into()
    } else {
        format!("{TAB_ROOMS}: {n}")
    }
}

pub fn room_line(host: &str, playing: bool, mine: bool) -> String {
    let host = if host.is_empty() { "—" } else { host };
    let phase = if playing { IN_GAME } else { IN_LOBBY };
    let mine = if mine {
        format!(" · {YOUR_ROOM}")
    } else {
        String::new()
    };
    format!("хост ⭐ {host} · {phase}{mine}")
}

pub fn room_title_placeholder(name: &str) -> String {
    if name.is_empty() {
        ROOM_TITLE_PLACEHOLDER.into()
    } else {
        format!("Комната {name}")
    }
}

pub fn pin_line(pin: &str) -> String {
    format!("{PRIVATE_ROOM} — PIN-код для входа: {pin}")
}

pub fn round_now(index: u32, total: u32, title: &str) -> String {
    format!("Идёт раунд {index} из {total}: «{title}»")
}

pub fn practice_now(title: &str) -> String {
    format!("Тренировка: «{title}». Раунд повторяется с ботами, пока вы не вернётесь.")
}

pub fn round_of(index: u32, total: u32) -> String {
    format!("Раунд {index} из {total}")
}

pub fn results_of(index: u32, total: u32) -> String {
    format!("итоги раунда {index} из {total}")
}

pub fn wins(name: &str) -> String {
    format!("{name} побеждает!")
}

pub fn in_secs(label: &str, s: f64) -> String {
    format!("{label} {} с", s.ceil().max(0.0) as u32)
}

pub fn finished(place: usize) -> String {
    if place > 0 {
        format!("🏁 Финиш: {} место!", ordinal(place))
    } else {
        "🏁 Финиш!".into()
    }
}

pub fn camera_on(name: Option<&str>) -> String {
    match name {
        Some(n) => format!("Камера: {n}"),
        None => CAMERA_OVERVIEW.into(),
    }
}

pub fn ping(ms: u32) -> String {
    format!("{ms} мс")
}

pub fn bell(mine: bool, name: &str, times: u32) -> String {
    let again = if times > 1 {
        format!(" (в {times}-й раз)")
    } else {
        String::new()
    };
    if mine {
        format!("🔔 Вы звоните в колокол на башне{again}!")
    } else {
        format!("🔔 {name} звонит в колокол на башне{again}!")
    }
}

/// How the feed shows what knocked a bean off (collider tags and server causes).
pub fn cause(c: &str) -> (&'static str, String) {
    let (icon, text) = match c {
        "hammer" => ("🔨", "молот"),
        "rotor" => ("🌀", "вертушка"),
        "ball" => ("🎳", "шар"),
        "bumper" => ("💥", "отбойник"),
        "wall" => ("🧱", "стена"),
        "pusher" => ("🧱", "толкатель"),
        "drum" => ("🥁", "барабан"),
        "sweeper" => ("🧹", "метла"),
        "gate" => ("🚪", "ворота"),
        "tile" => ("🔶", "плитка ушла из-под ног"),
        "fall" => ("🕳", "падение"),
        "tackle" => ("🤸", "сбит нырком"),
        "grab" => ("✊", "захват"),
        "shortcut" => ("🚫", "срезка пути"),
        other => return ("💫", other.to_string()),
    };
    (icon, text.to_string())
}

pub fn ko_line(cause_text: &str, out: bool, shortcut: bool) -> String {
    if shortcut {
        "срезка пути: −2".into()
    } else if out {
        format!("выбывает ({cause_text})")
    } else {
        cause_text.into()
    }
}

pub fn hat(h: Hat) -> &'static str {
    match h {
        Hat::None => "Без шапки",
        Hat::Cap => "🧢 Кепка",
        Hat::Beanie => "🧶 Шапка",
        Hat::Party => "🥳 Колпак",
        Hat::Tophat => "🎩 Цилиндр",
        Hat::Cowboy => "🤠 Ковбойская",
        Hat::Viking => "⚔ Викинг",
        Hat::Propeller => "🚁 Пропеллер",
        Hat::Bunny => "🐰 Ушки зайки",
        Hat::Cat => "🐱 Ушки кошки",
        Hat::Horns => "😈 Рожки",
        Hat::Halo => "😇 Нимб",
        Hat::Flower => "🌸 Цветок",
        Hat::Antenna => "👽 Антенны",
    }
}

pub fn glasses(g: Glasses) -> &'static str {
    match g {
        Glasses::None => "Без очков",
        Glasses::Round => "👓 Круглые",
        Glasses::Shades => "🕶 Тёмные",
        Glasses::Hearts => "💕 Сердечки",
        Glasses::Monocle => "🧐 Монокль",
        Glasses::Visor => "🥽 Визор",
    }
}

/// The controls line at the bottom of the screen, for the device used last and the keys bound.
pub fn keys(pad: bool, binds: &crate::settings::Bindings, grab: &str, chat: bool, lead: &str) -> String {
    use crate::keys::{Bind, label};
    let body = if pad {
        format!("Левый стик — бег · Правый — камера · A — прыжок · X/B — нырок · RB/RT — {grab} · Крестовина — эмоции")
    } else if binds.is_default() {
        format!("WASD — бег · Мышь — камера · Пробел — прыжок · E/ЛКМ — нырок · Q/ПКМ — {grab} · 1–5 — эмоции")
    } else {
        let first = |b: Bind| binds.keys(b).first().and_then(|k| label(*k)).unwrap_or("?");
        let run = [Bind::Forward, Bind::Left, Bind::Back, Bind::Right]
            .map(first)
            .join("/");
        format!(
            "{run} — бег · Мышь — камера · {} — прыжок · {}/ЛКМ — нырок · {}/ПКМ — {grab} · 1–5 — эмоции",
            first(Bind::Jump),
            first(Bind::Dive),
            first(Bind::Grab)
        )
    };
    let chat = if chat && !pad { " · Enter — чат" } else { "" };
    format!("{lead}{body}{chat}")
}

pub fn menu_lead(pad: bool, host: bool) -> String {
    let key = if pad { "Start" } else { "Esc" };
    if host {
        format!("{key} — меню и запуск игры · ")
    } else {
        format!("{key} — меню · ")
    }
}

/// A bonus kind's icon and name.
pub fn bonus(kind: u8) -> (&'static str, &'static str) {
    use fb_sim::physics::power;
    match kind {
        power::GIANT => ("🍄", "Великан"),
        power::JUMP => ("🦘", "Мега-прыжок"),
        _ => ("⚡", "Ускорение"),
    }
}

/// Seconds with a decimal comma: «12,3 с».
pub fn fmt_sec(s: f64) -> String {
    format!("{:.1} с", s).replace('.', ",")
}

pub fn finish_note(name: &str, place: usize, time: f64) -> String {
    format!("🏁 {name}: финиш, {} место ({})", ordinal(place), fmt_sec(time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "игрок", "игрока", "игроков"), "1 игрок");
        assert_eq!(plural(3, "игрок", "игрока", "игроков"), "3 игрока");
        assert_eq!(plural(11, "игрок", "игрока", "игроков"), "11 игроков");
        assert_eq!(plural(22, "игрок", "игрока", "игроков"), "22 игрока");
        assert_eq!(fmt_time(65.9), "1:05");
        assert_eq!(signed(-2), "−2");
    }
}
