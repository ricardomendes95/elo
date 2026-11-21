# Arquitetura

## Componentes (crates do workspace)

O projeto é um workspace Cargo em `crates/`:

| Crate | Roda em | Papel |
|-------|---------|-------|
| `elo-protocol` | qualquer | Tipos de fio: mensagens de controle do relay/diretório, mensagens de sessão (v2), framing e helpers de (de)serialização. |
| `elo-session` | qualquer | Transporte cifrado ponta a ponta: handshake Noise XX + transporte AEAD com framing, sobre qualquer stream `AsyncRead/AsyncWrite`. |
| `elo-relay` | Linux/qualquer | Servidor de rendezvous + diretório de IDs curtos: pareia host e controller por ID e depois copia bytes cegamente. Nunca vê o conteúdo. |
| `elo-client-core` | qualquer¹ | Núcleo independente de plataforma: codecs (tiles e H.264), áudio, área de transferência, TOTP, transferência de arquivo, laços de host/controller, rendezvous, e (só no Windows) o backend de captura/input. |
| `elo-p2p` | qualquer | Transporte P2P pela internet via iroh (QUIC + hole punching + relays gratuitos). Ver [p2p.md](p2p.md). |
| `elo-client` | Windows¹ | CLI: subcomandos `relay`, `host`, `connect` (relay, direto e P2P). Ferramenta de dev/headless. |
| `elo-gui` | Windows¹ | **O produto**: janela única (host + controller) e também o executável do serviço/worker. Gera `elo.exe`. |

¹ O backend de tela/input é `#[cfg(windows)]`. Em Linux os crates compilam
(sem esse backend) para permitir testes e CI; o papel de **host** exige Windows. O
áudio (`audio.rs`) também só tem implementação real no Windows (no resto são stubs com
`available() == false`).

### Dependências entre crates

```
elo-protocol ─────────────┐
                          ├──► elo-client-core ──► elo-client ─► (usa elo-relay p/ subcomando)
elo-session ──────────────┘            │
elo-p2p ───────────────────────────────┼──► elo-gui
                                       └──► (elo-gui também usa elo-p2p direto)
elo-relay ────────────────────────────────┘ (dev-dep de client-core para o teste e2e)
```

## O backend de plataforma (Windows)

`elo-client-core::platform` (arquivo `platform.rs`, `#[cfg(windows)]`):

- **`DxgiCapture`** — captura de tela via **DXGI Desktop Duplication**, numa **thread dedicada** que é
  dona de todos os objetos D3D11/DXGI (o lado async manda pedidos por um canal e espera a resposta). Enumera
  os outputs de todos os adaptadores (`monitors()`), troca de monitor (`select_monitor`) e recupera
  `DXGI_ERROR_ACCESS_LOST`. Entrega BGRA compacto `width*height*4`. Implementa o trait `Capture`.
- **`ensure_input_desktop()`** — antes de capturar/injetar, anexa a thread ao **desktop de entrada**
  atual (`Default` ↔ `Winlogon`), de modo que um processo SYSTEM na sessão do console continue funcionando
  na tela de bloqueio e em prompts UAC.
- **`SendInputSink`** — injeção de mouse/teclado via **`SendInput`**: mouse absoluto (coordenadas relativas ao monitor capturado, convertidas para 0..65535 no
  desktop virtual, `MOUSEEVENTF_VIRTUALDESK`), botões, roda (`WHEEL_DELTA`), teclas por VK, caracteres Unicode (`InputEvent::Char`) e
  **`send_sas()`** (Ctrl+Alt+Del via `sas.dll`/`SendSAS(FALSE)`; só funciona com o serviço e a política
  `SoftwareSASGeneration`). Implementa o trait `InputSink`.

Os traits `Capture` (`width`, `height`, `frame`, `monitors`, `select_monitor`) e `InputSink`
(`inject`, `set_area`, `send_sas`) são definidos em `client-core` e permitem que os laços de sessão
rodem contra o hardware real no Windows e contra **mocks** (`MockCapture`, `RecordingInput`) nos testes.

## Fluxo de uma sessão

1. **Transporte.** O host obtém um stream até o controller:
   - via relay (`rendezvous::register_host`): registra o ID, espera `Paired`;
   - direto (TCP porta 9000 na LAN) ou P2P (iroh/QUIC), entregues a `serve_host_opts`;
   O controller faz o espelho (`connect_controller`, `run_controller_direct_opts`, `connect_on_opts`).
2. **Handshake Noise XX** (`elo-session`). Cada lado aprende a chave estática do
   outro (no relay é comparada com a anunciada; nos outros modos vale senha/aprovação + fingerprint).
3. **Identificação e autenticação.** `Hello{name}` → `AuthRequired{salt}` → `AuthSubmit{proof}` →
   decisão de acesso (pareado → aberto → senha → aprovação) → opcional `TwoFaRequired`/`TwoFaSubmit` →
   `AuthOk`/`AuthFailed`. Detalhes em [protocolo.md](protocolo.md) (seção 5).
