# Documentação do Elo

Elo é um aplicativo **original** de acesso remoto (ver a tela e controlar outra
máquina, com áudio, área de transferência, chat e transferência de arquivos), escrito em Rust. Entregue como
**um único executável portátil** (`elo.exe`), sem instalador, e como **app Android** (controller; ver [android.md](android.md)).

> Elo não é uma cópia, modificação nem burla de nenhum produto comercial. Usa as
> mesmas APIs nativas do Windows que ferramentas do gênero usam (captura de tela
> via DXGI, injeção de input via `SendInput`), mas com protocolo, código e
> identidade próprios.

## Índice

| Documento | Conteúdo |
|-----------|----------|
| [arquitetura.md](arquitetura.md) | Visão geral, componentes, crates, módulos do app, serviço do Windows + worker + IPC, fluxo de uma sessão |
| [protocolo.md](protocolo.md) | Protocolo de fio **v2** + **formato binário exato** (framing, bincode, índices de variantes, exemplos de bytes), ordem do handshake, decisão de acesso, vídeo H.264, áudio, TOTP |
| [p2p.md](p2p.md) | Transporte P2P pela internet via iroh (hole punching, `Node`, presença, ID curto) |
| [acesso-nao-supervisionado.md](acesso-nao-supervisionado.md) | Regras de acesso, senha, aprovação, pareamento, 2FA, favoritos, autostart, serviço/tela bloqueada, campos do `config.json`, logs |
| [android.md](android.md) | **App Android** (controller): estrutura, build (cargo-ndk + Gradle), telas, gestos, teclado, MediaCodec/AudioTrack, API UniFFI, testes e limites |
| [porting-android.md](porting-android.md) | Guia para portar para Android/APK (núcleo Rust via NDK, `MediaCodec`, `AudioTrack`, `Char`, handshake v2) |
| [seguranca.md](seguranca.md) | Modelo de segurança: Noise, pinagem, decisão de acesso, 2FA, limite de tentativas, limitações |
| [referencia-funcoes.md](referencia-funcoes.md) | Referência de módulos e funções públicas por crate |
| [build-e-uso.md](build-e-uso.md) | Como compilar (Linux/Windows), rodar e testar |
| [interface.md](interface.md) | A interface gráfica (barra lateral, Início, Favoritos, Configurações, modais, sessão), tema e logo |
| [regras-e-decisoes.md](regras-e-decisoes.md) | Regras do projeto e decisões de design (e por quê) |
| [roadmap.md](roadmap.md) | O que está pronto e o que falta |

## Resumo em uma tela

```
  Host (máquina controlada)          Relay (opcional)            Controller (operador)
  ─────────────────────────         ──────────────            ─────────────────────
  captura DXGI ─► codec ─► Noise ═══╗                      ╔═══ Noise ─► decodifica ─► janela
  SendInput  ◄── input  ◄── Noise ══╬═ rendezvous + cópia ═╬══ Noise ◄── mouse/teclado
                                    ║   cega de bytes       ║
                 ══════ sessão cifrada ponta a ponta (o relay não vê nada) ══════
```

- **Na mesma rede (LAN):** conexão direta por IP, **sem relay nenhum**.
- **Pela internet:** P2P via iroh (hole punching, com relays gratuitos de reserva), por endereço Elo ou ID curto
  (diretório opcional). Alternativa self-hosted: relay embutido (`elo-client relay`), que só faz o encontro e copia bytes cegos.

Versão documentada: Elo `0.1.0`, protocolo v2. Itens implementados mas **ainda não verificados em execução** num Windows
real (serviço/worker, áudio, tela bloqueada, Ctrl+Alt+Del) estão listados em [roadmap.md](roadmap.md).
