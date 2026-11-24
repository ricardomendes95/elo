# Status e roadmap

## Pronto e testado (41 testes passam no Linux, mais 3 da GUI)

- [x] Protocolo de fio v2 (controle do relay/diretório + mensagens de sessão) e framing.
- [x] Transporte cifrado Noise XX (handshake, AEAD, framing em pedaços, `split`).
- [x] Pinagem de chave (anti-MITM via relay) e fingerprint (modo direto/P2P).
- [x] Servidor de relay (rendezvous + cópia cega) e diretório de IDs curtos.
- [x] Codec de tela tile-delta + zlib e **H.264** (OpenH264, Annex-B) negociado por `Caps`, com fallback para tiles.
- [x] Autenticação por senha; perfis de permissão; **decisão de acesso** (pareado → aberto → senha → aprovação) e
      **limite de tentativas** (5 em 60 s).
- [x] **2FA (TOTP)** RFC 6238, com cadastro por QR.
- [x] **Pareamento de dispositivos** ("confiar neste dispositivo") e lista de dispositivos pareados.
- [x] **Pedidos de aprovação** (Permitir/Negar) e `Hello` com o nome do operador.
- [x] Input remoto (mouse/teclado/roda, **texto Unicode** via `Char`); chat; transferência de arquivo (arrastar e soltar, **nos dois sentidos** — host → controlador via `HostLink`).
- [x] **Área de transferência** (texto) nos dois sentidos.
- [x] **Descoberta na LAN** (broadcast UDP na porta 9001; seção "Dispositivos na rede" na GUI e no Android; o anúncio sai por
      **todas** as interfaces IPv4 ativas — broadcast dirigido por sub-rede, crate `if-addrs` — para redes com vários adaptadores).
- [x] **Últimos acessos (recentes)** com presença, reconectar, renomear e remover (GUI Windows e Android).
- [x] **Áudio** do host (loopback, PCM 24 kHz mono) reproduzido no controller (código pronto; ver pendências).
- [x] **Multi-monitor** (seleção) e **mudança de resolução** no meio da sessão.
- [x] **Ponteiro do operador** na tela do host (sessões só de visualização).
- [x] **P2P pela internet via `iroh`**, `Node` com aceite concorrente, **presença** (`elo/ping/1`) e favoritos online/offline.
- [x] **ID curto numérico** via diretório auto-hospedável.
- [x] **CLI P2P**: `elo-client host --p2p`, `elo-client connect --p2p ... [--directory] [--password] [--code]`.
- [x] **Logs** em arquivo diário (14 dias) + aba de logs.
- [x] App gráfico único (`elo.exe`): barra lateral, Início, Favoritos, Configurações (Geral/Segurança/Conexão/Serviço/Logs/Sobre),
      **Manual** (guia de uso), tema claro/escuro, toasts, modais, faixa "X está acessando este computador", chat do host.
- [x] **Rodar em segundo plano / bandeja** (Windows): ícone na bandeja (menu Abrir Elo/Sair) e fechar-para-bandeja
      (`Config.close_to_tray`, padrão ligado), mantendo host e sessão vivos. (Compila.)
- [x] **Instalar no Windows / iniciar com o sistema em segundo plano** (Windows): cartão "Instalar no Windows" (`install.rs`)
      que, sob UAC, copia o Elo para `C:\Program Files\Elo`, cria a tarefa agendada que o inicia no logon como admin em
      `--tray` e um atalho no Menu Iniciar; **Desinstalar** desfaz tudo. (Compila.)
- [x] **Manter no topo** (Windows): botão na barra da sessão de controlador deixa a janela sempre acima das outras
      (`ViewportCommand::WindowLevel`); volta ao normal ao desconectar. (Compila.)
- [x] **Serviço do Windows + worker na sessão do console** (supervisor lança `elo.exe --session-worker` como SYSTEM; segue o
      desktop seguro/tela de bloqueio) com IPC local para aprovações/faixa/ponteiro/chat/kick; instalação configura
      `SoftwareSASGeneration=3`, permissão para Users e reinício em falha. (Compila.)
- [x] **Ctrl+Alt+Del** (`SendSas` → `SendSAS`) — requer o serviço. (Compila.)
- [x] Autostart, ícone embutido, documentação com formato binário exato e guia de porte Android.

## Não verificado em execução (implementado, compila)

Nada abaixo foi exercitado numa máquina Windows real com tela neste desenvolvimento; teste antes de confiar:

- [ ] **Serviço do Windows e worker na sessão do console** (instalação, relançamento ao trocar de sessão, fallback em processo).
- [ ] **Acesso com a tela bloqueada / UAC** (seguir o desktop seguro).
- [ ] **Ctrl+Alt+Del (SendSAS)**.
- [ ] **Áudio** (loopback WASAPI via `cpal` e reprodução).
- [ ] **GUI completa** em execução (captura DXGI, `SendInput`, multi-monitor, sobreposição do ponteiro).
- [ ] Qualidade/latência do H.264 em tela real e a troca de monitor/resolução ao vivo.

## Próximos passos

- [ ] **Verificação em runtime** num Windows real de todos os itens acima.
- [ ] **VP9/AV1** ou **codificação por hardware** (NVENC/Media Foundation) em vez do OpenH264 por software.
- [ ] **Navegador de arquivos** (UI para listar/baixar do host; hoje só há envio de arquivos avulsos nos dois sentidos, sem navegação).
- [x] **App Android — controller** ([android.md](android.md)): crate `elo-android` (UniFFI) + app Compose; conexão LAN/P2P/ID
      curto, senha/aprovação/2FA, H.264 via `MediaCodec`, áudio via `AudioTrack`, gestos (Direto/Touchpad, rolagem, zoom),
      teclado (`Char` + VK, teclas especiais), favoritos com presença, chat, envio de arquivo, área de transferência,
      multimonitor, Ctrl+Alt+Del. Verificado no emulador contra o `mock_host`; **falta testar num celular e num host Windows reais**.
- [ ] **App Android — host** (compartilhar a tela do celular via `MediaProjection`; controle só com AccessibilityService).
- [ ] App Android: serviço em primeiro plano para manter a sessão em segundo plano; build de release assinado.
- [ ] PAKE (SPAKE2/OPAQUE) no lugar de `SHA256(salt‖senha)`.
- [ ] Guardar senhas/segredo TOTP fora de texto claro (hoje em `config.json`).
- [ ] Contador de tentativas por origem, em vez de global por processo (o bloqueio já não se prolonga sozinho durante um ataque, mas ainda vale para todos).

## Verificação feita até aqui

- `cargo test`: 41/41 no Linux (e `cargo test -p elo-gui`: 3/3), incluindo o teste full-stack ponta a ponta e o do modo `raw_media`.
- App Android: `./gradlew assembleDebug` gera o APK; fluxo básico verificado no emulador (API 36) contra o `mock_host` (ver [android.md](android.md#testar-sem-um-windows)).
- `cargo build -p elo-client` e `cargo build -p elo-gui --release`: compilam nativamente no Windows (MSVC), conforme
  registrado em versões anteriores da documentação (não reexecutado ao atualizar estes textos); as partes novas do Windows (serviço/worker, áudio, SAS) **não foram executadas**.
