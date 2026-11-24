# Referência de módulos e funções

Lista das APIs públicas implementadas, por crate. (Assinaturas resumidas.)

## `elo-protocol`

`lib.rs`
- `type DeviceId = String`, `type PubKey = Vec<u8>`
- `enum Role { Host, Controller }`
- `enum ControlMsg { RegisterHost, Registered, Connect, Paired, Error, Register, Resolve, Resolved }`
- `const MAX_FRAME: usize = 16 MiB`
- `fn encode(&ControlMsg) -> Result<Vec<u8>, CodecError>` — frame com prefixo `u32` BE.
- `fn decode(&[u8]) -> Result<ControlMsg, CodecError>`
- `fn format_id(&DeviceId) -> String` — formata `123456789` como `123 456 789`.

`session.rs`
- `struct Permissions { control, file_transfer, clipboard, audio }` (+ `VIEW_ONLY`, `FULL`)
- `struct Rect`, `struct VideoFrame`, `struct MonitorInfo { index, name, x, y, width, height, primary, current }`
- `enum MouseButton`, `enum InputEvent { MouseMove, MouseButton, Scroll, Key, Char }`, `type TransferId = u32`
- `enum SessionMsg { … 27 variantes (v2) … }` (ver [protocolo](protocolo.md))
- `fn encode_msg(&SessionMsg) -> Vec<u8>`, `fn decode_msg(&[u8]) -> Result<SessionMsg, bincode::Error>`

## `elo-session`

- `struct Keypair { private: Vec<u8>, public: Vec<u8> }`
- `fn generate_keypair() -> Result<Keypair>` — par estático X25519.
- `struct EncryptedStream<S>`
  - `fn remote_static(&self) -> &[u8]` — chave estática do par (pinagem/fingerprint).
  - `async fn send(&mut self, &[u8]) -> Result<()>` — uma mensagem lógica (qualquer tamanho).
  - `async fn recv(&mut self) -> Result<Vec<u8>>`
  - `fn split(self) -> (EncryptedSender<S>, EncryptedReceiver<S>)` — leitura/escrita concorrentes.
- `struct EncryptedSender<S>` → `async fn send(&mut self, &[u8])`
- `struct EncryptedReceiver<S>` → `async fn recv(&mut self) -> Result<Vec<u8>>`, `fn remote_static(&self) -> &[u8]`
- `async fn handshake_initiator<S>(stream: S, &Keypair) -> Result<EncryptedStream<S>>` (controller)
- `async fn handshake_responder<S>(stream: S, &Keypair) -> Result<EncryptedStream<S>>` (host)

## `elo-relay`

- `async fn serve(listener: TcpListener) -> Result<()>` — aceita conexões para sempre; pareia hosts e
  controllers (cópia cega) e serve o **diretório** de IDs curtos (`Register`/`Resolve`).

## `elo-p2p`

Ver [p2p.md](p2p.md): `ALPN`, `PING_ALPN`, `Duplex`, `Node` (`bind_node`, `bind_node_ephemeral`,
`id_from_secret_file`, `accept`, `connect`, `ping`, `id`).

## `elo-client-core`

`lib.rs` — traits e mocks
- `trait Capture { width(); height(); async frame() -> Result<Vec<u8>>; monitors() -> Vec<MonitorInfo>; async select_monitor(u32) -> Result<()> }`
  (quadro BGRA, `width*height*4`; `monitors`/`select_monitor` têm implementação padrão: vazio / erro)
- `trait InputSink { inject(InputEvent) -> Result<()>; set_area(x, y, w, h); send_sas() -> Result<()> }`
  (`set_area` = retângulo do monitor capturado no desktop virtual; `send_sas` falha por padrão)
- `struct MockCapture` (padrão sintético) e `struct RecordingInput` (grava eventos) — usados nos testes.

`codec.rs` (tiles)
- `const TILE: u32 = 64`
- `struct Encoder` → `new(w,h)`, `force_keyframe()`, `encode(&[u8]) -> Result<VideoFrame>`
- `struct Decoder` → `new()`, `dims() -> (u32,u32)`, `apply(&VideoFrame) -> Result<&[u8]>`

