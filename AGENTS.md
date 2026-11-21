# Технический план разработки (Blueprint / AGENTS.md)
## Кроссплатформенный VPN-клиент (Rust + Tauri v2 + sing-box) с Zero-Cloud LAN Sync

> **Инструкция для AI-агента (GPT-6 Luna):**
> Этот документ — единственный источник истины (Single Source of Truth) для разработки проекта.
> 1. **Не пиши всё сразу.** Выполняй работу строго по фазам из Раздела 8. Переходи к следующей фазе только после прохождения проверок (`Definition of Done`).
> 2. **Не используй устаревший синтаксис `sing-box`.** Используй строго схемы из Раздела 4 (совместимые с `sing-box >= 1.14.0`). Запрещены `inet4_address`, `geosite`/`geoip` поля, `outbounds` типов `block`/`dns` и legacy-формат `dns.servers` с полем `address`.
> 3. **Разделяй платформенный код.** Бизнес-логика, парсеры, криптография и генерация конфигов живут в `crates/vpn-core` (чистый Rust без зависимостей от Tauri и ОС).
> 4. **Перед каждым запуском туннеля** валидируй сгенерированный конфиг и учитывай все крайние случаи из Раздела 7.

---

## 1. Концепция и целевые платформы

**Цель:** Создать легкий, быстрый кроссплатформенный VPN-клиент (в духе INCY / Throne) на базе ядра `sing-box` с чистым UI и главной уникальной фичей — **безоблачной P2P/LAN-синхронизацией (Zero-Cloud LAN Sync)** подписок, выбранных узлов и кастомных правил маршрутизации между ПК и смартфоном по QR-коду.

### Целевые платформы
1. **Linux (EndeavourOS / Arch / KDE Plasma):** sidecar `sing-box`, системный трей, TUN через Linux capabilities (`CAP_NET_ADMIN`) или fallback на System Proxy.
2. **Windows (10/11, включая урезанные сборки):** sidecar `sing-box.exe` + `wintun.dll`, TUN (с elevation) или System Proxy (без прав администратора), гарантированная очистка процессов через Windows Job Object.
3. **Android (Android 14–16):** нативный Tauri v2 мобильный плагин на Kotlin (`VpnService` + официальная библиотека `libbox.aar`), Foreground Service, сканирование QR-кода камерой для синхронизации с ПК.

---

## 2. Архитектура монорепозитория (Cargo Workspace + Tauri v2)

```text
vpn-client/
├── Cargo.toml                      # Workspace root
├── package.json                    # Frontend (Vite + React + TS + Tailwind)
├── BLUEPRINT.md                    # Этот файл (контекст для агента)
├── crates/
│   ├── vpn-core/                   # 100% чистый Rust (тестируется через cargo test)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models.rs           # Subscription, ProxyNode, RoutingRule, AppState
│   │       ├── storage.rs          # Атомарное сохранение JSON + бэкапы
│   │       ├── parser/
│   │       │   ├── mod.rs          # Диспетчер форматов (base64, plain, json)
│   │       │   ├── vless.rs        # VLESS + Reality + Vision + WS/gRPC
│   │       │   ├── hysteria2.rs    # hy2:// / hysteria2://
│   │       │   ├── trojan.rs       # trojan://
│   │       │   └── shadowsocks.rs  # ss:// (SIP002 + legacy)
│   │       ├── singbox/
│   │       │   ├── mod.rs
│   │       │   ├── schema.rs       # Типизированные структуры конфига sing-box 1.14+
│   │       │   └── builder.rs      # Генератор config.json (Desktop vs Android)
│   │       ├── clash_api.rs        # HTTP/WS клиент к локальному Clash API (пинг, трафик)
│   │       └── sync_merge.rs       # Детерминированный LWW-мердж с tombstones
│   │
│   └── vpn-sync/                   # Модуль Zero-Cloud LAN Sync
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── crypto.rs           # E2E AEAD шифрование (ChaCha20Poly1305)
│           ├── net_discovery.rs    # Фильтрация валидных LAN IPv4 (без docker/tun)
│           ├── qr_payload.rs       # Формат URI vpnsync://v1?...
│           ├── server.rs           # Временный axum-сервер (Host mode)
│           └── client.rs           # Параллельный probe + обмен (Client mode)
│
├── tauri-plugin-vpn/               # Кастомный Tauri v2 плагин для Android VpnService
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   └── mobile.rs               # Вызов Kotlin-методов через PluginHandle
│   └── android/
│       ├── build.gradle.kts        # Подключение libbox.aar
│       ├── libs/libbox.aar         # Собирается через gomobile или берется из релизов sing-box
│       └── src/main/java/com/app/vpn/
│           ├── VpnPlugin.kt        # Tauri Plugin entrypoint
│           ├── CoreVpnService.kt   # Android VpnService + ForegroundService
│           └── LibboxBridge.kt     # Имплементация PlatformInterface для libbox
│
├── src-tauri/                      # Tauri v2 Host (Desktop + Mobile Glue)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   │   ├── default.json
│   │   └── mobile.json             # Права камеры (barcode-scanner) и vpn-плагина
│   ├── binaries/                   # Sidecar бинарники (с target triple!)
│   │   ├── sing-box-x86_64-unknown-linux-gnu
│   │   ├── sing-box-x86_64-pc-windows-msvc.exe
│   │   └── wintun.dll              # Ресурс рядом с exe для Windows
│   └── src/
│       ├── main.rs
│       ├── lib.rs                  # Инициализация Tauri, IPC команды, трей
│       ├── commands.rs             # #[tauri::command] обертки
│       └── backend/
│           ├── mod.rs              # Trait TunnelBackend
│           ├── desktop_linux.rs    # Sidecar + capabilities + cleanup
│           ├── desktop_windows.rs  # Sidecar + JobObject + System Proxy / UAC
│           └── android.rs          # Вызов tauri-plugin-vpn
│
└── src/                            # Frontend (React 19 + TypeScript + Tailwind CSS)
    ├── api/tauri.ts                # Типизированные invoke() обертки
    ├── store/useAppStore.ts        # Zustand store
    ├── components/
    │   ├── ConnectionButton.tsx
    │   ├── NodeList.tsx
    │   ├── RulesEditor.tsx
    │   └── SyncModal.tsx           # QR генерация (Host) и сканер/ввод кода (Client)
    └── App.tsx
```

