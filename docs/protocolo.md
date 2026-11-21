# Protocolo de fio

Versão do protocolo documentada: **v2**. A v2 apenas **acrescenta** variantes ao
fim de `SessionMsg` e `InputEvent` (os índices antigos não mudam), então os
tipos da v1 continuam com a mesma codificação.

Há duas camadas de mensagens:

1. **Controle do relay / diretório** (`ControlMsg`) — só o relay parseia, durante o encontro.
2. **Sessão** (`SessionMsg`) — viaja *dentro* do canal cifrado; o relay nunca vê.

Ambas usam `bincode` para serializar. O framing depende da camada (ver a seção
"Camadas de framing"). Limite por frame lógico: `MAX_FRAME = 16 MiB`.

## 1. Controle do relay e diretório — `ControlMsg`

Definido em `crates/protocol/src/lib.rs`.

| Índice | Variante | Direção | Significado |
|-------:|----------|---------|-------------|
| 0 | `RegisterHost { device_id: String, pubkey: Vec<u8> }` | host → relay | Anuncia-se e fica disponível sob o ID. |
| 1 | `Registered` | relay → host | Confirma registro; o host aguarda. |
| 2 | `Connect { target_id: String, pubkey: Vec<u8> }` | controller → relay | Pede para ser roteado ao ID. |
| 3 | `Paired { peer_pubkey: Vec<u8> }` | relay → ambos | Pareados; entrega a chave pública do par. Tudo depois disso é a sessão E2E. |
| 4 | `Error { reason: String }` | relay → quem pediu | ID desconhecido, ocupado, malformado… |
| 5 | `Register { device_id: String, node_id: String }` | host → diretório | Publica ID curto → node id iroh; mantém a conexão aberta. O relay responde `Registered`. |
| 6 | `Resolve { device_id: String }` | controller → diretório | Pede o node id de um ID curto. |
| 7 | `Resolved { node_id: String }` | diretório → controller | Resposta do `Resolve` (ou `Error { reason: "id N is not online" }`). |

- `device_id`: string de 9 dígitos, exibida como `123 456 789` (`format_id`). O relay
  só aceita `Register` com exatamente 9 dígitos ASCII (senão responde `Error`).
- `pubkey`: chave estática X25519 crua (32 bytes). O relay apenas repassa; não a usa para decifrar.
- Depois do `Paired`, o relay faz **cópia bidirecional cega** dos bytes entre os dois sockets.
- Diretório: o mapeamento `device_id → node_id` vale enquanto a conexão do `Register`
  estiver aberta; ao fechar, o relay remove a entrada. O relay (`elo-relay`) faz os dois
  papéis (rendezvous e diretório) na mesma porta, distinguindo pela primeira mensagem.

## 2. Sessão — `SessionMsg`

Definido em `crates/protocol/src/session.rs`. Helpers `encode_msg`/`decode_msg`.

### Tabela completa (índice = tag `u32` LE do bincode)

| Idx | Variante e campos (na ordem do fio) | Direção | Desde |
|----:|-------------------------------------|---------|-------|
| 0 | `AuthRequired { salt: [u8;16] }` | host → controller | v1 |
| 1 | `AuthSubmit { proof: Vec<u8> }` | controller → host | v1 |
| 2 | `AuthOk` | host → controller | v1 |
| 3 | `AuthFailed` | host → controller | v1 |
| 4 | `Permissions(Permissions)` | host → controller | v1 |
| 5 | `ScreenInfo { width: u32, height: u32 }` | host → controller | v1 |
| 6 | `Frame(VideoFrame)` | host → controller | v1 |
| 7 | `Input(InputEvent)` | controller → host | v1 |
| 8 | `Chat(String)` | ambos | v1 |
| 9 | `FileOffer { id: u32, name: String, size: u64 }` | ambos | v1 |
| 10 | `FileAccept { id: u32 }` | (reservado; o código atual não envia) | v1 |
| 11 | `FileChunk { id: u32, offset: u64, data: Vec<u8> }` | ambos | v1 |
| 12 | `FileDone { id: u32 }` | ambos | v1 |
| 13 | `FileCancel { id: u32, reason: String }` | ambos | v1 |
| 14 | `Ping` | controller → host | v1 |
| 15 | `Pong` | host → controller | v1 |
| 16 | `TwoFaRequired` | host → controller | v2 |
| 17 | `TwoFaSubmit { code: String }` | controller → host | v2 |
| 18 | `Hello { name: String }` | controller → host | v2 |
| 19 | `Pointer { x: i32, y: i32 }` | controller → host | v2 |
| 20 | `Clipboard(String)` | ambos | v2 |
| 21 | `Monitors { list: Vec<MonitorInfo> }` | host → controller | v2 |
| 22 | `SelectMonitor { index: u32 }` | controller → host | v2 |
| 23 | `SendSas` | controller → host | v2 |
| 24 | `Caps { h264: bool, audio: bool }` | controller → host | v2 |
| 25 | `FrameH264 { width: u32, height: u32, key: bool, data: Vec<u8> }` | host → controller | v2 |
| 26 | `Audio { rate: u32, channels: u16, pcm: Vec<u8> }` | host → controller | v2 |

