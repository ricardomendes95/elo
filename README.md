# Elo — acesso remoto (original)

> Documentação completa em [`docs/`](docs/README.md): arquitetura, protocolo (formato binário exato), segurança,
> referência de funções, build/uso, interface, P2P, acesso não supervisionado, porte para Android e roadmap.

Ferramenta original de acesso remoto (ver a tela e controlar outra máquina, com áudio, área de transferência, chat e
transferência de arquivos), escrita em Rust. Inspirada no *conjunto de recursos* de ferramentas comerciais, mas com
protocolo, código e design próprios. Não é um clone, nem é compatível com nenhum produto existente.

## Como funciona

- **Internet (P2P, sem servidor seu):** cada máquina tem um **endereço Elo** (id de nó iroh). iroh faz NAT hole punching e
  usa relays gratuitos da n0 só como reserva. Opcionalmente um **diretório** (`elo-client relay`) traduz um **ID curto de 9
  dígitos** para o endereço; o vídeo continua P2P.
- **Rede local:** TCP direto na porta 9000 do host.
- **Relay próprio (opcional, CLI):** `elo-client relay` pareia por ID de 9 dígitos e copia bytes cegamente.
- **Criptografia ponta a ponta:** Noise XX (`25519_ChaChaPoly_BLAKE2s`); o relay/diretório nunca vê a sessão.

## Recursos (protocolo v2)

- **Acesso:** pareado → acesso aberto → senha → **pedido de aprovação** (Permitir/Negar) → recusa; **2FA (TOTP)** opcional;
  limite de 5 falhas em 60 s; dispositivos pareados ("confiar neste dispositivo").
- **Vídeo:** **H.264** (OpenH264, Annex-B) negociado por `Caps`, com *fallback* para o codec de tiles + zlib; mudança de
  resolução e **multi-monitor** durante a sessão.
- **Áudio** do host (loopback no Windows, PCM 24 kHz mono), **área de transferência** de texto, chat, **envio de arquivos**
  (arrastar e soltar), **Ctrl+Alt+Del** (exige o serviço), **ponteiro do operador** na tela do host, texto Unicode.
- **Windows:** app de janela única (`elo.exe`, portátil), e um **serviço** que lança um *worker* SYSTEM na sessão do console para
  funcionar com a **tela bloqueada**; a GUI vira o painel (aprovações, faixa "X está acessando este computador", chat, desconectar).

## Crates

| crate | roda em | o que é |
|-------|---------|---------|
| `elo-protocol` | qualquer | tipos de fio: controle do relay/diretório, mensagens de sessão v2, framing |
| `elo-session` | qualquer | Noise XX cifrado, com framing |
| `elo-relay` | Linux/qualquer | rendezvous + diretório + relay cego |
| `elo-p2p` | qualquer | transporte iroh (QUIC), `Node`, presença |
| `elo-client-core` | qualquer¹ | codecs (tiles e H.264), áudio, clipboard, TOTP, arquivos, laços de host/controller; backend Windows (DXGI + `SendInput`) |
| `elo-client` | Windows¹ | CLI de dev/headless |
| `elo-gui` | Windows¹ | **o produto** (`elo.exe`): ambos os papéis, serviço e worker |

¹ O papel de **controller** roda em qualquer SO; o de **host** precisa do Windows (captura/injeção `#[cfg(windows)]`).

## Estado

`cargo test` passa (37 testes no Linux, incluindo um teste full-stack com relay real, tela simulada e arquivo de 50 KB).
O serviço/worker, o áudio, o acesso com a tela bloqueada e o Ctrl+Alt+Del estão **implementados mas ainda não verificados em
execução num Windows real** — veja o [roadmap](docs/roadmap.md).

## Build e uso

```sh
cargo test                                # núcleo, no Linux/WSL
cargo run -p elo-client -- relay --bind 0.0.0.0:9000   # relay/diretório
```

Windows (app portátil):

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build -p elo-gui --release
# -> target\release\elo.exe  (sem instalador)
```

CLI (headless):

```sh
elo-client host    --p2p --password PW                                    # Windows; imprime o endereço Elo
elo-client connect --p2p ENDEREÇO|ID [--directory HOST:PORTA] [--password PW] [--code 2FA]   # sem senha = pedido de aprovação
elo-client host    --listen 0.0.0.0:9000 --password PW                    # direto (Windows)
elo-client connect --direct IP:9000 --password PW
```

Mais detalhes em [`docs/build-e-uso.md`](docs/build-e-uso.md).

## Escopo / ética

Software original para suporte remoto legítimo e acesso às suas próprias máquinas. Não é modificação, patch nem burla de
licença de nenhum produto de terceiros.