`codec_h264.rs` (OpenH264)
- `fn available() -> bool` — testa uma ida e volta 64×64 (resultado em cache).
- `struct H264Encoder` → `new(width, height, fps, bitrate_kbps)`, `encode(&bgra, force_key) -> Result<(Vec<u8>, bool)>`
  (Annex-B + "é key"; bytes vazios = frame pulado).
- `struct H264Decoder` → `new()`, `decode(&[u8]) -> Result<Option<(w, h, bgra)>>` (tamanho par).

`audio.rs`
- `fn available() -> bool` (Windows: há dispositivo de saída padrão; outros SOs: `false`)
- `struct AudioCapture` → `start(Sender<(rate, channels, pcm)>) -> Result<Self>` (loopback, mono 24 kHz, 480 amostras/bloco)
- `struct AudioPlayer` → `new()`, `push(rate, channels, &pcm)`.

`clipboard.rs`
- `struct ClipboardBridge` → `start(Sender<String>) -> Option<Self>` (thread que verifica a área de
  transferência a cada 500 ms; limite de 1 MiB), `set(String)`.

`totp.rs`
- `PERIOD = 30`; `generate_secret()`, `secret_to_base32()`, `secret_from_base32()`, `code_at(secret, unix)`,
  `now()`, `verify(secret, code, unix)` (±1 passo), `otpauth_uri(secret, account, issuer)`.

`transfer.rs`
- `const CHUNK = 60 KiB`
- `struct IncomingFile` → `new(id,name,size)`, `push(offset,&[u8])`, `is_complete()`, `finish() -> Result<Vec<u8>>`
- `fn chunks(&[u8]) -> impl Iterator<Item=(u64,&[u8])>`

`discovery.rs` (descoberta na LAN por broadcast UDP)
- `const DISCOVERY_PORT: u16 = 9001` (separada da `DIRECT_PORT = 9000`)
- `struct DiscoveredDevice { name, ip, port, short_id }`
- `fn start(name, short_id, port) -> Announcer` — anuncia este dispositivo a cada ~2 s, por **todas** as interfaces
  IPv4 ativas (função interna `announce_once`, usando o broadcast dirigido de cada sub-rede — ex.: `192.168.0.255` —
  mais `255.255.255.255` como reforço, na porta 9001); a thread para no `drop` do `Announcer`. Usa o crate `if-addrs`.
- `fn listen() -> io::Result<DiscoveryHandle>` — escuta anúncios; `DiscoveryHandle` é `Clone`.
- `DiscoveryHandle::devices() -> Vec<DiscoveredDevice>` — lista atual (poda entradas > 10 s).

`rendezvous.rs`
- `async fn register_host(relay, device_id, pubkey) -> Result<(TcpStream, Vec<u8>)>`
- `async fn connect_controller(relay, target_id, pubkey) -> Result<(TcpStream, Vec<u8>)>`
- `async fn register_directory(dir, device_id, node_id) -> Result<TcpStream>` — mantenha o stream aberto.
- `async fn resolve_directory(dir, device_id) -> Result<String>` — ID curto → node id.
- `fn auth_proof(salt, password) -> Vec<u8>` — `SHA256(salt‖senha)`

`host.rs`
- `struct PeerInfo { name, fingerprint }`; `enum HostEvent { Connected, Disconnected, AuthFailed, Pointer, Chat, FileReceived }`
- `struct HostSessionHandle` → `send_chat(&str)`, `disconnect()` (controlar a sessão pela UI do host).
- `struct HostLink` (Clone) → `send_file(name: String, data: Vec<u8>)`, `send_clipboard(text: String)` —
  envia do host para o controlador conectado (fire-and-forget; arquivo em chunks de 60 KiB).
- `struct HostOptions { password, open_access, trusted, totp_secret, approver, on_event, permissions, fps,
  download_dir, prefer_h264, show_pointer, link_out }` + `HostOptions::with_password(...)`.
  - `link_out: Option<Arc<Mutex<Option<HostLink>>>>` — receptáculo do `HostLink` da sessão ativa
    (o host grava `Some(link)` ao conectar e `None` ao desconectar; a UI do host lê daqui para enviar
    arquivo/clipboard ao controlador).