Semântica:

- `AuthSubmit.proof = SHA256(salt ‖ senha_utf8)`. Senha vazia é um caso especial
  (pedido de aprovação; ver seção 5).
- `Hello.name`: nome exibido; o host trunca em 64 caracteres. Se a primeira mensagem
  não for `Hello`, o host assume o nome `"desconhecido"`.
- `Pointer { x, y }`: posição do ponteiro do operador em pixels **relativos ao monitor
  capturado**. O host só repassa à interface se `permissions.control == false` e
  `show_pointer` estiver ligado; caso contrário ignora.
- `Clipboard`: texto (UTF-8), no máximo 1 MiB por mensagem (acima disso não é enviado
  nem aplicado). Só funciona se `permissions.clipboard`.
- `Monitors`: lista dos monitores do host. O host só envia se a lista não for vazia.
- `SelectMonitor { index }`: índice (`MonitorInfo.index`) do monitor a capturar.
- `SendSas`: Ctrl+Alt+Del. O host só age se `permissions.control`; exige o serviço do
  Windows (ver [acesso-nao-supervisionado.md](acesso-nao-supervisionado.md)). Se falhar, o host
  responde com um `Chat` de texto `[Ctrl+Alt+Del indisponível: …]`.
- `Caps`: o que o controller suporta (H.264 e reprodução de áudio).
- `FrameH264`: um frame H.264 em Annex-B (ver seção 3). `width`/`height` são os do
  host (podem ser ímpares); o fluxo H.264 em si carrega o tamanho **par**.
- `Audio`: PCM s16le entrelaçado. Hoje o host envia sempre `rate = 24000`, `channels = 1`,
  `pcm` de 960 bytes (480 amostras = 20 ms).
- Arquivos: o host só aceita `FileOffer` se `permissions.file_transfer`; senão responde
  `FileCancel { reason: "file transfer not permitted" }`. Sem `FileAccept` explícito: o
  receptor aceita direto. Os chunks do remetente têm até 60 KiB (`transfer::CHUNK`) e devem
  chegar contíguos (`offset` crescente).
- `Ping`/`Pong`: o controller envia `Ping` a cada **15 s**; o host responde `Pong`.

### Tipos auxiliares (campos na ordem exata do fio)

```rust
struct Permissions { control: bool, file_transfer: bool, clipboard: bool, audio: bool } // 4 bytes
// Perfis prontos: Permissions::VIEW_ONLY (tudo false), Permissions::FULL (tudo true)

struct Rect { x: u32, y: u32, w: u32, h: u32, data: Vec<u8> }
struct VideoFrame { width: u32, height: u32, key: bool, seq: u64, rects: Vec<Rect> }

enum InputEvent {
    MouseMove { x: i32, y: i32 },                 // idx 0 — pixels no monitor capturado
    MouseButton { button: MouseButton, down: bool }, // idx 1
    Scroll { dx: i32, dy: i32 },                  // idx 2
    Key { code: u16, down: bool },                // idx 3 — código VK do Windows
    Char { code: u32 },                           // idx 4 — escalar Unicode (v2)
}
enum MouseButton { Left, Right, Middle }          // 0, 1, 2

struct MonitorInfo {                              // v2
    index: u32, name: String,
    x: i32, y: i32,                               // posição no desktop virtual
    width: u32, height: u32,
    primary: bool,
    current: bool,                                // monitor capturado agora
}
```

