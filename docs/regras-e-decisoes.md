# Regras do projeto e decisões de design

## Regras / princípios

1. **Software original.** Elo não copia, modifica nem burla nenhum produto de
   terceiros. Protocolo, código, marca e identidade visual são próprios. Usa APIs
   nativas do SO (DXGI, `SendInput`), que são públicas e comuns ao gênero.
2. **O relay é cego.** Nenhum servidor jamais vê conteúdo de sessão em claro. A
   confidencialidade é ponta a ponta entre os dois clientes.
3. **Sem servidor quando possível.** Na LAN ou com host alcançável, a conexão é
   direta, sem intermediário. O relay só entra quando a rede (NAT dos dois lados)
   exige.
4. **Núcleo testável sem Windows.** Toda lógica independente de plataforma compila e
   é testada no Linux via traits + mocks; só captura/input reais são `#[cfg(windows)]`.
5. **Sem `unsafe` fora do backend.** `unsafe` só no `platform.rs` (Win32), em blocos
   pequenos e documentados.
6. **Entradas hostis são limitadas.** Todo frame tem teto de tamanho; transferência
   de arquivo valida offsets/limite; nomes de arquivo são saneados.

## Decisões de design (e por quê)

### Rust
Memória segura, ótimo ecossistema de cripto/rede, e um único `.exe` estático
portátil (`+crt-static`) sem runtime a instalar — o requisito de "portátil como o
AnyDesk".

### Noise XX como transporte cifrado
XX transmite as chaves estáticas de ambos os lados, o que permite **pinar** a chave
contra a que o relay anuncia (anti-MITM) e expor um **fingerprint** no modo direto.
ChaCha20-Poly1305 + BLAKE2s são rápidos e sólidos.

### Dois codecs: H.264 (OpenH264) e tile-delta + zlib
O **H.264** (OpenH264, Annex-B, perfil de conteúdo de tela) é o preferido por comprimir muito melhor telas reais; é negociado
por `Caps` e depende de o controller conseguir decodificar. O **codec de tiles + zlib** continua como alternativa e como
*fallback* (inclusive no meio da sessão, se o encode H.264 falhar): é 100% Rust, sem dependência nativa pesada, e é o que
roda em qualquer SO. Ambos ficam atrás de `VideoEncoder` no host.

### Decisão de acesso explícita e em ordem
Pareado → acesso aberto → senha → aprovação do usuário → recusa. O 2FA vale para os três primeiros e não para a aprovação
(o usuário já está vendo quem pede). Falhas são limitadas (5 em 60 s).

### Serviço como supervisor + worker na sessão do console
Um serviço na sessão 0 não alcança a tela do usuário. Em vez de hospedar nele, o serviço lança um *worker* SYSTEM na sessão
ativa do console, que segue o desktop seguro; GUI e worker conversam por IPC local autenticado por token. O laço de host é o
mesmo (`hostrun.rs`) para a GUI e o worker.

### Relay separado + embutido no binário
Fisicamente, com NAT dos dois lados é impossível fugir de um intermediário com IP
público (é o que a nuvem do AnyDesk faz). Em vez de um crate/exe à parte, o relay é
um **subcomando** do mesmo binário (`elo-client relay`), então continua sendo um
artefato só.

### Modo direto (LAN/IP) por padrão
Resolve o caso mais comum (mesma rede) sem servidor nenhum e sem configuração —
evita o erro "conexão recusada" de apontar para um relay inexistente.

### GUI com runtime Tokio de fundo
A thread do egui nunca bloqueia em rede: o host roda em tarefas de fundo; a sessão
de saída entrega eventos por canal `mpsc` que a UI drena a cada frame, e o input é
disparado com `rt.spawn` (fire-and-forget).

### Captura e injeção atrás de traits
`Capture` e `InputSink` desacoplam os laços de sessão do SO. Produção usa DXGI/
`SendInput`; testes usam `MockCapture`/`RecordingInput`. É o que torna o teste
end-to-end possível no Linux.

## Convenções de código

- Workspace Cargo; versões/edição/licença herdadas de `[workspace.package]`.
- Dependências comuns em `[workspace.dependencies]`.
- `elo-gui` fora de `default-members` (não quebra `cargo test` por falta de libs de
  display).
- Mensagens de erro e limitações nunca são escondidas; o que não foi verificado é
  dito explicitamente.