- `type ApproveFn` (callback assíncrono "permitir?"), `type EventFn`.
- `async fn run_host(HostConfig, capture, input)` — via relay (só senha).
- `async fn run_host_direct(DirectHostConfig, capture, input)` — um controller direto, sem relay.
- `async fn serve_host_on(stream, &keypair, password, permissions, fps, download_dir, capture, input)` — só senha.
- `async fn serve_host_opts(stream, &keypair, HostOptions, make_backend)` — **entrada completa**: handshake
  (20 s), `Hello`, decisão de acesso, 2FA, `Caps`, cria o backend (`make_backend`) só depois de aceitar, e roda a sessão.
  Limite de falhas: 5 em 60 s.

`controller.rs`
- `const ERR_2FA_REQUIRED = "2FA_REQUIRED"`
- `struct ConnectOptions { name: Option<String>, totp_code: Option<String>, raw_media: bool }` — com `raw_media` o
  controller anuncia `Caps { h264: true, audio: true }` e **não** decodifica/toca/aplica: entrega `FrameH264`, `Audio`
  e `Clipboard` crus (app Android) e não inicia o `ClipboardBridge`.
- `struct ControllerConfig { relay, target_id, keypair, password, download_dir }`, `DirectControllerConfig { host, keypair, password, download_dir }`
- `enum ControllerEvent { Frame{width,height,bgra}, ScreenChanged{width,height}, Monitors(Vec<MonitorInfo>), Chat(String), FileReceived{path}, FrameH264{width,height,key,data}, Audio{rate,channels,pcm}, Clipboard(String), Disconnected }`
  — `FrameH264`/`Audio`/`Clipboard` só com `raw_media` (`Audio` não chega com o som desligado).
- `struct ControllerHandle { screen, permissions, peer_fingerprint, … }`
  - `send_input(InputEvent)`, `send_chat(&str)`, `send_pointer(x, y)`, `select_monitor(index)`, `send_sas()`,
    `send_file(id, name, &[u8])`, `set_audio(bool)`/`audio()` (silenciar localmente), `send_clipboard(&str)` (≤ 1 MiB),
    `close()` — encerra a sessão (fecha o envio; o stream de eventos termina com `Disconnected`). Soltar **todas** as
    cópias do handle também encerra a sessão.
- `run_controller(cfg)` — via relay (sem opções); `run_controller_direct(cfg)` / `run_controller_direct_opts(cfg, opts)` —
  direto por IP (timeout de 10 s); `connect_on(stream, &keypair, password, download_dir)` /
  `connect_on_opts(..., opts)` — sobre qualquer stream pronto (P2P/QUIC).
- Se `opts.name` vazio, o nome enviado em `Hello` é `$USERNAME`/`$USER`/`"Elo"`.

`platform.rs` (`#[cfg(windows)]`)
- `struct DxgiCapture` → `new() -> Result<Self>`; implementa `Capture` (inclui `monitors`/`select_monitor`).
- `struct SendInputSink` → `new(width, height)`; implementa `InputSink` (inclui `Char` e `send_sas`).
- `fn ensure_input_desktop() -> bool` — anexa a thread ao desktop de entrada atual.

## `elo-client` (CLI)

Subcomandos (um binário faz tudo):
- `elo-client relay [--bind 0.0.0.0:9000]` — roda o relay/diretório embutido.
- `elo-client host --relay HOST:PORT [--id N] --password PW` — host via relay (Windows).
- `elo-client host --listen 0.0.0.0:9000 --password PW` — host direto, sem relay (Windows).
- `elo-client host --p2p --password PW` — host P2P pela internet (Windows); imprime o endereço Elo.
  Guarda o segredo do nó em `./.elo-cli/node.key`. Só aceita por senha (sem aprovação/2FA/pareados).
- `elo-client connect --relay HOST:PORT --id N [--password PW]` — controller via relay.
- `elo-client connect --direct HOST:PORT [--password PW]` — controller direto por IP.
- `elo-client connect --p2p ENDEREÇO|ID9 [--directory HOST:PORT] [--password PW] [--code 2FA]` — controller P2P;
  um número de 9 dígitos exige `--directory`. **`--password` é opcional**: sem ele (senha vazia) o host recebe
  um **pedido de aprovação**. `--code` envia o código 2FA.
