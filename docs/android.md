# App Android (controller)

O app Android do Elo (`io.github.ricardomendes95.elo`, versão `0.3.0`) **vê e controla** um computador que roda o Elo
(`elo.exe`). Ele reaproveita o núcleo Rust (protocolo, Noise, P2P/iroh) compilado com o NDK — o "Caminho A" de
[porting-android.md](porting-android.md) — e a interface é Kotlin + Jetpack Compose.

O papel de **host** (compartilhar a tela do celular, para um PC ver e controlar) existe desde a 0.3.0; ver [Host no Android](#host-no-android).

## Estrutura

```
crates/android/                  crate `elo-android` (cdylib libelo_android.so), API exposta via UniFFI
  src/lib.rs                     Elo (runtime Tokio + nó P2P), Session, SessionListener, EloError
  src/bin/uniffi-bindgen.rs      gerador dos bindings Kotlin (feature `bindgen`)
  uniffi.toml                    pacote Kotlin: io.github.ricardomendes95.elo.ffi
android/                         projeto Gradle (AGP 8.11.1, Kotlin 2.2.0, Compose BOM 2025.06.01, Gradle 8.14.3)
  build-rust.sh                  cargo ndk → app/src/main/jniLibs/<abi>/libelo_android.so + bindings em .../elo/ffi/
  app/src/main/java/io/github/ricardomendes95/elo/
    MainActivity.kt              inicia EloCore e Prefs; pede POST_NOTIFICATIONS (Android 13+)
    core/EloCore.kt              ponte UI ↔ núcleo: connect/deviceId/isOnline, ActiveSession (flows), VideoSink
    data/Prefs.kt                preferências (SharedPreferences + JSON): favoritos, recentes, nome, diretório, tema, sessão
    ui/EloApp.kt                 NavHost + barra inferior (Início · Favoritos · Configurações); rota `session`
    ui/Connector.kt              conexão em andamento (compartilhada por Início e Favoritos), 2FA, cancelar
    ui/home, ui/favorites, ui/settings, ui/theme   telas e tema
    ui/session/                  SessionScreen, VideoDecoder (MediaCodec), AudioOut (AudioTrack),
                                 RemoteKeyboard + KeyCaptureView (IME), SessionGestures, Keymap (VK)
```

`app/src/main/jniLibs/` e `.../elo/ffi/` são **gerados** por `build-rust.sh` (estão no `.gitignore`).

## Build

Pré-requisitos (versões usadas): Rust estável com os targets `aarch64-linux-android` (e `x86_64-linux-android` para
o emulador), `cargo-ndk`, Android SDK com platform 36, **NDK 28.2.13676358** e **JDK 17**.

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
cd android
export JAVA_HOME=/caminho/do/jdk-17
./gradlew assembleDebug        # roda build-rust.sh (tarefa buildRust, ligada ao preBuild) e empacota
# -> app/build/outputs/apk/debug/app-debug.apk
```

- `-PskipRust` pula a compilação do Rust (usa o `.so` e os bindings já gerados).
- `-PeloAbis=arm64-v8a,x86_64` inclui a ABI do emulador (padrão: só `arm64-v8a`, celulares reais).
- `local.properties` aponta o SDK (`sdk.dir=...`); `build-rust.sh` usa `ANDROID_HOME`/`ANDROID_NDK_HOME` ou, na falta,
  `~/Android/Sdk` e o NDK mais novo instalado.
- O `.so` é compilado em `release` (`ELO_PROFILE=debug` muda) e o núcleo vai **sem** `arboard` e sem OpenH264
  (`elo-client-core` com `default-features = false`).
- minSdk 26, target/compileSdk 36. O APK de debug tem ~44 MB (o `.so` arm64 tem ~23 MB antes de o AGP remover os símbolos).

## Telas

| Tela | O que tem |
|------|-----------|
| **Início** | "Conectar a outro computador": **Internet / Rede local**, endereço (endereço Elo, ID de 9 dígitos ou IP), senha opcional ("sem senha, o outro computador recebe um pedido para aprovar"), **Conectar**, estrela para salvar como favorito; estado "Conectando…" com Cancelar; erro em cartão. **Dispositivos na rede** (descoberta na LAN; toque conecta pelo `IP:porta` anunciado; botão **Atualizar** reinicia a varredura — `stop()`/`start()` do `Discovery`). "**Este dispositivo**": nome do aparelho **editável** aqui (o nome que o outro lado vê, `Prefs.deviceName`), e o endereço Elo (P2P) com **Copiar** e **Compartilhar**. **Recentes** (últimos 8) com **presença** (bolinha online), toque para reconectar (senha salva entra direto; sem ela, preenche o endereço para digitar a senha), **Renomear** (mantém o endereço), **Adicionar aos favoritos** (`Prefs.addFavorite`, sem duplicar) e **Remover** — paridade com os favoritos. |
| **Favoritos** | Cartões com nome, endereço, Internet/LAN e **presença** (online/offline/verificando, a cada 45 s com a tela aberta); toque = conectar com a senha salva; Editar/Remover; botão "+" para criar. |
| **Configurações** | *Geral*: nome deste aparelho (vai no `Hello`), tema (Escuro/Claro/Sistema). *Conexão*: servidor de diretório `host:porta` (ID curto), porta LAN 9000 (informativa). *Sessão*: modo de toque padrão (Direto/Touchpad), som ao entrar, manter a tela ligada. *Sobre*: versão e pasta dos arquivos recebidos. |
| **Sessão** | Vídeo em tela cheia (modo imersivo), alça no topo com o nome do computador (e "só visualização" quando não há permissão de controle) que abre/fecha a **barra de ações**. |
| **2FA** | Se o host pedir, diálogo com o código de 6 dígitos; o app reconecta com o código (`TwoFaSubmit`). |

### Barra de ações da sessão

Teclado · Teclas especiais · Modo de toque (Direto ⇄ Touchpad) · Monitor (só com 2+ monitores; envia `SelectMonitor`) ·
Ctrl+Alt+Del (`SendSas`; requer o serviço do Elo no Windows) · Som (liga/desliga localmente) · Enviar área de
transferência (texto local → `Clipboard`) · Chat (folha inferior, com contador de não lidas) · Enviar arquivo (seletor
do sistema; até 64 MB, vai para a pasta de downloads do host) · Informações (tamanho, permissões, impressão digital do
host, resumo dos gestos) · Desconectar (com confirmação; o botão Voltar também pergunta).

Botões que dependem de permissão do host (controle, arquivos, área de transferência, som) ficam desabilitados sem ela.

### Gestos

| Gesto | Modo Direto | Modo Touchpad |
|-------|-------------|---------------|
| Toque | clique esquerdo no ponto tocado | clique esquerdo onde está o cursor |
| Dois dedos tocando / pressionar (~0,5 s) | clique direito (com vibração) | idem, no cursor |
| Arrastar um dedo | arrastar (botão esquerdo pressionado do início ao fim) | move o cursor (relativo, ganho 1,4×) |
| Tocar e logo pressionar-arrastar | — | arrastar com o botão esquerdo |
| Dois dedos deslizando | rolagem (1 "clique" de roda a cada 48 dp; vertical e horizontal) | idem |
| Pinça | zoom local da imagem (1× a 5×), com deslocamento | idem; com zoom, a imagem acompanha o cursor |

Coordenadas: o ponto tocado é convertido para pixels do **monitor capturado** do host
(`x = local/largura_exibida × (largura_host − 1)`), descontando o zoom/deslocamento. Sem permissão de controle, o
app manda `Pointer { x, y }` (o host mostra o ponteiro) em vez de input. No Touchpad o app desenha um cursor local
(anel teal).

### Teclado

- A tecla Teclado mostra o IME do Android sobre uma `View` invisível (`KeyCaptureView`) cujo `InputConnection` usa
  `TYPE_TEXT_VARIATION_VISIBLE_PASSWORD` + sem sugestões, para o teclado mandar cada caractere na hora.
- Texto → um `InputEvent::Char` por **code point** (ç, á, €, emojis…). Se o IME compuser texto, a diferença vira
  Backspace + caracteres.
- Enter, Backspace, Tab, setas, F1–F12 etc. (IME ou teclado físico) → `InputEvent::Key` com **VK do Windows**
  (tabela em `ui/session/Keymap.kt`, igual à de `crates/gui/src/keymap.rs`). Ctrl/Alt/Win de um teclado físico também
  vão como VK.
- **Teclas especiais**: Ctrl, Alt, Shift, Win (1º toque = vale para a próxima tecla, 2º = travado 🔒, 3º = solta), Esc,
  Tab, setas, Home, End, PgUp, PgDn, Del, Ins, ⌫, Enter, PrtSc, **Ctrl+Shift+Esc**, **Alt+Tab**, **Win+D** e F1–F12.
  Com um modificador ativo, letras/dígitos digitados vão como VK (para atalhos como Ctrl+C), não como `Char`.

### Vídeo, áudio e área de transferência

- O app conecta com `ConnectOptions { raw_media: true }`: anuncia `Caps { h264: true, audio: true }` e recebe o
  **H.264 Annex-B cru** (`on_video_h264`). O `VideoDecoder` configura um `MediaCodec` `video/avc` no primeiro
  keyframe (SPS/PPS também em `csd-0`/`csd-1`, `KEY_LOW_LATENCY` no Android 11+), decodifica numa thread própria direto
  no `Surface` de uma `TextureView` e se reconfigura quando o tamanho muda (troca de monitor/resolução). Se a fila
  passar de 60 quadros, descarta até o próximo keyframe. O host força um keyframe a cada 3 s, então o vídeo volta
  sozinho em até 3 s depois de o app voltar do segundo plano.
- Se o host não tiver H.264, chegam quadros BGRA do codec de tiles (`on_video_bgra`), desenhados com `lockCanvas`.
- Áudio: `AudioOut` toca o PCM s16le (hoje 24 kHz mono) num `AudioTrack` `MODE_STREAM`, escrita sem bloqueio,
  descartando blocos quando há mais de ~250 ms pendentes.
- Área de transferência (só **texto**): o que é copiado no host chega por `on_clipboard`, vai para o
  `ClipboardManager` do Android e mostra "Copiado para a área de transferência.". No sentido contrário o envio é
  **manual** — o botão **Enviar área de transferência ao PC** (ícone de upload) lê o clipboard do celular e manda
  um `Clipboard` ao host —, porque o Android 10+ não deixa apps em segundo plano lerem a área de transferência.
- Arquivos enviados **pelo host** chegam por `on_file_received` (chunks de 60 KiB, remontados em memória) e são
  gravados em `Android/data/io.github.ricardomendes95.elo/files/Recebidos/`.

### Conexão

- **Rede local**: `IP` ou `IP:porta` (padrão 9000), TCP direto (`run_controller_direct_opts`).
- **Internet**: endereço Elo (node id do iroh) ou ID curto de 9 dígitos resolvido no **servidor de diretório**
  configurado (`resolve_directory`), e então `Node::connect` + `connect_on_opts`.
- O nó P2P do aparelho usa uma chave persistente em `files/p2p.key` (o endereço mostrado em "Este dispositivo"); a
  chave Noise da sessão é nova a cada conexão.
- Senha vazia = pedido de aprovação (a chamada espera o dono do host decidir, até ~2 min).
- Recentes guardam **nome e senha**: `Prefs.addRecent` preserva o nome renomeado pelo usuário e a senha já salva de uma
  entrada existente ao reconectar (uma reconexão não sobrescreve mais o nome pelo endereço).
- Presença dos favoritos: LAN = TCP na porta; ID curto = registrado no diretório; endereço Elo = `Node::ping`.
- **Dispositivos na rede** (descoberta na LAN): a tela Início escuta o broadcast UDP da descoberta
  (`elo_client_core::discovery`, porta 9001) via o objeto `Discovery` do UniFFI e relê a lista a cada ~2 s.
  Receber broadcast UDP no Android exige segurar um `WifiManager.MulticastLock` enquanto a tela está aberta e a
  permissão **`CHANGE_WIFI_MULTICAST_STATE`** (declarada no `AndroidManifest.xml`). Só LAN/sub-rede.
  - Com o compartilhamento ligado, o celular também **se anuncia** na descoberta (`discovery::start`), então o PC o encontra.
- Logs do núcleo: `files/elo-android.log`.

## API do núcleo (`elo-android`, via UniFFI)

Pacote Kotlin gerado: `io.github.ricardomendes95.elo.ffi`.

- `Elo(dataDir)` — cria a runtime Tokio (2 threads) e o log. Métodos **bloqueantes** (chame em `Dispatchers.IO`):
  - `deviceId(): String` — liga o nó iroh na 1ª chamada.
  - `connect(target, lan, password, name, totpCode?, directory, downloadDir, listener): Session` — lança
    `EloException.TwoFaRequired` (peça o código e chame de novo) ou `EloException.Failed(msg)`.
  - `isOnline(target, lan, directory): Boolean`.
- `Session` — métodos **não bloqueantes** (enfileiram em ordem): `info()`, `mouseMove(x, y)`,
  `mouseButton(button: 0 esq./1 dir./2 meio, down)`, `scroll(dx, dy)` (em "cliques"; o host multiplica por 120),
  `key(vk, down)`, `char(codePoint)`, `text(s)`, `pointer(x, y)`, `chat(text)`, `selectMonitor(i)`, `sendSas()`,
  `setAudio(on)`, `sendClipboard(text)` (≤ 1 MiB), `sendFile(name, bytes)`, `hangup()` (encerra; `close()` é o
  `AutoCloseable` gerado pelo UniFFI, que só libera o objeto).
- `SessionListener` (implementado em Kotlin, chamado em thread de fundo): `onVideoH264`, `onVideoBgra`,
  `onScreenChanged`, `onMonitors`, `onAudio`, `onClipboard`, `onChat`, `onFileReceived`, `onDisconnected` (último).
- `SessionInfo { width, height, control, fileTransfer, clipboard, audio, fingerprint }`, `Monitor { index, name, x, y,
  width, height, primary, current }`.
- `Discovery()` (descoberta na LAN): `start()` (começa a escutar; erro se o bind falhar), `devices(): List<DiscoveredDevice>`
  (lista atual, podada após 10 s), `stop()`. `DiscoveredDevice { name, ip, port, shortId }`. Exige o `MulticastLock`
  e a permissão `CHANGE_WIFI_MULTICAST_STATE`.

## Testar sem um Windows

`crates/client-core/examples/mock_host.rs` é um host de teste com tela sintética (1280×720) que aceita conexões
diretas, aprova pedidos automaticamente e imprime cada input/chat recebido:

```sh
cargo run --release -p elo-client-core --example mock_host      # 0.0.0.0:9000, senha "elo"
```

No emulador (AVD x86_64), conecte em **Rede local → `10.0.2.2`** (com ou sem senha). Use `--release`: em debug o
OpenH264 é lento e o vídeo fica abaixo de 1 qps.

Verificado assim no emulador (Android 16 / API 36, decoder `c2.goldfish.h264.decoder`): conexão por senha e por
aprovação, vídeo H.264 via MediaCodec (~23 qps), toque → `MouseMove` + clique no pixel certo, arrastar, pressionar →
clique direito, texto pelo IME (`Char`), Enter/Backspace/setas (`Key`), Ctrl+C e Tab pela barra de teclas especiais,
chat e fim da sessão pelo host. **Não verificados** ainda: celular real, host Windows real, gestos de dois dedos e
pinça (o `adb input` não faz multitoque), áudio, multimonitor, envio de arquivo, área de transferência, 2FA e P2P pela
internet.

## Limitações conhecidas

- Senhas de favoritos ficam em texto claro nas preferências do app (como no `config.json` do desktop).
- A sessão não tem serviço em primeiro plano: se o Android encerrar o app em segundo plano, a sessão cai.
- **Não há picture-in-picture (PiP) / janela flutuante** no Android: em *Configurações → Sessão* só existe **manter a tela
  ligada**. O recurso "flutuante" (manter no topo) existe apenas no app Windows.
- Quando o host encerra a sessão, o app volta ao Início sem mostrar um aviso.
- APK de debug (assinado com a chave de debug). Não há build de release assinado.

## Host no Android

Com **Compartilhar esta tela** ligado (cartão na tela Início), o celular aceita conexões do Elo do PC (LAN TCP 9000 e
P2P) e o PC vê e controla a tela. A GUI Windows não mudou.

### Como funciona

- **Rust** (`crates/android/src/host.rs`): `Elo.startHost(name, password, listener)` sobe um loop igual ao do
  `hostrun.rs` do desktop (LAN `0.0.0.0:9000` + `node.accept()` P2P, uma sessão por vez) com `serve_host_opts`,
  anuncia na LAN, e usa uma chave Noise estável em `files/host.key` (ID curto de 9 dígitos derivado dela). Permissões:
  só **controle** (sem arquivos, área de transferência ou áudio). Sem H.264 (`prefer_h264 = false`): o vídeo vai pelo
  codec de tiles BGRA, 15 qps. Senha vazia = `on_approval` (bloqueia até o dono responder, ~2 min); com senha, não
  há pedido de aprovação.
- **Captura** (`host/HostService.kt`): consentimento do `MediaProjection` na UI → serviço em primeiro plano
  (`foregroundServiceType="mediaProjection"`, notificação "Compartilhando a tela" com **Parar**) → `VirtualDisplay`
  → `ImageReader` RGBA_8888 (lado maior ≤ 1280 px, ~15 qps, com envio final garantido para telas paradas) →
  `Host.pushFrame(data, w, h, rowStride)` (converte para BGRA compacto e guarda o último quadro). Rotação recria
  o leitor e redimensiona o `VirtualDisplay`; `MediaProjection.Callback.onStop` (obrigatório no Android 14+) desliga tudo.
- **Controle** (`host/EloAccessibilityService.kt`): converte coordenadas do quadro para a tela real
  (`x * larguraReal / larguraDoQuadro`). Clique = toque; botão esquerdo + movimento = arrasto (traços encadeados,
  `willContinue`); botão direito = toque longo (650 ms); roda = deslizar; Esc = Voltar; Home = Início; Backspace remove o
  último caractere do campo focado (campo vazio = Voltar); Enter = `ACTION_IME_ENTER` (API 30+); `Char` =
  `ACTION_SET_TEXT` no campo focado, acrescentando ao texto. Sem o serviço ligado, a sessão é **só visualização**.
- **UI**: cartão "Compartilhar esta tela" (liga/desliga, senha opcional salva em `Prefs.hostPassword`, ID P2P e IP da LAN
  com copiar, estado da sessão, **Encerrar sessão**, botão para as configurações de acessibilidade) e diálogo de
  aprovação em qualquer aba; com o app em segundo plano o pedido vira notificação com **Aceitar/Recusar**.

### API UniFFI do host

- `Elo.startHost(name, password, listener): Host` (bloqueante, `Dispatchers.IO`).
- `Host`: `address()`, `pushFrame(data, width, height, rowStride)`, `endSession()`, `stop()`.
- `HostListener` (threads de fundo): `onApproval(name, fingerprint): Boolean` (bloqueante), `onSessionStarted(peer)`,
  `onSessionEnded()`, `onInput(ev: HostInput)`, `onStatus(text)`.
- `HostInput`: `MouseMove{x,y}`, `MouseButton{button,down}`, `Scroll{dx,dy}`, `Key{code,down}` (VK Windows), `Char{code}`.

### Como testar

1. Instale o APK, abra o Elo, ligue **Compartilhar esta tela** e aceite a captura. Anote o IP da LAN (ou o ID P2P).
2. Ligue o serviço **Elo: controle remoto** em Configurações > Acessibilidade (botão no cartão). Em Android 13+, apps
   instalados fora da loja precisam antes de Informações do app > ⋮ > *Permitir configurações restritas*.
3. No Elo do Windows, conecte em **Rede local → IP do celular** (ou o ID P2P) com a senha, ou sem senha e aceite o
   pedido no celular. O celular também aparece em "Dispositivos na rede" do PC.
4. Verifique: imagem, clique, arrastar, botão direito (toque longo), roda, Esc/Home, digitação num campo de texto,
   rotação da tela, **Parar** na notificação e **Encerrar sessão**.

### Limitações

- **Não testado em celular real** (nem no emulador): o desenvolvimento não teve aparelho.
- Controle limitado ao que acessibilidade permite: sem teclas arbitrárias (só Esc/Backspace/Home/Enter e texto em
  campos editáveis), sem multitoque, sem Ctrl+Alt+Del; telas protegidas (`FLAG_SECURE`, senha, alguns bancos) saem
  pretas e podem bloquear gestos. A Play Store restringe o uso de `AccessibilityService` para controle remoto.
- Vídeo por tiles BGRA (sem H.264): usa mais banda que o host Windows; sem áudio, arquivos ou área de transferência.
- O consentimento do `MediaProjection` é pedido a cada vez que se liga; o endereço P2P só aparece depois do nó iroh subir.
- Não há registro no servidor de diretório (ID curto de 9 dígitos não resolve pelo diretório); use o ID P2P ou a LAN.
- O arrasto longo envia traços de ~40 ms; apps muito sensíveis podem perceber pequenas pausas.
