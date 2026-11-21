# P2P pela internet (iroh)

O crate `elo-p2p` dá acesso **de qualquer rede**, sem você hospedar servidor, via
NAT hole punching — a mesma ideia de torrent/WebRTC.

## Como funciona

- Construído sobre o **iroh 1.x** (QUIC dialado por chave pública de nó).
- Cada máquina tem um **endereço Elo** = o `EndpointId` (chave pública ed25519) do
  seu nó iroh. É o que se cola para conectar.
- Ao conectar, o iroh:
  1. descobre o endereço público do par (discovery da n0, preset `N0`);
  2. faz **hole punching** UDP → conexão **direta** máquina-a-máquina quando possível;
  3. cai para os **relays públicos gratuitos da n0** quando o furo falha (NAT simétrico).
- Toda a mídia (tela/input/arquivos) trafega nesse canal QUIC; por cima dele roda a
  sessão Elo (Noise + codec) sem mudança.

## API (`elo-p2p`)

- `const ALPN: &[u8] = b"elo/1"` — protocolo de aplicação (a sessão Elo).
- `const PING_ALPN: &[u8] = b"elo/ping/1"` — sonda de **presença** (ver abaixo).
- `struct Duplex` — um stream QUIC bidirecional como `AsyncRead + AsyncWrite`.
- `fn load_or_create_secret(path) -> Result<SecretKey>` — segredo persistente do nó (32 bytes) que
  mantém o endereço Elo estável.
- `fn id_from_secret_file(path) -> Option<String>` — o endereço Elo gravado num arquivo de segredo, **sem
  abrir o endpoint** (a GUI usa para mostrar o endereço do serviço).
- `async fn bind(secret) -> Result<Endpoint>`, `accept(&Endpoint)`, `connect(&Endpoint, node)` — funções de baixo nível
  (só `ALPN`).
- `struct Node` (clonável) encapsulando o endpoint, com ALPN `elo/1` **e** `elo/ping/1`:
  - `async fn bind_node(secret_path) -> Result<Node>` — nó persistente (host).
  - `async fn bind_node_ephemeral() -> Result<Node>` — nó descartável com chave nova, sem gravar nada
    (controller; a CLI `connect --p2p` e a GUI em modo serviço usam).
  - `Node::id() -> String` (endereço Elo), `Node::accept()`, `Node::connect(&str)`,
    `Node::ping(&str, timeout) -> bool` (presença).

### `Node`: aceite concorrente em segundo plano

Ao criar o `Node`, uma tarefa de fundo aceita **todas** as conexões de entrada em paralelo: cada conexão é
tratada na sua própria tarefa, então um par lento ou ocioso não bloqueia os outros.
- ALPN `elo/1`: espera até **20 s** o par abrir o stream bidirecional e o entrega a `Node::accept()` por um canal
  (capacidade 8).
- ALPN `elo/ping/1` (**sonda de presença**): o host só completa o handshake QUIC, espera 200 ms e fecha a
  conexão ("pong"); nunca chega a `Node::accept()`.
- `Node::ping(node, timeout)` conecta com `PING_ALPN` e retorna `true` se o handshake completar dentro do tempo.
  É o que alimenta o indicador **online/offline** dos favoritos (timeout de 8 s, a cada ~45 s). Para favoritos com ID curto, a GUI usa o diretório (`resolve` bem-sucedido = online); para
  favoritos de LAN, tenta abrir TCP na porta 9000 (3 s).

## Integração

- O stream `Duplex` é passado a `elo_client_core::host::serve_host_opts` (host) ou
  `controller::connect_on_opts` (controller) — os mesmos pontos de entrada genéricos
  usados pelo modo direto TCP. O Noise + senha/aprovação continuam valendo por cima (defesa
  em profundidade; o iroh já autentica por chave de nó).
- No `elo-gui`, **um único `Node`** é criado no início e usado tanto para aceitar
  (host) quanto para discar (controller). Com o serviço instalado, a GUI usa só um `bind_node_ephemeral` para
  conexões de saída, e o worker do serviço é quem aceita.
- O segredo do nó é salvo em `%APPDATA%\elo\node.key` (Windows) / `~/.config/elo/node.key`
  (serviço: `C:\ProgramData\elo\node.key`; CLI: `./.elo-cli/node.key`).

## ID curto (diretório), estilo AnyDesk

O endereço nativo é o node id (longo). Para um **número curto** (ex.: `123 456 789`),
Elo usa um **diretório** opcional — um mapa `ID curto → node id`:

- O ID curto de 9 dígitos é gerado uma vez (aleatório) e persistido (`config.json → short_id`).
- Em **Configurações → Conexão → ID curto (diretório)**, informe um servidor
  `host:porta`. O host então **registra** `short_id → node_id` nesse diretório e
  mantém a conexão aberta (`spawn_directory` reconecta a cada 5 s). Sem diretório, mostra-se o node id.
- O controller (GUI ou CLI `--p2p ... --directory`), ao digitar um número de 9 dígitos (a GUI considera ID curto
  se tem 9 dígitos e até 11 caracteres, p.ex. `123 456 789`) com diretório configurado,
  **resolve** o número → node id e então conecta **P2P direto**. O diretório só
  resolve o número; **não vê a sessão**.
- O diretório é o próprio `elo-relay` / `elo-client relay` (mesmas mensagens
  `Register`/`Resolve`/`Resolved`, índices 5/6/7 do `ControlMsg`), auto-hospedável num VPS. É o equivalente, em
  escala mínima, ao diretório que o AnyDesk opera.

## Estabilidade

- **Keepalive:** o controller envia `Ping` (`SessionMsg`) a cada 15 s; o host responde `Pong`.
  Mantém a conexão e os mapeamentos de NAT/relay vivos e detecta links mortos.
- **Reconexão do registro** no diretório com backoff de 5 s.

## Segurança

- A identidade do par é a chave pública do nó iroh (autenticada pelo QUIC/TLS).
- Elo ainda executa seu próprio handshake Noise + autenticação por senha dentro do
  canal, e expõe o fingerprint do host (`ControllerHandle::peer_fingerprint`).
- Os relays da n0, quando usados como fallback, veem apenas bytes cifrados.
