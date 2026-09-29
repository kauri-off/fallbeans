# Fall Beans

Браузерная пати-игра в духе Fall Guys на 2–5 игроков. Сервер отдаёт сайт и держит игру на одном порту.
Модели сделаны в Blender (`blender/fallguys_assets.blend`, экспорт в `public/assets/*.glb`).

## Запуск на сервере (Linux x64)

```sh
scp dist/FallBeans-linux-x64 deploy/fallbeans.service user@SERVER:/tmp/
ssh user@SERVER
sudo mkdir -p /opt/fallbeans && sudo mv /tmp/FallBeans-linux-x64 /opt/fallbeans/ && sudo chmod +x /opt/fallbeans/FallBeans-linux-x64
sudo cp /tmp/fallbeans.service /etc/systemd/system/ && sudo systemctl enable --now fallbeans
```

Игра открывается на `http://СЕРВЕР:7777`. Первый зашедший — хост, он запускает шоу.
Параметры: `--port 8080`, `--solo` (разрешить старт одному, для проверки).
Тренировка отдельной карты: `/?practice=race1` (`race2`, `jumpclub`, `hex`).

## Шоу под число игроков

| Игроков | Раунды |
|---|---|
| 2 | разминка → разминка → финал |
| 3 | разминка → отсев 1 → финал |
| 4 | отсев 1 → отсев 1 → финал |
| 5 | отсев 1 → отсев 1 → отсев 1 → финал |

Карты: «Дверной переполох» и «Молоты и качели» (гонки), «Прыг-клуб» (выживание), финал «Хекс-а-гон».

## Управление

WASD — бег, мышь — камера, Пробел — прыжок, E/ЛКМ — нырок, Q/ПКМ — хватать, 1/2/3 — эмоции. Геймпад поддерживается.

## Разработка

```sh
bun install
bun build.js --manifest-only && bun server.js   # запуск из исходников
bun build.js                                    # сборка dist/FallBeans-linux-x64
```
