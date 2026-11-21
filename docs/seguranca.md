# Modelo de segurança

## Camadas

1. **Cifra ponta a ponta (Noise XX).** Toda a sessão (tela, áudio, input, área de
   transferência, chat, arquivos) trafega dentro de um túnel
   `Noise_XX_25519_ChaChaPoly_BLAKE2s`:
   - handshake XX (3 mensagens) → ambos os lados aprendem a chave estática do outro;
   - transporte AEAD (ChaCha20-Poly1305) com nonces independentes por direção.
   Implementado em `crates/session/src/lib.rs`. O host aborta se o handshake passar de 20 s.

2. **Pinagem de chave (anti-MITM via relay).** No modo relay, o servidor anuncia a
   chave pública de cada par (`ControlMsg::Paired`). Depois do handshake, cada lado
   compara `enc.remote_static()` com essa chave; se diferirem, aborta. Assim um
   relay malicioso não consegue se passar por uma das pontas.

3. **Identidade do dispositivo = chave estática Noise.** O host conhece, depois do
   handshake, o **fingerprint** do controller (hex da chave estática de 32 bytes). Ele é
   estável por dispositivo e é a base do **pareamento** (dispositivos confiáveis).

4. **Decisão de acesso no host** (ver [protocolo.md](protocolo.md), seção 5), nesta ordem:
   dispositivo pareado → acesso aberto → senha correta → senha vazia + aprovação do usuário →
   rejeição. A prova de senha é `SHA256(salt ‖ senha)` com `salt` aleatório de 16 bytes por conexão,
   dentro do canal já cifrado.

5. **Segundo fator (2FA / TOTP).** Opcional, RFC 6238 (HMAC-SHA1, 30 s, 6 dígitos, tolerância de
   ±1 passo). Exigido depois de pareado/aberto/senha correta; **não** exigido quando o usuário do host
   aprova o pedido na hora. Segredo em base32 em `config.json` do host (ver "Limitações").

6. **Fingerprint do host (modo direto/P2P).** O `ControllerHandle` expõe `peer_fingerprint` (a chave
   estática do host), mostrado na barra da sessão e em **Configurações → Sobre** no host, para
   conferência manual (TOFU).

## Limite de tentativas

Falhas de autenticação (senha errada, pedido negado ou sem aprovador, código 2FA errado) vão para um
contador **global do processo**. Com **5 falhas em 60 s**, o host responde `AuthFailed` a novas conexões
(já depois do `Hello`) até a janela de 60 s esvaziar. Cada falha também espera **1 s** antes da resposta.
O contador vale por processo host (o worker do serviço e a GUI têm contadores separados).

## O relay é cego

O relay (`elo-relay`) só:
- parseia o handshake de controle (`ControlMsg`) para parear por ID e para o diretório de IDs curtos;
- depois copia bytes crus entre os dois sockets (`copy_bidirectional`).

Ele **nunca** vê tela, input, chat ou arquivos em claro. O diretório só conhece
`ID curto → node id`.

## Permissões

O host decide o que o controller pode fazer (`Permissions`), enviado logo após a autenticação.
Na GUI vêm da configuração:

- `control` (`allow_control`) — injetar mouse/teclado, Ctrl+Alt+Del; se falso, é sessão só-visualização
  (e o ponteiro do operador pode ser mostrado no host, se `show_pointer`);
- `file_transfer` (`allow_files`) — aceitar arquivos (senão o host responde `FileCancel`);
- `clipboard` (`allow_clipboard`) — sincronizar texto da área de transferência;
- `audio` (`allow_audio`) — transmitir o som do host.

Perfis prontos: `Permissions::VIEW_ONLY` e `Permissions::FULL`. A CLI usa `FULL`; a GUI usa os
interruptores de **Configurações → Segurança → Controle**.

## Endurecimento contra entradas hostis

- **Framing limitado:** todo frame tem teto (`MAX_FRAME`/`MAX_LOGICAL` = 16 MiB);
  tamanhos maiores são rejeitados antes de alocar.
- **Transferência de arquivo validada:** `IncomingFile::push` exige chunks
  contíguos e dentro do tamanho declarado (impede escrita fora dos limites).
- **Nome de arquivo saneado:** host e controller removem componentes de caminho do nome
  recebido (`sanitize`/`file_name`), evitando `../` para escapar da pasta de download.
- **Clipboard limitado** a 1 MiB de texto.
- **Nome do operador** truncado em 64 caracteres no host.
- **Captura só depois de aceitar:** o backend de captura/input só é criado depois de o par ser aceito
  (sondagens e pares recusados não ativam a captura).
- **IPC local autenticado** (GUI ↔ worker do serviço): escuta só em `127.0.0.1` e exige um token
  (`ipc.token`) na primeira mensagem. Ver [arquitetura.md](arquitetura.md).
- **Sem `unsafe`** no núcleo (protocol/session/client-core puro). `unsafe` existe
  só no backend Windows (`platform.rs`) e no serviço (`service.rs`), em blocos pequenos e
  documentados ao redor das chamadas Win32.

## Acesso aberto

`open_access` ("acesso não supervisionado" na UI) aceita **qualquer** pessoa que alcance o endereço, sem
senha nem aprovação (com 2FA, se ligado). A tela inicial mostra uma faixa vermelha de aviso enquanto estiver
ligado. Desligado por padrão.

## Limitações conhecidas (honestas)

- **Não verificado em execução numa máquina Windows com tela:** captura DXGI, `SendInput`, a GUI, o
  **serviço do Windows e o worker na sessão do console**, o **áudio (loopback)**, o acesso com a **tela
  bloqueada** e o **Ctrl+Alt+Del (SendSAS)** foram implementados e compilam, mas ainda precisam de
  verificação no ambiente real. O que roda no Linux (testes) é o núcleo: protocolo, Noise, codecs, host/
  controller com captura simulada.
- A prova de senha é `SHA256(salt‖senha)` dentro do canal autenticado — adequada
  aqui, mas não é um PAKE; para robustez extra pode-se adotar SPAKE2/OPAQUE.
- A senha fixa, as senhas dos favoritos e o segredo TOTP ficam em **texto claro** em `config.json`
  (a pasta do serviço, `C:\ProgramData\elo`, dá permissão de modificação ao grupo Users para que a GUI
  atualize a configuração).
- O contador de tentativas é por processo, não por origem: um atacante pode bloquear o acesso legítimo
  por 1 minuto (negação de serviço leve) em troca de proteger contra força bruta.
- O SAS (`SendSAS`) exige a política `SoftwareSASGeneration = 3`, que a instalação do serviço configura
  (afeta a política do sistema).
- `SendInput` não injeta em janelas elevadas se o host não estiver elevado (UIPI); com o serviço (SYSTEM)
  isso não se aplica.
