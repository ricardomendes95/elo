# Status e roadmap

## Pronto e testado (37 testes passam no Linux)

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
- [x] Input remoto (mouse/teclado/roda, **texto Unicode** via `Char`); chat; transferência de arquivo (arrastar e soltar).
- [x] **Área de transferência** (texto) nos dois sentidos.
- [x] **Áudio** do host (loopback, PCM 24 kHz mono) reproduzido no controller (código pronto; ver pendências).
- [x] **Multi-monitor** (seleção) e **mudança de resolução** no meio da sessão.
- [x] **Ponteiro do operador** na tela do host (sessões só de visualização).
- [x] **P2P pela internet via `iroh`**, `Node` com aceite concorrente, **presença** (`elo/ping/1`) e favoritos online/offline.
- [x] **ID curto numérico** via diretório auto-hospedável.
- [x] **CLI P2P**: `elo-client host --p2p`, `elo-client connect --p2p ... [--directory] [--password] [--code]`.
- [x] **Logs** em arquivo diário (14 dias) + aba de logs.
- [x] App gráfico único (`elo.exe`): barra lateral, Início, Favoritos, Configurações (Geral/Segurança/Conexão/Serviço/Logs/Sobre),
      tema claro/escuro, toasts, modais, faixa "X está acessando este computador", chat do host.
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
- [ ] **Navegador de arquivos** (UI para listar/baixar do host; hoje só há envio por arrastar-e-soltar controller → host).
- [ ] **App Android** em si (o guia está em [porting-android.md](porting-android.md); o núcleo precisa de ajustes de `cfg` e eventos
      para H.264/áudio crus).
- [ ] Transferência do host → controller iniciada pelo host (o receptor existe no controller, mas o host não envia arquivos).
- [ ] PAKE (SPAKE2/OPAQUE) no lugar de `SHA256(salt‖senha)`.
- [ ] Guardar senhas/segredo TOTP fora de texto claro (hoje em `config.json`).
- [ ] Contador de tentativas por origem, em vez de global por processo (o bloqueio já não se prolonga sozinho durante um ataque, mas ainda vale para todos).
- [ ] Descoberta automática na LAN.

## Verificação feita até aqui

- `cargo test`: 37/37 no Linux, incluindo o teste full-stack ponta a ponta.
- `cargo build -p elo-client` e `cargo build -p elo-gui --release`: compilam nativamente no Windows (MSVC), conforme
  registrado em versões anteriores da documentação (não reexecutado ao atualizar estes textos); as partes novas do Windows (serviço/worker, áudio, SAS) **não foram executadas**.
