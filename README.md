# Bight

Bight — локальный VPN-клиент для Linux, Windows и Android. Он подключает sing-box, хранит профили на устройстве и синхронизирует их напрямую между телефоном и компьютером.

Без аккаунта Bight можно использовать как обычный клиент: импортировать VLESS, Hysteria2, Trojan или Shadowsocks, выбрать ноду и поднять соединение. Когда нужно перенести профиль на другое устройство, Bight поднимает локальную зашифрованную сессию, показывает QR-код, а второй клиент подключается к ней по Wi‑Fi.

[Сайт проекта](https://qualbyyy.github.io/Bight/) · [Релизы и скачивание](https://github.com/qualbyyy/Bight/releases) · [Actions](https://github.com/qualbyyy/Bight/actions)

## Что делает Bight

### Синхронизация ПК и телефона

Главная функция — перенос профилей без облачного сервера:

1. На компьютере откройте `LAN Sync` и создайте сессию.
2. На телефоне отсканируйте QR-код или введите код пары.
3. Bight устанавливает локальное соединение по Wi‑Fi.
4. Профили и настройки передаются в зашифрованном виде.
5. Изменения объединяются через LWW-мердж; удалённые объекты сохраняются как tombstone, поэтому не воскресают при следующей синхронизации.

Для обнаружения устройств используется mDNS. Данные синхронизации защищены X25519 и ChaCha20-Poly1305. Интернет и внешний сервер для этой операции не нужны.

### Подключение

- sing-box `>= 1.14`;
- VLESS, Hysteria2, Trojan и Shadowsocks;
- TUN и System Proxy на десктопе;
- Clash API на динамическом локальном порту;
- импорт URI и подписок;
- проверка пинга нод;
- просмотр активных соединений и логов;
- правила маршрутизации Direct / Proxy / Reject;
- исключение локальной сети из туннеля, чтобы LAN Sync продолжал работать при активном VPN.

## Состояние проекта

Проект находится в активной разработке. В репозитории уже есть:

- Rust workspace с `vpn-core`, `vpn-sync` и Tauri-бэкендом;
- интерфейс Tauri на React, TypeScript и Vite;
- desktop-логика запуска sing-box;
- локальная синхронизация профилей;
- GitHub Actions для Linux, Windows и Android;
- отдельный Android-плагин для системного `VpnService`.

Публикуемые артефакты появляются в [GitHub Actions](https://github.com/qualbyyy/Bight/actions) или в разделе [Releases](https://github.com/qualbyyy/Bight/releases), когда создан соответствующий релиз.

## Скачать

Стабильные релизы: [github.com/qualbyyy/Bight/releases](https://github.com/qualbyyy/Bight/releases)

Если релиз ещё не опубликован, тестовые артефакты можно найти на странице нужного запуска в Actions. Для Linux и Windows это desktop-бандлы, для Android — debug APK из workflow.

## Сборка из исходников

### Требования

- Node.js 22;
- Rust stable;
- npm;
- системные зависимости Tauri 2;
- для Android — JDK 17, Android SDK/NDK и Go;
- для Windows — Visual Studio Build Tools и WebView2;
- для Linux — GTK/WebKitGTK, AppIndicator, librsvg и `patchelf`.

### Frontend

```bash
npm ci
npm run build
npm run typecheck
```

### Rust workspace

```bash
CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0" cargo check --workspace
```

### Desktop-приложение

```bash
npm ci
npm run build
npx tauri build
```

### Android

Android-сборка использует системный `VpnService` и `libbox.aar`, а не дочерний процесс sing-box. Полная сборка выполняется workflow `Build Bight`, потому что `libbox.aar` собирается под Android NDK.

```bash
npm ci
npx tauri android init
npx tauri android build --debug --apk --target aarch64
```

## Архитектура

```text
Bight
├── crates/vpn-core       модели, URI/подписки, sing-box, Clash API
├── crates/vpn-sync       LAN Sync, mDNS, криптография, LWW-мердж
├── src-tauri             Tauri-команды, состояние, tray, запуск core
├── tauri-plugin-vpn      desktop/mobile bridge для VPN
├── src                   React-интерфейс приложения
└── docs                   публичная страница проекта
```

`vpn-core` не зависит от Tauri и операционной системы. `vpn-sync` работает как отдельный Rust-крейт и отвечает только за транспорт, pairing, шифрование и слияние данных. Tauri связывает эти части с интерфейсом и системными возможностями платформы.

## Сетевые правила

Bight намеренно оставляет локальную сеть доступной при включённом VPN:

- RFC1918-сети исключаются из маршрута TUN;
- правило `ip_is_private` ставится перед пользовательскими правилами;
- LAN Sync может работать, пока туннель активен;
- Clash API слушает динамический порт на `127.0.0.1`.

## Разработка

Для изменений frontend:

```bash
npm run dev
```

Для проверки перед коммитом:

```bash
npm run build && npm run typecheck && CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0" cargo check --workspace
```

Сборочные workflow находятся в `.github/workflows/`. `build.yml` собирает Linux, Windows и Android и прикладывает артефакты к запуску Actions. `pages.yml` публикует содержимое `docs/` на GitHub Pages.

## Безопасность

Bight не отправляет профили в собственный облачный сервис. LAN Sync работает внутри локальной сети и использует E2E-шифрование. Это не отменяет базовые меры предосторожности: не публикуйте pairing-код, QR-код или URI подписок в открытых чатах и логах.

Если вы нашли проблему, связанную с безопасностью, не прикладывайте рабочие ключи и конфиги в публичный issue. Сначала свяжитесь с владельцем репозитория через GitHub.

## Участие

Небольшие исправления можно отправлять pull request'ом. Перед этим проверьте frontend, TypeScript и Rust workspace командами из раздела выше. Для изменений в VPN-маршрутизации добавляйте сценарий, который проверяет работу LAN Sync при активном туннеле.

## Автор

Bight разрабатывается [qualbyyy](https://github.com/qualbyyy).