### Ключевые зависимости (проверенные и стабильные)
* **`vpn-core`:** `serde`, `serde_json`, `url`, `base64`, `uuid`, `chrono`, `thiserror`, `reqwest` (с `rustls-tls`, **НЕ** `native-tls`, чтобы не ломать кросс-компиляцию под Android!), `tokio`.
* **`vpn-sync`:** `axum`, `chacha20poly1305`, `rand`, `get_if_addrs` (или `if-addrs`), `zeroize`, `qrcode` (для генерации SVG на бэкенде или `qrcode.react` на фронтенде).
* **`src-tauri`:** `tauri` (v2, features: `tray-icon` на desktop), `tauri-plugin-shell` (только `#[cfg(desktop)]`), `tauri-plugin-barcode-scanner` (только `#[cfg(mobile)]`), `tauri-plugin-clipboard-manager`.

---

## 3. Модель данных и стратегия хранения

Чтобы синхронизация ПК ↔ Android работала надежно и без конфликтов, каждая синхронизируемая сущность **обязана** содержать `id`, `updated_at` (Unix timestamp в миллисекундах) и флаг мягкого удаления `deleted: bool` (tombstone).

### Пример структур (`crates/vpn-core/src/models.rs`)

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub schema_version: u32,            // Текущая версия = 1
    pub device_id: Uuid,                // Уникальный ID устройства (генерируется 1 раз)
    pub subscriptions: Vec<Subscription>,
    pub custom_nodes: Vec<ProxyNode>,   // Ноды, добавленные вручную без подписки
    pub routing_rules: Vec<RoutingRule>,
    pub settings: LocalSettings,        // НЕ синхронизируется (TUN/Proxy, автозапуск и т.д.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub nodes: Vec<ProxyNode>,
    pub traffic_used: Option<u64>,      // Из заголовка subscription-userinfo
    pub traffic_total: Option<u64>,
    pub expire_at: Option<i64>,
    pub last_fetched_at: Option<i64>,
    // Поля для синхронизации:
    pub updated_at: i64,                // Unix ms (UTC)
    pub deleted: bool,                  // Tombstone для удаления на других устройствах
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: Uuid,
    pub enabled: bool,
    pub match_type: RuleMatchType,      // DomainSuffix, DomainKeyword, IpCidr, RuleSet
    pub value: String,
    pub action: RuleAction,             // Proxy, Direct, Reject
    pub comment: String,
    pub updated_at: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum ProxyProtocol {
    Vless {
        uuid: String,
        flow: Option<String>,           // "xtls-rprx-vision"
        security: TlsMode,              // None, Tls, Reality
        sni: Option<String>,
        fingerprint: Option<String>,    // "chrome", "firefox"
        pbk: Option<String>,            // Reality public key
        sid: Option<String>,            // Reality short id
        transport: TransportType,       // Tcp, Ws, Grpc, HttpUpgrade
    },
    Hysteria2 {
        password: String,
        sni: Option<String>,
        insecure: bool,
        obfs_type: Option<String>,      // "salamander"
        obfs_password: Option<String>,
        up_mbps: Option<u32>,
        down_mbps: Option<u32>,
    },
    Trojan {
        password: String,
        sni: Option<String>,
        insecure: bool,
        transport: TransportType,
    },
    Shadowsocks {
        method: String,
        password: String,
    },
}
```

### Надежное хранение (Atomic Write)
Никогда не пиши напрямую в `state.json` (при выключении питания или краше файл обнулится):
1. Сериализуй в `state.json.tmp`.
2. Вызови `file.sync_all()`.
3. Сделай резервную копию текущего `state.json` -> `state.json.bak` (если существует).
4. Атомарно переименуй `state.json.tmp` -> `state.json`.
5. При старте, если `state.json` поврежден, автоматически восстанавливайся из `state.json.bak`.

---

## 4. Актуальный эталонный конфиг `sing-box` (>= 1.14 / 1.15)

**Внимание для GPT-6 Luna:** Большинство старых примеров в интернете не работают на современных версиях `sing-box` (ядро падает с `FATAL: legacy ... removed`). Генератор `builder.rs` **обязан** собирать JSON строго по этому эталону.

### Эталонный шаблон `config.json` (режим TUN, Desktop)

```json
{
  "log": {
    "level": "info",
    "timestamp": true
  },
  "dns": {
    "servers": [
      {
        "tag": "remote-dns",
        "type": "https",
        "server": "1.1.1.1",
        "detour": "proxy"
      },
      {
        "tag": "local-dns",
        "type": "local"
      }
    ],
    "rules": [
      {
        "rule_set": "geosite-ru-direct",
        "server": "local-dns"
      }
    ],
    "final": "remote-dns",
    "strategy": "prefer_ipv4"
  },
  "inbounds": [
    {
      "type": "tun",
      "tag": "tun-in",
      "address": [
        "172.19.0.1/30"
      ],
      "mtu": 1400,
      "auto_route": true,
      "strict_route": false,
      "route_exclude_address": [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16",
        "fc00::/7",
        "fe80::/10"
      ]
    },
    {
      "type": "mixed",
      "tag": "mixed-in",
      "listen": "127.0.0.1",
      "listen_port": 2080
    }
  ],
  "outbounds": [
    {
      "type": "vless",
      "tag": "proxy",
      "server": "example.com",
      "server_port": 443,
      "uuid": "00000000-0000-0000-0000-000000000000",
      "flow": "xtls-rprx-vision",
      "tls": {
        "enabled": true,
        "server_name": "www.microsoft.com",
        "utls": {
          "enabled": true,
          "fingerprint": "chrome"
        },
        "reality": {
          "enabled": true,
          "public_key": "REALITY_PBK",
          "short_id": "REALITY_SID"
        }
      }
    },
    {
      "type": "direct",
      "tag": "direct"
    }
  ],
  "route": {
    "auto_detect_interface": true,
    "default_domain_resolver": "local-dns",
    "rules": [
      {
        "action": "sniff"
      },
      {
        "protocol": "dns",
        "action": "hijack-dns"
      },
      {
        "ip_is_private": true,
        "outbound": "direct"
      },
      {
        "rule_set": [
          "geoip-ru",
          "geosite-ru-direct"
        ],
        "outbound": "direct"
      }
    ],
    "rule_set": [
      {
        "tag": "geoip-ru",
        "type": "remote",
        "format": "binary",
        "url": "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-ru.srs",
        "download_detour": "proxy"
      },
      {
        "tag": "geosite-ru-direct",
        "type": "remote",
        "format": "binary",
        "url": "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs",
        "download_detour": "proxy"
      }
    ],
    "final": "proxy"
  },
  "experimental": {
    "cache_file": {
      "enabled": true,
      "path": "cache.db",
      "store_dns": true
    },
    "clash_api": {
      "external_controller": "127.0.0.1:DYNAMIC_PORT",
      "secret": "RANDOM_SESSION_SECRET"
    }
  }
}
```

### Критические отличия генерации конфига по платформам
* **Linux:** Для `tun-in` можно добавить `"auto_redirect": true` (улучшает совместимость с Docker и снижает нагрузку на CPU), **не** задавать поле `"stack"` (удалено/deprecated в 1.15, используется встроенный стек `sing-tun`).
* **Windows:** `"auto_redirect"` не используется; в режиме *System Proxy* (без прав администратора) блок `tun-in` полностью исключается из `inbounds`, остается только `mixed-in` на `127.0.0.1:<port>`, а системный прокси прописывается в реестр.
* **Android (`libbox`):**
  * В `route` **убрать** `"auto_detect_interface": true` (маршрутизацией интерфейсов управляет `PlatformInterface` в Kotlin).
  * В `inbounds` `tun-in` обязательно оставить `"route_exclude_address"` с RFC1918, чтобы работал LAN Sync!
  * `cache_file.path` должен указывать на внутреннюю директорию приложения (`context.filesDir/cache.db`).

---

## 5. Уникальная фича: Zero-Cloud LAN Sync (ПК ↔ Android)

Синхронизация происходит напрямую по локальной Wi-Fi сети без серверов, аккаунтов и облаков. Протокол поддерживает **двустороннее слияние (Bidirectional LWW Merge)** или **перезапись (Push/Pull)** за один HTTP-запрос.

### 5.1. Поток работы (Handshake & Exchange)

1. **Устройство A (Host, например ПК):**
   * Нажимает «Синхронизировать устройства».
   * Генерирует:
     * `key`: 32 случайных байта (`ChaCha20Poly1305` AEAD-ключ).
     * `sid`: UUID v4 (одноразовый идентификатор сессии).
   * Запускает локальный `axum`-сервер на `0.0.0.0:0` (ОС сама выдает свободный порт `port`).
   * Собирает список **всех валидных LAN IPv4** (например, `["192.168.1.42"]`).
   * Формирует QR-строку и короткий текстовый код (на случай, если синхронизируются 2 ПК без камеры):
     ```text
     vpnsync://v1?ips=192.168.1.42&port=51234&key=<base64url_32b>&sid=<uuid>&mode=merge
     ```
   * Запускает таймер автоотключения сервера (**TTL = 180 секунд**).

2. **Устройство B (Client, например Galaxy S25):**
   * Сканирует QR через `tauri-plugin-barcode-scanner` (или вставляет код из буфера).
   * Параллельно отправляет `GET http://{ip}:{port}/health?sid={sid}` на все IP из `ips` с таймаутом `1500ms`. Выбирает первый ответивший `200 OK`.
   * Собирает свой `SyncPayload` (только `subscriptions`, `custom_nodes`, `routing_rules`, **без** локальных настроек `LocalSettings`).
   * Шифрует `SyncPayload` ключом `key` с помощью `ChaCha20Poly1305` + случайного `12-byte nonce`, используя `sid` как **AAD** (Additional Authenticated Data).
   * Отправляет `POST http://{ip}:{port}/sync` с JSON-конвертом `{ sid, nonce_b64, ciphertext_b64 }`.