- `InputEvent::Char { code }`: o controller envia para texto não ASCII (ç, acentos de
  teclas mortas, €…); teclas ASCII continuam indo como `Key` com código VK. O host Windows
  digita com `SendInput` + `KEYEVENTF_UNICODE` (unidades UTF-16, independente do layout;
  pressiona e solta). Um `code` que não é escalar Unicode válido é ignorado. Só há
  evento de "pressionar" — não existe `down`.
- Coordenadas de `MouseMove`/`Pointer` são **relativas ao monitor capturado** (0..largura-1,
  0..altura-1); o host soma a origem do monitor (`set_area`) ao injetar.

## 3. Vídeo

O host escolhe **uma** das duas codificações por sessão (e por mudança de monitor/resolução):

### 3a. H.264 (`FrameH264`) — preferido

Implementado em `crates/client-core/src/codec_h264.rs` com **OpenH264** (compilado do código-fonte).

- **Usado quando**: o controller anunciou `Caps.h264 = true`, o host tem `prefer_h264` (padrão
  ligado) e `codec_h264::available()` (teste de ida e volta de 64×64 no primeiro uso).
- Fluxo em **Annex-B** (NAL units com start code `00 00 00 01`). Cada `FrameH264.data` é um
  frame completo; `key = true` para IDR/I. Frame descartado pelo encoder (skip) **não é enviado**.
- Perfil de uso `ScreenContentRealTime`, controle de taxa por bitrate, `skip_frames` ligado.
  Bitrate: `clamp(largura × altura × fps / 20000, 800, 6000)` kbps; fps do host = 30.
- **Conversão de cor** BGRA → I420: **BT.601, faixa limitada**:
  `Y = ((66R + 129G + 25B + 128) >> 8) + 16`;
  `U = ((-38R - 74G + 112B + 128) >> 8) + 128`; `V = ((112R - 94G - 18B + 128) >> 8) + 128`
  (com R, G, B da média 2×2 para U/V).
- **Tamanhos ímpares**: o encoder estende até um tamanho par replicando o último pixel. O
  **decoder devolve o tamanho par** do fluxo (o fluxo não carrega o tamanho original); o controller então
  **corta** a coluna/linha extra usando o tamanho do último `ScreenInfo` (diferença de no máximo 1 px).
- Decodificação I420 → BGRA (inversa BT.601 limitada): `c = (Y-16)*298`, `d = U-128`, `e = V-128`;
  `R = (c + 409e + 128) >> 8`, `G = (c - 100d - 208e + 128) >> 8`, `B = (c + 516d + 128) >> 8`
  (limitados a 0..255).
- Keyframe forçado a cada `fps × 3` frames (~3 s) e sempre no primeiro.
- Se o encode H.264 falhar no meio da sessão, o host passa para o codec de tiles (`Frame`) até o fim.
- O controller cria o decoder sob demanda ao receber o primeiro `FrameH264`.

### 3b. Codec de tiles (`Frame`) — alternativa/fallback

Implementado em `crates/client-core/src/codec.rs`.

- A tela é dividida em **tiles de 64×64** (`TILE`), com bordas menores nas pontas.
- **Key frame**: emite todos os tiles. **Delta**: só os tiles cujos bytes mudaram desde o quadro anterior.
- Cada tile vira um `Rect` com `data` = pixels BGRA do tile (row-major, `w*h*4` bytes) comprimidos com
  **zlib DEFLATE** (`flate2`).
- O `Encoder` guarda o quadro anterior; o laço do host força um key frame a cada `fps × 3` quadros (~3 s).
- O `Decoder` mantém um canvas e aplica os rects; devolve o canvas BGRA completo.

### 3c. Mudança de resolução / monitor no meio da sessão