- O controller da CLI é headless: imprime estatísticas e repassa linhas do stdin como chat.

## `elo-gui` (o produto, `elo.exe`)

Ver os módulos em [arquitetura.md](arquitetura.md). Principais itens públicos:
- `config::Config` (campos em [acesso-nao-supervisionado.md](acesso-nao-supervisionado.md); inclui
  `close_to_tray: bool` — fechar a janela esconde na bandeja em vez de sair, padrão `true`), `Favorite`, `TrustedDevice`,
  `Recent { name, address, lan, password }` (e `Config.recents: Vec<Recent>` — últimos acessos, mais recente
  primeiro), `config_dir()`, `service_dir()`, `seed_service_dir()`, `set_autostart(bool)`.
- `hostrun::{HostSettings, HostRun, run, load_or_create_keypair, spawn_directory, DIRECT_PORT = 9000}`.
- `worker::{run_headless(state_dir), run_worker()}`; `ipc::{Msg, PORT = 47199, token(dir), spawn_gui_server, WorkerLink}`.
- `service::{is_installed, install, uninstall, elevate, run_dispatcher}` (Windows).
- `install::{is_installed, installed_exe, install_dir, elevate_install, elevate_uninstall}` (instalação "um clique" do
  app, distinta do serviço; `run_install`/`run_uninstall` rodam nos modos elevados `--install-app`/`--uninstall-app`).
  No Windows copia o exe para `C:\Program Files\Elo`, cria a tarefa agendada `Elo` (início no logon, admin, `--tray`) e o
  atalho no Menu Iniciar; fora do Windows tudo é no-op/stub. `const INSTALL_DIR_DISPLAY = r"C:\Program Files\Elo"`.
- `logging::{init, log_dir, recent, clear_view}`.
- `tray::{start, Tray, TrayEvent}` (Windows; stubs no-op nos demais SOs): `start(tooltip: &str, wake: egui::Context) -> io::Result<(Tray, Receiver<TrayEvent>)>`
  cria o ícone na bandeja e o canal de eventos; a thread chama `wake.request_repaint()` ao emitir um evento (para o `update()`
  processar "Abrir"/"Sair" com a janela escondida); `enum TrayEvent { Open, Quit }`; `Tray` remove o ícone e encerra a thread no `Drop`.
- `ui_manual::manual_page(ui: &mut egui::Ui, pal: &Palette)` — renderiza a página **Manual**.
- `App::{start_host, start_connect, start_connect_fav, save_favorite, save_config, add_favorite_unique}`; `ui_session::drive_session`.
  `add_favorite_unique(name, address, lan, password) -> bool` adiciona um recente aos favoritos sem duplicar por `address`+`lan`.

## `elo-android` (biblioteca do app Android)

API exposta ao Kotlin via UniFFI (pacote `io.github.ricardomendes95.elo.ffi`); descrição completa em
[android.md](android.md#api-do-núcleo-elo-android-via-uniffi).

- `Elo::new(data_dir) -> Arc<Elo>`; `device_id()`, `connect(target, lan, password, name, totp_code, directory,
  download_dir, listener) -> Arc<Session>` (erro `EloError::TwoFaRequired` / `EloError::Failed { msg }`),
  `is_online(target, lan, directory)` — bloqueantes.
- `Session`: `info()`, `mouse_move`, `mouse_button(u8, bool)`, `scroll`, `key(u16, bool)`, `char(u32)`, `text`,
  `pointer`, `chat`, `select_monitor`, `send_sas`, `set_audio`, `send_clipboard`, `send_file`, `hangup` — não bloqueantes,
  enfileirados em ordem numa tarefa.
- `trait SessionListener` (implementado em Kotlin): `on_video_h264`, `on_video_bgra`, `on_screen_changed`,
  `on_monitors`, `on_audio`, `on_clipboard`, `on_chat`, `on_file_received`, `on_disconnected`.
- `const DIRECT_PORT = 9000`.

`elo-session`: `EncryptedSender::shutdown()` fecha a direção de envio (FIN do TCP / fim do stream QUIC).

`elo-client-core/examples/mock_host.rs`: host de teste com `MockCapture` (1280×720) em `0.0.0.0:9000`, senha `elo`,
aprovação automática; imprime input/chat recebidos.