3. **Слияние на Host и ответ:**
   * Host проверяет `sid` (если не совпадает или сессия уже использована — `403 Forbidden`).
   * Расшифровывает payload клиента (при подделке тега AEAD — `401 Unauthorized`).
   * Выполняет `merge_states(&mut host_state, client_payload)` по правилам LWW.
   * Атомарно сохраняет новый стейт на диске, обновляет UI на ПК.
   * Шифрует **объединенный** `SyncPayload` новым `nonce` и тем же ключом `key`, отправляет в теле ответа `200 OK`.
   * Через 1 секунду **автоматически останавливает** временный HTTP-сервер и затирает ключ в памяти (`zeroize`).

4. **Применение на Client:**
   * Client расшифровывает ответ, сохраняет объединенный стейт, обновляет UI.
   * Если добавились новые подписки (только URL без нод или устаревшие) — предлагает/запускает фоновое обновление.

### 5.2. Ключевые фрагменты кода для `vpn-sync`

#### E2E-шифрование (`crates/vpn-sync/src/crypto.rs`)
Использование AEAD поверх HTTP полностью избавляет от головной боли с самоподписанными TLS-сертификатами на Android и при этом гарантирует конфиденциальность и целостность данных в локальной сети.

```rust
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub sid: String,
    pub nonce: Vec<u8>,       // 12 bytes
    pub ciphertext: Vec<u8>,
}

pub fn encrypt_payload(
    key: &[u8; 32],
    sid: &str,
    plaintext: &[u8],
) -> Result<EncryptedEnvelope, String> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad: sid.as_bytes() })
        .map_err(|e| format!("Encryption error: {e}"))?;

    Ok(EncryptedEnvelope {
        sid: sid.to_string(),
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    })
}

pub fn decrypt_payload(
    key: &[u8; 32],
    expected_sid: &str,
    env: &EncryptedEnvelope,
) -> Result<Vec<u8>, String> {
    if env.sid != expected_sid || env.nonce.len() != 12 {
        return Err("Invalid session ID or nonce length".into());
    }
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(&env.nonce);

    cipher
        .decrypt(
            nonce,
            Payload {
                msg: env.ciphertext.as_slice(),
                aad: expected_sid.as_bytes(),
            },
        )
        .map_err(|_| "Decryption failed: wrong key or tampered payload".into())
}
```