4. **Capacidades e setup.** Controller envia `Caps{h264,audio}`; o host cria o backend de captura/input
   **só agora**, e envia `Permissions`, `ScreenInfo` e `Monitors`.
5. **Sessão ao vivo.** O canal cifrado é dividido (`split`) em escrita/leitura:
   - host: tarefa de captura/codificação (H.264 ou tiles, com troca de monitor e de resolução), tarefa de
     áudio (loopback), ponte de área de transferência e o laço de recepção (input, ponteiro, monitor,
     SAS, clipboard, chat, arquivos, ping);
   - controller: tarefa de leitura que decodifica vídeo, toca áudio, aplica clipboard e emite
     `ControllerEvent`s; fila de saída para input/chat/arquivos; **keepalive** `Ping` a cada 15 s.

## Modos de conexão

| Modo | Relay? | Quando | Como as pontas se acham |
|------|--------|--------|--------------------------|
| **LAN direto** | não | Mesma rede | IP (porta 9000) do host |
| **P2P (iroh)** | relays gratuitos da n0 como fallback | Qualquer rede (internet) | Endereço Elo (node id) ou ID curto de 9 dígitos via diretório |
| **Relay próprio** | sim | Alternativa self-hosted (CLI) | ID de 9 dígitos via `elo-client relay` |

## O app (`elo-gui`): módulos

| Módulo | Papel |
|--------|-------|
| `main.rs` | Ponto de entrada (modos ocultos `--service`, `--session-worker`, `--install-service`, `--uninstall-service`), `App`, conexão de saída, modais (aprovação, 2FA), toasts, sobreposição do ponteiro, janela de chat do host, presença dos favoritos. |
| `config.rs` | `Config` persistida (JSON), `Favorite`, `TrustedDevice`, pastas (`config_dir`, `service_dir`), autostart. |
| `hostrun.rs` | **O laço de host compartilhado** pela GUI e pelo worker do serviço: aceita LAN (TCP 9000) + P2P, e atende **uma conexão por vez** com as regras atuais (`HostSettings`); mantém o registro no diretório (`spawn_directory`). |
| `worker.rs` | Host "headless" (`run_headless`): o que o serviço executa. Relê a configuração **a cada conexão**. |
| `ipc.rs` | IPC local GUI ↔ worker (`127.0.0.1:47199`, JSON por linha). |
| `service.rs` | (Windows) instalar/desinstalar o serviço `EloHost`; supervisor do worker. |
| `events.rs` | `UiEvent` (eventos do host entregues à UI) e `HostCtl` (controlar a sessão: local ou remoto via IPC). |
| `logging.rs` | Logs em arquivo diário + buffer em memória. |
| `ui_home.rs` | Barra lateral, faixa de sessão, páginas Início e Favoritos. |
| `ui_settings.rs` | Configurações (Geral, Segurança, Conexão, Serviço, Logs, Sobre). |
| `ui_session.rs` | Sessão de controle (tela remota, barra de ferramentas, input, arquivos). |
| `theme.rs`, `keymap.rs`, `logo_pixels.rs` | Tema/componentes, tabela egui→VK, ícone. |

### Host compartilhado (`hostrun.rs`)

`hostrun::run(HostRun { node, keypair, download_dir, settings, approver, on_event, status, online_text,
after_session })` roda para sempre: escuta TCP `0.0.0.0:9000` e `node.accept()` (P2P) em paralelo; para cada
conexão chama `settings()` (para pegar as regras vigentes), monta `HostOptions` (fps 30, H.264 preferido) e
`serve_host_opts`. `HostSettings::from_config` calcula a senha efetiva: a **senha fixa** (se definida e
`otp` desligado) ou a **senha da sessão** (aleatória de 6 dígitos; vira nova a cada sessão com `otp`).
Em não-Windows o laço só reporta "pronto para conectar" (servir a própria tela exige Windows).

### Serviço do Windows e worker

```
 Windows (sessão 0)                         Sessão do console (usuário logado ou tela de login)
 ┌─────────────────────┐   CreateProcessAsUser   ┌───────────────────────────────┐
 │ EloHost (SYSTEM)    │ ──────────────────────▶ │ elo.exe --session-worker      │
 │ elo.exe --service   │   (token SYSTEM com     │ (SYSTEM, winsta0\default)     │
 │ = supervisor        │    a sessão do console) │ hostrun + captura/input       │
 └─────────────────────┘                         └──────────┬────────────────────┘
                                                            │ IPC 127.0.0.1:47199
                                                   ┌────────▼────────┐
                                                   │ elo.exe (GUI)   │ aprovações, faixa,
                                                   │ do usuário      │ ponteiro, chat, kick
                                                   └─────────────────┘
```

