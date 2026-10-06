Альфа-версия: возможны ошибки, сохранения и протокол могут меняться между версиями. Подписей у файлов нет.

## Игра

| Система | Файл | Как поставить |
| --- | --- | --- |
| Windows 10/11 | `FallBeans-@VERSION@-setup.exe` | запустить; ставится в профиль пользователя без прав администратора. SmartScreen: «Подробнее» → «Выполнить в любом случае» |
| Linux | `FallBeans-@VERSION@-x86_64.AppImage` | `chmod +x` и запустить |
| Linux (Flatpak) | `FallBeans-@VERSION@-x86_64.flatpak` | `flatpak install --user FallBeans-@VERSION@-x86_64.flatpak` |

Windows и AppImage обновляются сами: при новой версии на главном экране появится кнопка «Обновить». Flatpak покажет
ссылку на релиз; новый файл ставится командой `flatpak install --user --reinstall <файл>` (настройки сохраняются).

Нужна видеокарта с DirectX 12 (Windows 10 и новее) или Vulkan 1.2 (Linux, а на Windows — запасной вариант).

При первом запуске добавьте сервер: IP, имя машины, домен или `хост:порт`, которые скажет тот, кто его запустил.

## Сервер

| Система | Файл | Как поставить |
| --- | --- | --- |
| Debian, Ubuntu | `fallbeans-server_*_amd64.deb` | `sudo apt install ./fallbeans-server_*_amd64.deb` |
| Fedora, RHEL | `fallbeans-server-*.x86_64.rpm` | `sudo dnf install ./fallbeans-server-*.x86_64.rpm` |
| openSUSE | `fallbeans-server-*.x86_64.rpm` | `sudo zypper install --allow-unsigned-rpm ./fallbeans-server-*.x86_64.rpm` |

Служба `fallbeans` запускается сразу. Открыть в файерволе: **5887/tcp** (HTTP API), **5888/udp** (игра),
**5889/tcp** (WebSocket, запасной путь). Настройки — `/etc/fallbeans/fallbeans.env` (`FB_NAME` — название в
списке серверов, `FB_ARGS` — флаги `fb_server --help`), затем `sudo systemctl restart fallbeans`. Логи —
`journalctl -u fallbeans`. Клиент подключается только к серверу с той же версией протокола, а она
учитывает и код симуляции: обновите сервер вместе с клиентами.

Контрольные суммы — `SHA256SUMS`. Лицензия — GNU AGPL v3 или новее (`LICENSE` в каждом пакете, лицензии встроенных
библиотек — `THIRD-PARTY-LICENSES.html` рядом); исходный код этой версии — архивы «Source code» ниже. Кто меняет
сервер и пускает на него игроков, обязан дать им свои исходники.