#### Фильтрация LAN IP (`crates/vpn-sync/src/net_discovery.rs`)
Одна из самых частых ошибок — выбрать IP виртуального интерфейса (`docker0`, `tun0`, `wintun`, `veth`, `WSL`).

```rust
use std::net::Ipv4Addr;

pub fn is_valid_lan_interface(name: &str, ip: Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
        return false;
    }
    // Исключаем наш собственный TUN диапазон (172.19.0.0/24) и виртуальные мосты
    let octets = ip.octets();
    if octets[0] == 172 && octets[1] == 19 {
        return false;
    }
    let lower = name.to_lowercase();
    let blocked_prefixes = [
        "docker", "br-", "veth", "virbr", "tun", "tap", "wg", "utun",
        "tailscale", "vethernet", "wsl", "vbox", "vmnet",
    ];
    if blocked_prefixes.iter().any(|p| lower.contains(p)) {
        return false;
    }
    // Разрешаем только RFC1918 (192.168.x.x, 10.x.x.x, 172.16-31.x.x)
    ip.is_private()
}
```

#### Детерминированный LWW Merge (`crates/vpn-core/src/sync_merge.rs`)
Для каждого списка (`subscriptions`, `custom_nodes`, `routing_rules`):
1. Собираем `HashMap<Uuid, Item>` из локального стейта.
2. Проходим по элементам удаленного стейта:
   * Если `id` нет локально -> добавляем.
   * Если `id` есть -> сравниваем `remote.updated_at > local.updated_at`: если удаленная запись свежее (включая `deleted: true`), перезаписываем локальную.
   * **Очистка мусора (Garbage Collection):** tombstones (`deleted == true`) старше 30 дней можно физически удалять из файла, чтобы JSON не раздувался.
