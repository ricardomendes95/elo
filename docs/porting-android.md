# Porte para Android (APK)

Objetivo: um app Android que seja **controller** (ver e controlar um host Windows)
e, opcionalmente, **host** (compartilhar a tela do Android). Há dois caminhos.

## Caminho A (recomendado): reaproveitar o núcleo Rust via NDK

O protocolo, a cripto (Noise), o codec e o P2P (iroh) já estão prontos e testados
em Rust e **compilam para Android** (iroh e tokio suportam Android). Em vez de
reimplementar tudo em Kotlin, compile os crates como biblioteca nativa e exponha
uma API fina para o Kotlin. É menos código, sem divergência de protocolo.

### Estrutura

1. Novo crate `elo-android` (`crate-type = ["cdylib"]`) que depende de
   `elo-client-core`, `elo-session`, `elo-protocol`, `elo-p2p`.
2. Exponha uma API via **UniFFI** (gera bindings Kotlin) ou JNI manual. Funções
   sugeridas:
   - `fn start() ` — inicializa uma runtime Tokio global (ex.: `tokio::runtime::Runtime` em `OnceCell`).
   - `fn p2p_bind(secret_path) -> NodeHandle` e `node_id() -> String`.
   - `fn connect_p2p(node, password, name, totp_code) -> SessionHandle` / `connect_direct(ip, password, name, totp_code) -> SessionHandle`
     (internamente: `elo_p2p::bind_node_ephemeral` + `Node::connect` + `controller::connect_on_opts` com
     `ConnectOptions { name, totp_code }`, ou `run_controller_direct_opts`). Senha vazia = pedido de aprovação.
     Se o erro for `controller::ERR_2FA_REQUIRED` (`"2FA_REQUIRED"`), peça o código ao usuário e **reconecte** com ele.
   - `fn poll_frame(session) -> Option<Frame{width,height,bgra}>` — drena `ControllerEvent::Frame`
     de um canal; o Kotlin desenha num `Surface`/`Bitmap`. (Veja "Vídeo" abaixo para a opção de decodificar H.264 no Kotlin.)
   - `fn send_pointer(session, x, y, down)` / `send_key(session, vk, down)` / `send_char(session, codepoint)` /
     `send_chat(session, text)` / `select_monitor(session, index)` / `send_sas(session)` / `set_audio(session, on)`.
   - Eventos úteis: `ScreenChanged`, `Monitors`, `Chat`, `FileReceived`, `Disconnected`.
3. Build: **cargo-ndk**
   ```sh
   cargo install cargo-ndk
   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
   cargo ndk -t arm64-v8a -t armeabi-v7a -o app/src/main/jniLibs build --release -p elo-android
   ```
4. UI Kotlin (Jetpack Compose ou Views): tela inicial igual à do desktop (endereço
   + senha + conectar; "este dispositivo" com o endereço Elo), e a tela de sessão
   renderizando os frames e capturando toque/teclado.

> **Atenção (não verificado):** `elo-client-core` depende de `arboard` (área de transferência) e `openh264`
> (compilado do código-fonte), e o áudio (`cpal`) só é compilado no Windows. Compilar o núcleo para Android pode exigir
> *feature flags*/`cfg` para desligar `arboard` e, conforme o caso, `openh264`; ninguém compilou isso para Android ainda.
> O áudio já é um stub fora do Windows (`audio::available() == false`), então no Android o som deve ser tocado pelo
> Kotlin (`AudioTrack`) a partir das mensagens `Audio`, o que exige expor esses eventos na API (hoje o núcleo só os toca pelo `AudioPlayer`). O mesmo vale para o `MediaCodec`: o `ControllerEvent` atual só entrega BGRA já decodificado, **não** expõe o `FrameH264` cru; usar o `MediaCodec` exige acrescentar um evento com os bytes Annex-B (mudança no núcleo) ou implementar o controller em Kotlin (Caminho B).

### Controller no Android (o caso principal)