Quando o tamanho capturado muda (configuração de vídeo, rotação) ou o controller manda
`SelectMonitor`, o host **recria o encoder** e reenvia `ScreenInfo { width, height }` (e `Monitors`
atualizado, se houver). O controller deve tratar `ScreenInfo` a qualquer momento, não só no setup.
No caso de `SelectMonitor` inválido, o host só registra um aviso e continua no monitor atual.

## 4. Áudio

- Captura no host: **loopback** do dispositivo de saída padrão (WASAPI via `cpal`), **somente Windows**
  (em outros SOs `audio::available()` é `false`).
- Formato no fio: **PCM s16le, mono, 24 000 Hz**, blocos de **20 ms (480 amostras = 960 bytes)**
  → ~384 kbit/s.
- O host só envia áudio se `Caps.audio == true`, `permissions.audio` e o áudio estiver disponível.
- O controller reproduz com `AudioPlayer` (reamostra/remapeia canais para o dispositivo local; fila
  limitada a ~250 ms). Silenciar é local (`set_audio(false)`): o host continua enviando.

## 3b. Codificação binária exata (para reimplementar, p.ex. no Android)

As mensagens usam **bincode 1.3** (via serde) com a configuração padrão (`bincode::serialize`).
Regras de bytes:

- **Inteiros**: largura fixa, **little-endian** (`u16`, `u32`, `u64`, `i32`…; `i32` em complemento de dois).
- **bool**: 1 byte (`0`/`1`).
- **enum**: índice da variante como **`u32` little-endian** (tag), seguido dos campos da variante na ordem.
- **struct**: os campos em ordem, sem tag.
- **`Vec<T>` / `String` / bytes**: **`u64` little-endian** com o número de elementos/bytes, seguido dos
  elementos (String em UTF-8).
- **arrays fixos** (`[u8; 16]`): só os bytes, sem comprimento.
- **`Option<T>`**: 1 byte (`0` = None; `1` = Some, seguido do valor).

Índices das variantes (ordem de declaração):

- `ControlMsg`: `0=RegisterHost, 1=Registered, 2=Connect, 3=Paired, 4=Error, 5=Register, 6=Resolve, 7=Resolved`.
- `SessionMsg`: `0=AuthRequired, 1=AuthSubmit, 2=AuthOk, 3=AuthFailed, 4=Permissions, 5=ScreenInfo, 6=Frame,
  7=Input, 8=Chat, 9=FileOffer, 10=FileAccept, 11=FileChunk, 12=FileDone, 13=FileCancel, 14=Ping, 15=Pong,
  16=TwoFaRequired, 17=TwoFaSubmit, 18=Hello, 19=Pointer, 20=Clipboard, 21=Monitors, 22=SelectMonitor,
  23=SendSas, 24=Caps, 25=FrameH264, 26=Audio`.
- `InputEvent`: `0=MouseMove, 1=MouseButton, 2=Scroll, 3=Key, 4=Char`.
- `MouseButton`: `0=Left, 1=Right, 2=Middle`.

### Exemplos (bytes de `encode_msg`, em hexadecimal, gerados a partir do código)

| Mensagem | Bytes |
|----------|-------|
| `Hello { name: "A" }` | `12 00 00 00` `01 00 00 00 00 00 00 00` `41` |
| `Ping` | `0e 00 00 00` |
| `Pong` | `0f 00 00 00` |
| `SelectMonitor { index: 1 }` | `16 00 00 00` `01 00 00 00` |
| `Caps { h264: true, audio: false }` | `18 00 00 00` `01` `00` |
| `Pointer { x: -1, y: 300 }` | `13 00 00 00` `ff ff ff ff` `2c 01 00 00` |
| `TwoFaSubmit { code: "123456" }` | `11 00 00 00` `06 00 00 00 00 00 00 00` `31 32 33 34 35 36` |
| `Audio { rate: 24000, channels: 1, pcm: [1,2] }` | `1a 00 00 00` `c0 5d 00 00` `01 00` `02 00 00 00 00 00 00 00` `01 02` |
| `FrameH264 { 2, 2, key: true, data: [0,0,0,1] }` | `19 00 00 00` `02 00 00 00` `02 00 00 00` `01` `04 00 00 00 00 00 00 00` `00 00 00 01` |
| `Permissions(FULL)` | `04 00 00 00` `01 01 01 01` |
| `Input(Key { code: 0x41, down: true })` | `07 00 00 00` `03 00 00 00` `41 00` `01` |
| `Input(Char { code: 0xE7 })` (ç) | `07 00 00 00` `04 00 00 00` `e7 00 00 00` |
| `Monitors { [ {0,"M",0,0,1920,1080,true,true} ] }` | `15 00 00 00` `01 00 00 00 00 00 00 00` `00 00 00 00` `01 00 00 00 00 00 00 00` `4d` `00 00 00 00` `00 00 00 00` `80 07 00 00` `38 04 00 00` `01` `01` |