3. **Защита от рассинхрона часов:** в `/health` можно возвращать `host_time_ms` и предупреждать пользователя, если часы устройств расходятся более чем на 5 минут.

---

## 6. Платформенные адаптеры (`TunnelBackend`)

В `src-tauri/src/backend/mod.rs` вводится единый асинхронный интерфейс:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum TunnelStatus {
    Disconnected,
    Connecting,
    Connected { clash_port: u16, mode: String },
    Error { message: String },
}

#[async_trait::async_trait]
pub trait TunnelBackend: Send + Sync {
    async fn start(&self, config_json: String, clash_port: u16, use_tun: bool) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn status(&self) -> TunnelStatus;
}
```

### 6.1. Linux (`desktop_linux.rs` — EndeavourOS / KDE Plasma)
* **Sidecar:** `sing-box` лежит рядом с приложением (`externalBin` в `tauri.conf.json`).
* **Права TUN без `sudo` на каждый клик:**
  * При первом включении TUN проверяем capabilities бинарника через `getcap <path_to_sing_box>`.
  * Если `cap_net_admin` нет — показываем понятный диалог и выполняем один раз через `pkexec`:
    ```bash
    pkexec setcap cap_net_admin,cap_net_bind_service,cap_dac_read_search=+ep /path/to/sing-box
    ```
  * После этого `sing-box` запускается обычным непривилегированным пользователем и спокойно поднимает `tun0`.
* **Graceful Shutdown:** Посылаем `SIGTERM` дочернему процессу, ждем до 3 секунд, затем `SIGKILL`. Перехватываем `ExitRequested` в Tauri, чтобы закрыть туннель при выходе из приложения.

### 6.2. Windows (`desktop_windows.rs`)
* **Sidecar + `wintun.dll`:** `wintun.dll` (архитектура строго совпадает с `sing-box.exe`!) копируется в ту же рабочую папку, откуда запускается `sing-box.exe` (или лежит в `resources/`).
* **Два режима работы:**
  1. **System Proxy (по умолчанию без UAC):** `sing-box.exe` поднимает только `mixed-in` на `127.0.0.1:2080`. Клиент выставляет `ProxyEnable = 1` и `ProxyServer = 127.0.0.1:2080` в `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` и вызывает `InternetSetOptionW(NULL, INTERNET_OPTION_SETTINGS_CHANGED, ...)` + `INTERNET_OPTION_REFRESH`.
  2. **TUN (с правами администратора):** Проверка токена процесса (`IsUserAnAdmin`). Если прав нет — перезапуск приложения или запуск helper с `"runas"`.
* **Защита от «зомби-процессов» (Windows Job Object):**
  * Привязывай дочерний процесс `sing-box.exe` к WinAPI `JobObject` с флагом `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Если GUI упадет по `panic!` или его убьют через Диспетчер задач, Windows **автоматически** убьет `sing-box.exe`.
  * Для System Proxy на старте приложения проверяй, не остался ли наш `127.0.0.1:2080` в реестре после краша, и очищай его.