- **Vídeo**: há duas opções, e o host decide pelo `Caps` que o controller anuncia:
  - **Via núcleo Rust (mais simples):** `elo-client-core` decodifica tanto H.264 (OpenH264) quanto o codec de
    tiles e entrega BGRA `w*h*4` em `ControllerEvent::Frame`. Converta para `Bitmap` (`ARGB_8888`) — atenção à
    ordem de canais (BGRA→ARGB) — e desenhe num `ImageView`/`Canvas`/`Surface`, escalando para caber. Cuidado:
    o OpenH264 é compilado do código-fonte (precisa de um toolchain C/C++ no NDK) e a decodificação é por CPU.
  - **Decodificação nativa com `MediaCodec` (recomendada para desempenho):** o fluxo de `FrameH264` é **H.264 em
    Annex-B** (NAL units com start code `00 00 00 01`, um frame por mensagem, `key = true` nos IDR). O `MediaCodec`
    (`video/avc`) aceita exatamente esse formato: configure o decoder com um `Surface` e alimente `queueInputBuffer`
    com `FrameH264.data`; o OpenH264 normalmente emite SPS/PPS junto do IDR (não verificado no código — confirme com um dump do fluxo e, se faltar, extraia SPS/PPS do primeiro keyframe para o `csd-0/csd-1`). Se o dispositivo anunciar `Caps { h264: true }` o host
    manda H.264; para forçar o codec de tiles, anuncie `h264: false`. O tamanho decodificado é **par**
    (largura/altura ímpares do host são acrescidas de 1 px no fluxo; corte para o tamanho do `ScreenInfo`/`FrameH264.width/height`, como faz o núcleo em `controller::crop_to`).
    Para o vídeo H.264 não é preciso a conversão de cor do Rust (o `MediaCodec` faz YUV→RGB na GPU).
  - Se o app **não** decodificar H.264, anuncie `h264: false` e implemente o codec de tiles (zlib; ver [protocolo.md](protocolo.md), 3b).
- **Áudio**: mensagens `Audio { rate: 24000, channels: 1, pcm }` com **PCM s16le mono 24 kHz**, blocos de 20 ms
  (960 bytes). Reproduza com **`AudioTrack`** em modo `MODE_STREAM` (`ENCODING_PCM_16BIT`, `CHANNEL_OUT_MONO`,
  `sampleRate = rate`), escrevendo cada bloco; mantenha um buffer curto (~250 ms, como o desktop) para não acumular
  latência. Anuncie `Caps { audio: true }` só se for tocar; o botão de mudo é local. (O áudio é capturado só pelo host
  Windows; num host Android não há captura implementada.)
- **Área de transferência**: `Clipboard(String)` nos dois sentidos (texto UTF-8, ≤ 1 MiB) — use o `ClipboardManager`
  (só se `permissions.clipboard`).
- **2FA (TOTP)**: se o host pedir, o app pergunta o código de 6 dígitos (RFC 6238; o usuário o obtém num app
  autenticador). O app Android só **envia** o código (`TwoFaSubmit`); não precisa implementar TOTP, a não ser que queira ser ele
  mesmo o **host** com 2FA (então: HMAC-SHA1, passo de 30 s, 6 dígitos, ±1 passo, segredo base32 — ver [protocolo.md](protocolo.md), 5b).
- **Multi-monitor**: o host envia `Monitors { list }`; mostre um seletor e envie `SelectMonitor { index }`. Ao trocar,
  chega um novo `ScreenInfo` (e o vídeo recomeça com keyframe). O `ScreenInfo` também chega quando a resolução do host muda
  no meio da sessão.
- **Ctrl+Alt+Del**: botão que envia `SendSas` (só funciona se o host Windows roda o serviço e há controle).
- **Pedido de aprovação**: envie senha vazia para o host mostrar "Permitir/Negar" (o app deve esperar sem limite curto no
  laço pós-`AuthSubmit`; o dono tem até 120 s).
- **Nome**: envie o nome do aparelho no `Hello { name }` (≤ 64 caracteres).
- **Mouse por toque**: mapeie o toque na área da imagem para pixels absolutos do
  host (igual ao `to_host` do desktop): `x = (tx-rect.left)/rect.w * (sw-1)`, onde `sw` é a largura do **monitor
  capturado** (coordenadas são relativas a ele). Sem permissão de controle, envie `Pointer { x, y }` (o host mostra o ponteiro).
  Toque = `MouseMove` + `MouseButton{Left,down}`. Arraste = `MouseMove` contínuos.
- **Teclado**: use o teclado virtual. Teclas **ASCII/de controle** (letras, números, Enter, Backspace, setas…) vão como
  `InputEvent::Key { code, down }` com **códigos VK do Windows** (a tabela de `crates/gui/src/keymap.rs`). Texto que o
  VK não expressa — **o que o teclado virtual do Android digitar fora do ASCII** (ç, á, ñ, €, emojis…) — vai como
  `InputEvent::Char { code: u32 }` (**escalar Unicode**, índice 4 do `InputEvent`); o host Windows digita o caractere com
  `SendInput` + `KEYEVENTF_UNICODE`, independente do layout. Para texto vindo do IME (`InputConnection.commitText`), envie
  um `Char` por *code point* (não por unidade UTF-16; um emoji é um único `code`). Modificadores (Shift/Ctrl/Alt) por
  diferença de estado.
- **Rolagem**: gesto de scroll → `Scroll{dx,dy}`.

### Host no Android (opcional, com limites)