Controle (já com o prefixo de frame `u32` BE): `Register { "123456789", "ab" }` =
`00 00 00 1f` `05 00 00 00` `09 00 00 00 00 00 00 00` `31..39` `02 00 00 00 00 00 00 00` `61 62`.

### Camadas de framing (ordem exata do fio)

1. **Controle do relay** (antes do pareamento / diretório): cada `ControlMsg` vai como
   `u32` **big-endian** (tamanho do corpo) + `bincode(ControlMsg)`.
2. **Handshake Noise**: cada mensagem do handshake vai como `u16` **big-endian**
   (tamanho) + bytes da mensagem Noise. Padrão `Noise_XX_25519_ChaChaPoly_BLAKE2s`,
   3 mensagens (iniciador = controller escreve / lê / escreve; respondedor = host o espelho).
3. **Mensagem de sessão** (dentro do Noise):
   - monte `plaintext = bincode(SessionMsg)`;
   - monte `framed = u32_be(plaintext.len) ++ plaintext`;
   - fatie `framed` em pedaços de ≤ 65519 bytes; para cada pedaço, cifre com o Noise
     transport e escreva `u16_be(ciphertext.len) ++ ciphertext`.
   O receptor lê pedaços (`u16` + ciphertext), decifra, acumula num buffer e extrai a
   mensagem lógica quando tem `u32` de tamanho + o corpo completo. Limite lógico: 16 MiB.

Observação: o `u32` de tamanho da camada 3 é **big-endian** (framing), enquanto os inteiros
**dentro** do bincode são little-endian.

## 4b. Transporte de bytes

- **LAN direta**: TCP na porta **9000** do host (`DIRECT_PORT`).
- **P2P**: um stream bidirecional QUIC (iroh), ALPN `elo/1` — ver [p2p.md](p2p.md). O stream
  é tratado como um byte-stream; por cima vêm as camadas 2 e 3.
- **Relay próprio**: o socket TCP do relay depois do `Paired`.

## 5. Ordem do handshake de sessão (v2)

Depois que o transporte está pronto (TCP/QUIC/relay):

```
Controller                                       Host
   │ ══════ Noise XX (3 mensagens, 20 s de limite no host) ══════ │
   │── Hello { name } ─────────────────────────────▶│  (host espera até 15 s)
   │◀── AuthRequired { salt } ──────────────────────│  (ou AuthFailed se bloqueado)
   │── AuthSubmit { proof } ───────────────────────▶│  (host espera até 30 s)
   │            [host decide o acesso — ver abaixo; pode perguntar ao usuário]
   │◀── TwoFaRequired ──────────────────────────────│  (opcional)
   │── TwoFaSubmit { code } ───────────────────────▶│  (host espera até 120 s)
   │◀── AuthOk  |  AuthFailed ──────────────────────│
   │── Caps { h264, audio } ───────────────────────▶│  (host espera até 10 s)
   │◀── Permissions ────────────────────────────────│
   │◀── ScreenInfo { width, height } ───────────────│
   │◀── Monitors { list } ──────────────────────────│  (se houver lista)
   │◀═══ Frame / FrameH264 / Audio / Clipboard … ═══│
```

Notas importantes para quem reimplementa o controller:

- O controller **precisa enviar `Hello` primeiro**; o host só lê ele antes de qualquer outra coisa.
- Depois de `AuthSubmit`, o controller deve ficar num laço lendo: `AuthOk` encerra a fase; `AuthFailed`
  é falha; `TwoFaRequired` pede `TwoFaSubmit` (ou o erro `2FA_REQUIRED` se não houver código). Esse laço
  **não tem limite curto** — o dono do host pode estar decidindo "Permitir/Negar" (até 120 s).
- `Caps` é enviado **logo depois** do `AuthOk`; em seguida o controller lê `Permissions` e `ScreenInfo`
  nessa ordem exata (outra coisa aqui é erro). `Monitors` pode vir depois.
- Se o host não receber `Caps` em 10 s (ou vier outra mensagem), assume `h264 = false, audio = false`.
- No `AuthFailed` o host dorme **1 s** antes de enviar (reduz força bruta).

### Decisão de acesso no host (`host.rs`)

Depois do `AuthSubmit` (o host já conhece o **fingerprint** = hex da chave estática Noise do
controller, e o nome do `Hello`), na ordem:

1. **Dispositivo pareado** (fingerprint na lista `trusted`, sem diferenciar maiúsculas) → aceito.
2. **Acesso aberto** (`open_access`, "acesso não supervisionado") → aceito.
3. **Senha correta** (`proof == SHA256(salt ‖ senha)` e a senha do host não é vazia) → aceito.
4. **Prova da senha vazia** (`proof == SHA256(salt ‖ "")`) **e** há aprovador → o host pergunta ao
   usuário (Permitir/Negar, até 120 s). Permitir → aceito; negar/timeout → rejeitado.
5. Caso contrário → rejeitado (`AuthFailed`).

**2FA**: se o host tem segredo TOTP, ele envia `TwoFaRequired` e exige `TwoFaSubmit` válido nos casos
1, 2 e 3. **Não** pede no caso 4 (aprovado pelo usuário). Código errado → `AuthFailed`.

**Limite de tentativas**: cada falha (senha errada, negado, 2FA errado) é registrada num contador
**global do processo**; com **5 falhas em 60 s**, novas conexões recebem `AuthFailed` logo depois do
`Hello` (sem `AuthRequired`) pelos próximos 60 s (a janela é deslizante).

**`ERR_2FA_REQUIRED`**: constante `"2FA_REQUIRED"` em `controller.rs`; é o texto do erro devolvido
quando o host pede 2FA e o controller não passou código. A UI deve pedir o código e **reconectar** (a
conexão atual é abandonada) com `ConnectOptions.totp_code`.

### 5b. TOTP

`crates/client-core/src/totp.rs`: **RFC 6238**, HMAC-SHA1, passo de **30 s**, **6 dígitos**
(`hotp` com truncamento dinâmico; contador = `unix / 30` em 8 bytes big-endian). A verificação aceita
**±1 passo** (−30 s, 0, +30 s) e ignora caracteres não numéricos do código; exige 6 dígitos.
O segredo tem 20 bytes aleatórios e é guardado em **base32 sem padding** (`secret_to_base32`;
`secret_from_base32` ignora espaços, `=` e maiúsculas/minúsculas). URI de cadastro:
`otpauth://totp/Elo:<conta>?secret=<base32>&issuer=Elo&algorithm=SHA1&digits=6&period=30`
(com `issuer`/`conta` em percent-encoding), renderizável como QR.

### Autenticação — prova

`proof = SHA256(salt ‖ senha_utf8)`, onde `salt` são os 16 bytes de `AuthRequired`.

## 6. Transporte cifrado (`elo-session`)

Sobre o stream (TCP ou outro), o canal Noise adiciona seu próprio framing para
permitir mensagens lógicas grandes (frames, arquivos) acima do teto de 65535
bytes do Noise:

- Uma mensagem lógica = `u32` tamanho + corpo, fatiada em pedaços de ≤ 65519 bytes;
- cada pedaço no fio = `u16` tamanho do ciphertext + ciphertext (Noise transport);
- o receptor remonta num buffer até ter a mensagem lógica completa.
- Limite lógico: 16 MiB.