### 6.3. Android (`tauri-plugin-vpn` + `libbox.aar`)
На Android **нельзя** запускать `sing-box` как дочерний бинарник (`Command::new`) — система блокирует исполнение бинарников и требует системный `VpnService` с файловым дескриптором.

#### Архитектура моста Rust ↔ Kotlin ↔ `libbox.aar`
1. `libbox.aar` кладется в `tauri-plugin-vpn/android/libs/libbox.aar` и подключается в `build.gradle.kts` (`implementation(files("libs/libbox.aar"))`).
2. `VpnPlugin.kt` (наследник `app.tauri.plugin.Plugin`):
   * Метод `@Command fun startVpn(invoke: Invoke)`:
     * Получает `configJson: String` из Rust.
     * Проверяет разрешение `VpnService.prepare(activity)`. Если не выдано — запрашивает через `startActivityForResult`, сохраняя `invoke`.
     * После выдачи разрешения запускает `CoreVpnService` через `ContextCompat.startForegroundService(context, intent)` (передавая путь к файлу конфига, **не** сам большой JSON в `Intent` extras, т.к. у `Binder` лимит ~1MB!).
3. `CoreVpnService.kt` (наследник `android.net.VpnService` + реализует `io.nekohasekai.libbox.PlatformInterface`):
   * **Обязательно:** `builder.addDisallowedApplication(packageName)` — исключает само приложение (и его LAN Sync / запросы подписок) из туннеля, предотвращая бесконечную петлю!
   * **Обязательно для Android 14–16:** В `AndroidManifest.xml` указать `android:foregroundServiceType="specialUse"` (или `systemExempted`) и вызвать `startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)` в первые 5 секунд `onStartCommand`.
   * Вызывает `Libbox.newService(configContent, platformInterface)` и `boxService.start()`.

#### Фрагмент `AndroidManifest.xml` (обязательные поля)
```xml
<uses-permission android:name="android.permission.INTERNET" />
<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.CAMERA" />

<application android:networkSecurityConfig="@xml/network_security_config">
    <service
        android:name="com.app.vpn.CoreVpnService"
        android:exported="false"
        android:permission="android.permission.BIND_VPN_SERVICE"
        android:foregroundServiceType="specialUse">
        <intent-filter>
            <action android:name="android.net.VpnService" />
        </intent-filter>
        <property
            android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"
            android:value="VPN tunnel service powered by sing-box" />
    </service>
</application>
```

#### Фрагмент `res/xml/network_security_config.xml` (для LAN Sync на Android!)
Android по умолчанию блокирует `http://` (cleartext). Чтобы LAN Sync по `http://192.168.x.x` с E2E-шифрованием работал стабильно, разреши cleartext (безопасно, так как весь payload зашифрован `ChaCha20Poly1305`):
```xml
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <base-config cleartextTrafficPermitted="true" />
</network-security-config>
```

---

## 7. Реестр критических ошибок (Pitfalls & Стабильные решения)

Этот раздел обязателен к проверке при написании каждого модуля.