- **Captura de tela**: `MediaProjection` (`MediaProjectionManager`) → um
  `VirtualDisplay` com um `ImageReader`; cada `Image` vira um buffer; converta para
  BGRA e implemente o trait `Capture` do lado Rust (ou empurre os frames via JNI
  para um `Capture` que lê de um canal).
- **Injeção de input**: Android **não** tem um `SendInput` geral. Opções:
  - **AccessibilityService** + `dispatchGesture` (toques/arrastes) — exige o usuário
    habilitar o serviço de acessibilidade; não injeta teclas arbitrárias facilmente.
  - **Root** / APK de sistema (`InputManager.injectInputEvent` via reflection) — só em
    dispositivos com root; não é para loja.
  - Realisticamente: no Android o papel de host costuma ficar **somente-visualização**
    (sem controle), a menos que haja acessibilidade/root.
- **Permissões/serviço**: rode a captura num `ForegroundService` com notificação;
  `MediaProjection` exige consentimento por sessão.

### Permissões (AndroidManifest)
`INTERNET`, `ACCESS_NETWORK_STATE`, `FOREGROUND_SERVICE` (+ `FOREGROUND_SERVICE_MEDIA_PROJECTION`
no host), e o fluxo de consentimento do `MediaProjection`. P2P (iroh) só precisa de
`INTERNET`.

## Ordem do handshake de sessão (v2) — resumo para o app

Detalhes completos em [protocolo.md](protocolo.md) (seção 5):

1. Transporte pronto (QUIC/iroh ou TCP) → **Noise XX** como iniciador.
2. Enviar `Hello { name }`.
3. Ler `AuthRequired { salt }`; enviar `AuthSubmit { proof = SHA256(salt ‖ senha) }`.
4. Laço de leitura: `TwoFaRequired` → enviar `TwoFaSubmit { code }` (ou abortar com `2FA_REQUIRED` e pedir o código);
   `AuthOk` → seguir; `AuthFailed` → erro.
5. Enviar `Caps { h264, audio }`.
6. Ler `Permissions`, depois `ScreenInfo`; depois chegam `Monitors`, `Frame`/`FrameH264`, `Audio`, `Clipboard`, `Chat`…
7. Enviar `Ping` a cada 15 s.

## O que **não** vai para o Android

- **IPC GUI ↔ worker** (`127.0.0.1:47199`), o **serviço do Windows**, o supervisor/worker na sessão do console,
  `SendSAS` local, captura DXGI e `SendInput` são **só desktop (Windows)**. Em Android o app é controller; nada disso é necessário.
  (`send_sas` é uma *mensagem* que o controller manda ao host; o Android pode enviá-la normalmente.)
- Funções de GUI (`elo-gui`) não são reaproveitadas: o app Android tem a própria UI.

## Caminho B: reimplementar o protocolo em Kotlin/Java

Mais trabalho e risco de divergência; só vale se não quiser Rust no APK. Você teria
que reimplementar, **exatamente** conforme [protocolo.md](protocolo.md):

- o framing (prefixos `u32`/`u16` com a **ordem de bytes** indicada);
- a serialização **bincode** (inteiros LE, tag de enum `u32` LE, `Vec`/`String` com
  comprimento `u64` LE, arrays fixos crus);
- o handshake **Noise XX** (`Noise_XX_25519_ChaChaPoly_BLAKE2s`) — há libs Noise em
  Java (ex.: Noise-Java);
- o codec tile-delta + **zlib** (a `Inflater`/`Deflater` do Java serve) sobre BGRA, e/ou o fluxo H.264
  (`MediaCodec` decodifica o Annex-B nativamente — é aqui que o Caminho B ganha mais);
- o PCM de áudio (`AudioTrack`), o TOTP (se for host) e as variantes v2 (`Hello`, `Caps`, `Monitors`, `Clipboard`…);
- a prova `SHA256(salt‖senha)`.

Para P2P puro em Kotlin não há equivalente fácil ao iroh; usaria o modo relay/direto
(TCP) ou embutiria o iroh nativo mesmo (o que recai no Caminho A). **Por isso o
Caminho A é recomendado.**

## O que a documentação garante

- [protocolo.md](protocolo.md) tem o **formato binário exato** (framing, bincode,
  índices de variantes 0..26 de `SessionMsg` e 0..4 de `InputEvent`, exemplos de bytes, ordem do handshake, Noise,
  prova de senha, TOTP) — suficiente para reimplementar o fio.
- [referencia-funcoes.md](referencia-funcoes.md) lista as funções do núcleo que o
  `elo-android` chamaria no Caminho A.
- [arquitetura.md](arquitetura.md) e [p2p.md](p2p.md) descrevem o fluxo e o transporte.
