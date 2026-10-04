Альфа-версия: возможны ошибки, сохранения и протокол могут меняться между версиями. Подписей у файлов нет.

## Игра

| Система | Файл | Как поставить |
| --- | --- | --- |
| Windows 10/11 | `FallBeans-@VERSION@-setup.exe` | запустить; ставится в профиль пользователя без прав администратора. SmartScreen: «Подробнее» → «Выполнить в любом случае» |
| Linux | `FallBeans-@VERSION@-x86_64.AppImage` | `chmod +x` и запустить |
| Linux (Flatpak) | `FallBeans-@VERSION@-x86_64.flatpak` | `flatpak install --user FallBeans-@VERSION@-x86_64.flatpak` |

Windows и AppImage обновляются сами: при новой версии на главном экране появится кнопка «Обновить». Flatpak покажет
ссылку на релиз; новый файл ставится командой `flatpak install --user --reinstall <файл>` (настройки сохраняются).

При первом запуске добавьте сервер: IP или домен, который скажет тот, кто его запустил.

## Сервер

| Система | Файл | Как поставить |
| --- | --- | --- |
| Debian, Ubuntu | `fallbeans-server_*_amd64.deb` | `sudo apt install ./fallbeans-server_*_amd64.deb` |
| Fedora, RHEL, openSUSE | `fallbeans-server-*.x86_64.rpm` | `sudo dnf install ./fallbeans-server-*.x86_64.rpm` |

Служба `fallbeans` запускается сразу. Открыть в файерволе: **5887/tcp** (HTTP API), **5888/udp** (игра),
**5889/tcp** (WebSocket, запасной путь). Настройки — `/etc/fallbeans/fallbeans.env` (`FB_NAME` — название в
списке серверов, `FB_ARGS` — флаги `fb_server --help`), затем `sudo systemctl restart fallbeans`. Логи —
`journalctl -u fallbeans`.

Контрольные суммы — `SHA256SUMS`.