| № | Проблема / Ошибка | Почему возникает | Стабильное решение (внедрить сразу) |
| :- | :--- | :--- | :--- |
| **1** | `sing-box` падает при старте с `FATAL: legacy ... deprecated/removed` | AI сгенерировал старую схему (`inet4_address`, `geoip`, `block` outbound, `dns.servers[].address`) | Жестко использовать эталон из Раздела 4 (`sing-box >= 1.14`). На Desktop перед запуском выполнять `sing-box check -c config.json` и выводить stderr пользователю. |
| **2** | **Петля маршрутизации (Routing Loop):** после включения VPN интернет полностью пропадает | Трафик до самого VPN-сервера или DNS-запросы заворачиваются обратно в TUN | 1) Desktop: `"auto_detect_interface": true` + `"default_domain_resolver": "local-dns"`.<br>2) Android: **обязательно** `builder.addDisallowedApplication(packageName)` и `protect(fd)` в `PlatformInterface.autoDetectInterfaceControl`. |
| **3** | **LAN Sync не может подключиться** при включенном VPN | Весь локальный трафик `192.168.x.x` уходит в туннель на удаленный сервер | Обязательно включать RFC1918 в `tun.route_exclude_address` + правило `{"ip_is_private": true, "outbound": "direct"}` на первом месте маршрутизации. |
| **4** | **LAN Sync выбрал неверный IP** (например `172.17.0.1` от Docker или `172.19.0.1` от TUN) | Наивное взятие первого IP из сетевых интерфейсов | Использовать фильтр `is_valid_lan_interface()` (Раздел 5.2) + передавать в QR **все** валидные LAN IP массивом `ips=ip1,ip2` и параллельно пинговать их с клиента. |
| **5** | **Конфликт порта Clash API (`9090` занят)** | Запущен другой клиент (INCY, Clash, v2rayN) | Перед каждой генерацией `config.json` открывать `TcpListener::bind("127.0.0.1:0")`, забирать `local_addr().port()`, закрывать сокет и вписывать этот порт в `clash_api.external_controller`. |
| **6** | **Интернет сломан после краша/закрытия на Windows** | В реестре остался `ProxyEnable = 1` на мертвый локальный порт или висит `sing-box.exe` | 1) Запускать `sing-box.exe` внутри WinAPI `JobObject` (`KILL_ON_JOB_CLOSE`).<br>2) Вешать очистку реестра на выход из приложения + при следующем запуске проверять/сбрасывать зависший системный прокси. |
| **7** | **Android 14–16 убивает VPN через минуту** | Отсутствует `foregroundServiceType` или уведомление | Объявить `specialUse` в `AndroidManifest.xml`, вызывать `startForeground(...)` немедленно, запросить `POST_NOTIFICATIONS` и опционально предложить отключить оптимизацию батареи. |
| **8** | **Падение `reqwest` при сборке под Android** | `native-tls` требует OpenSSL, который ломает кросс-компиляцию под `aarch64-linux-android` | В `Cargo.toml` всегда отключать `default-features` у `reqwest` и включать `rustls-tls` + `json`. |
| **9** | **Парсер подписок падает на реальных ссылках** | Подписки бывают с обрезанным Base64 без `=`, с URL-safe символами `-/_`, пробелами `\r\n`, emoji и `%20` в `#remark` | Написать нормализатор Base64 (удалять whitespace, добивать `=` до кратности 4, пробовать `STANDARD` и `URL_SAFE`), делать `percent_decode_str` для имен нод и не ронять всю подписку из-за 1 битой строки (логировать и пропускать). |
| **10** | **Зависание UI при проверке пинга 100+ серверов** | Одновременный спаун сотен TCP/HTTP проверок или блокировка main thread | Делать проверку в `tokio` через `Arc<Semaphore::new(12)>` с жестким таймаутом `2500ms` и отправлять результаты на фронтенд потоково через Tauri Event (`app.emit("ping-result", ...)`). |
| **11** | **Windows Firewall блокирует LAN Sync (Host)** | Входящие подключения к неизвестному порту режутся в Public-профиле сети | При старте LAN Sync Host предупреждать, если сеть помечена как Public, и дать кнопку «Поменять роли» (поднять Host на Android, а подключиться с Windows — исходящие запросы Firewall не блокирует!). |
| **12** | **Binder TransactionTooLargeException на Android** | Передача огромного `config.json` (если много правил) напрямую в `Intent.putExtra` | Сохранять `config.json` во внутренний файл `filesDir/active_config.json` и передавать в `CoreVpnService` только путь к файлу. |

---

## 8. Пошаговый план разработки для GPT-6 Luna (Milestones)

Скармливай агенту строго **по одной фазе за раз**, требуя проверки `Definition of Done` перед переходом дальше.

### Фаза 0: Инициализация Workspace и каркаса Tauri v2
* **Задача:** Создать Cargo Workspace (`crates/vpn-core`, `crates/vpn-sync`, `src-tauri`) + React/TypeScript/Tailwind фронтенд (`src/`). Настроить `tauri.conf.json` (sidecar `sing-box`, desktop tray, mobile init).
* **Definition of Done:** `cargo check --workspace` и `npm run build` проходят без ошибок.

### Фаза 1: Ядро `vpn-core` (Модели, Парсеры, Генератор `sing-box`)
* **Задача:**
  1. Реализовать `models.rs` и атомарное хранилище `storage.rs`.
  2. Написать парсеры `vless://` (обязательно с Reality + `xtls-rprx-vision`), `hy2://`, `trojan://`, `ss://` и декодер подписок (base64 + разбор заголовка `subscription-userinfo`).
  3. Написать `singbox/builder.rs`, генерирующий валидный конфиг `sing-box >= 1.14` (в режимах Desktop TUN, Desktop Proxy, Android TUN).
  4. Написать `sync_merge.rs` (LWW merge + tombstones).