- O serviço (`--service`) vive na sessão 0, sem acesso à tela do usuário; por isso é só um **supervisor**:
  a cada 2 s consulta `WTSGetActiveConsoleSessionId` e (re)lança `elo.exe --session-worker` como **SYSTEM**
  dentro da sessão ativa do console (`DuplicateTokenEx` + `SetTokenInformation(TokenSessionId)` +
  `CreateProcessAsUserW`, desktop `winsta0\default`), relançando se o worker morrer ou a sessão do console mudar.
  Ao parar o serviço, o worker é terminado.
- Como o worker roda como SYSTEM na sessão do console, a captura/input **seguem o desktop seguro** (tela de
  bloqueio, UAC) via `ensure_input_desktop`.
- Se o lançamento do worker falhar **3 vezes seguidas**, o serviço cai para **hospedar no próprio processo**
  (sessão 0, `run_in_process`) — o que dificilmente captura a tela do usuário.
- O worker guarda tudo em `C:\ProgramData\elo` (`node.key`, `host.key`, `config.json`, `ipc.token`,
  `logs\`, `recebidos\`) e **relê `config.json` a cada conexão**; mudanças feitas na GUI valem sem reiniciar.
  A senha de sessão do worker é **vazia** (não há senha aleatória no serviço): só entram quem tem a senha
  fixa (e `otp` desligado), dispositivos pareados, acesso aberto, ou quem for aprovado na GUI.
- **Instalação** (`--install-service`, elevado): copia `node.key`, `host.key`, `config.json` do usuário para
  `ProgramData\elo` (`seed_service_dir`); `icacls ... /grant *S-1-5-32-545:(OI)(CI)M` (grupo **Users** pode
  modificar a pasta, para a GUI gravar a configuração); `reg add HKLM\...\Policies\System /v
  SoftwareSASGeneration /t REG_DWORD /d 3` (serviços podem gerar Ctrl+Alt+Del); cria o serviço `EloHost`
  ("Elo Remote Host", início automático, LocalSystem, argumento `--service`); `sc failure ... reset= 60
  actions= restart/5000` (reinicia 5 s depois de uma falha); inicia o serviço.
- **IPC GUI ↔ worker**: a GUI escuta em `127.0.0.1:47199`; o worker conecta (e reconecta a cada 3 s) e
  manda `Hello{token}` com o conteúdo de `ipc.token` (criado pelo worker, 32 hex). Mensagens (JSON por linha,
  campo `t` em snake_case): `hello{token}`, `approve{id,name,fp}` (worker → GUI), `approve_reply{id,allow}`
  (GUI → worker), `connected{name,fp}`, `disconnected{name}`, `auth_failed{name,reason}`,
  `pointer{x,y,ax,ay,aw,ah}`, `chat{name,text}` (nos dois sentidos), `file{path}`, `kick` (GUI → worker).
  Sem GUI conectada, `approve` retorna "negado" na hora. A IPC existe **só no desktop**.
- Se o serviço está instalado, a GUI vira o painel dele (`service_mode`): lê o endereço do `node.key` do
  serviço, não hospeda por conta própria, e usa um nó P2P efêmero só para conexões de saída.
- Pedido de aprovação com o worker: a GUI precisa estar aberta para aprovar (120 s de limite).

### Captura numa thread dedicada

A captura DXGI roda numa thread própria que chama `ensure_input_desktop()` a cada quadro (e quando muda de
desktop, recria a duplicação). Nenhum objeto COM atravessa threads.

## Concorrência (sessão)

Cada sessão usa partes sobre o canal Noise dividido:
- um **writer** (único dono do `EncryptedSender`) drenando uma fila `mpsc`;
- host: tarefa de **captura+codificação**, tarefa de **áudio**, **clipboard** e o **laço de recepção**;
  controller: tarefa de **leitura/decodificação**, **keepalive**, **clipboard**.

Como o `TransportState` do Noise é compartilhado entre as metades via
`Arc<Mutex<_>>` travado só ao redor das operações rápidas de cripto, leitura e
escrita acontecem de verdade em paralelo sem travar no socket.

## Arquivos de estado

| Onde | Arquivo | Conteúdo |
|------|---------|----------|
| `%APPDATA%\elo\` (Linux: `~/.config/elo/`) | `node.key` | segredo do nó P2P (32 bytes) — o endereço Elo |
| idem | `host.key` | chave Noise do host: privada(32) ‖ pública(32) |
| idem | `config.json` | configuração (ver [acesso-nao-supervisionado.md](acesso-nao-supervisionado.md)) |
| idem | `logs\elo-AAAA-MM-DD.log` | logs diários (14 dias) |
| `C:\ProgramData\elo\` | `node.key`, `host.key`, `config.json`, `ipc.token`, `logs\`, `recebidos\` | estado do serviço/worker |