* **Definition of Done:** Написаны и проходят `cargo test -p vpn-core`:
  * Юнит-тесты парсинга минимум 5 реальных форматов URI (включая битые строки и URL-кодирование).
  * Юнит-тесты `merge_states` (добавление, обновление, удаление через tombstone, конфликт таймстампов).
  * Сгенерированный JSON проходит проверку реальным бинарником `sing-box check -c test_config.json` с кодом возврата `0`.

### Фаза 2: Модуль `vpn-sync` (Zero-Cloud LAN Sync)
* **Задача:**
  1. Реализовать `crypto.rs` (`ChaCha20Poly1305` + AAD `sid` + `zeroize`).
  2. Реализовать `net_discovery.rs` (фильтр LAN IP).
  3. Реализовать `server.rs` (`axum`, endpoints `GET /health` и `POST /sync`, авто-таймаут TTL 180с, одноразовая сессия) и `client.rs` (параллельный probe по всем IP + обмен).
* **Definition of Done:** Интеграционный тест в `cargo test -p vpn-sync`, который поднимает реальный локальный сервер на `127.0.0.1`, подключается клиентом, успешно мержит два разных `AppState` и проверяет, что неверный ключ/подмена байта/повторный запрос с тем же `sid` отклоняются.

### Фаза 3: Desktop Backend (Linux EndeavourOS + Windows)
* **Задача:**
  1. Реализовать `desktop_linux.rs` (динамический Clash API порт, проверка `getcap` + `pkexec setcap`, graceful stop, чтение логов).
  2. Реализовать `desktop_windows.rs` (System Proxy через реестр + TUN с `wintun.dll`, `JobObject` для авто-убийства дочернего процесса).
  3. Подключить Clash API клиент (`clash_api.rs`): проверка задержки (ping) и поток статистики трафика (upload/download speed).
  4. Настроить системный трей (Подключить/Отключить, быстрый выбор профиля, Выход с остановкой туннеля).
* **Definition of Done:** На EndeavourOS клиент запускает `sing-box`, поднимает TUN (или Proxy), трафик идет через выбранную ноду, при закрытии приложения процесс `sing-box` корректно завершается.

### Фаза 4: Frontend UI (Минимализм + Sync Modal)
* **Задача:**
  1. Сделать чистый адаптивный интерфейс (одинаково удобный для десктопного окна `900x620` и экрана смартфона):
     * Главный экран: кнопка Connect/Disconnect, индикатор скорости (RX/TX), текущий сервер, переключатель режима (TUN / System Proxy на Desktop).
     * Вкладка подписок и нод: добавление по URL/буферу, обновление, массовый пинг с цветовой индикацией.
     * Вкладка правил: простые пользовательские правила (Домен -> Direct / Proxy / Reject) + пресеты (Обход РФ / Глобальный).
     * **Модальное окно «Синхронизация (LAN Sync)»:** две вкладки — *«Показать QR (Хост)»* (с таймером обратного отсчета и копированием текстового кода) и *«Подключиться (Клиент)»* (кнопка камеры на Android + поле вставки кода на ПК + сводка «Добавлено X подписок, Y правил»).
* **Definition of Done:** Полный цикл работы на Desktop без единого обращения к консоли: добавил подписку -> пропинговал -> подключился -> открыл Sync QR.

### Фаза 5: Android плагин (`tauri-plugin-vpn` + `libbox.aar`)
* **Задача:**
  1. Инициализировать Android-проект (`cargo tauri android init`).
  2. Реализовать `VpnPlugin.kt`, `CoreVpnService.kt` и `LibboxBridge.kt` с `addDisallowedApplication(packageName)` и `ForegroundService`.
  3. Подключить `tauri-plugin-barcode-scanner` и `network_security_config.xml`.
  4. Подключить `backend/android.rs` к `TunnelBackend`.
* **Definition of Done:** APK собирается (`cargo tauri android build`), ставится на Galaxy S25, сканирует QR-код с EndeavourOS по домашнему Wi-Fi, забирает подписки и правила за 1 секунду и успешно поднимает VPN через системный `VpnService`.

### Фаза 6: Полировка и E2E-тестирование
* **Задача:**
  1. Проверить синхронизацию при **уже включенном VPN** на обоих устройствах (тест исключений RFC1918).
  2. Проверить сценарий удаления подписки на ПК -> синхронизация -> подписка исчезает на телефоне (tombstone check).
  3. Проверить сборку и работу на Windows (System Proxy + TUN + принудительное закрытие через Диспетчер задач).
* **Definition of Done:** Все сценарии из Раздела 7 отработаны, утечек DNS и зависших процессов нет.