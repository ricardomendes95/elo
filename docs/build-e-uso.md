# Build e uso

## Pré-requisitos

- **Rust estável** (instale via <https://rustup.rs>).
- Para o app/host Windows: toolchain **MSVC** (`x86_64-pc-windows-msvc`) com
  Visual C++ Build Tools + Windows 10 SDK.

## Testes (Linux/WSL)

Todo o núcleo independente de plataforma é testado no Linux:

```sh
cargo test
```

Cobre 41 testes (no estado atual): protocolo, sessão (handshake + framing + concorrência), codecs (tiles e
H.264), TOTP (vetores da RFC 6238), DSP de áudio, transferência, descoberta na LAN (parse de anúncio), relay (e2e),
decisão de acesso (aprovação, pareamento, 2FA, kick), o modo `raw_media` do controller (usado pelo app Android) e o **teste full-stack** em
`crates/client-core/src/e2e_tests.rs` — sobe um relay real, dois clientes,
tela sintética e injetor gravado, e confere que frame, input, chat e um arquivo
chegam de ponta a ponta. Os 3 testes de configuração da GUI rodam com `cargo test -p elo-gui`.

> O H.264 usa o **OpenH264 compilado do código-fonte** (crate `openh264`), então o primeiro build precisa de
> um compilador C/C++ (no Windows, as Visual C++ Build Tools; no Linux, `gcc`/`g++`).
> A área de transferência usa `arboard`, e o áudio (Windows) usa `cpal` (WASAPI).

> O crate `elo-gui` fica fora dos `default-members` (puxa stack de janela), então
> `cargo test`/`cargo build` sem alvo não o compilam. Rode-o com `-p elo-gui`.

## Rodar o relay (opcional, só para internet com NAT)

```sh
cargo run -p elo-relay -- 0.0.0.0:9000
# ou, pelo binário único:
cargo run -p elo-client -- relay --bind 0.0.0.0:9000
```

Hospede num host alcançável pelas duas pontas (um VPS). Para LAN **não precisa**.

## Compilar o app portátil (Windows)

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build -p elo-gui --release
# -> target\release\elo.exe   (copie para qualquer lugar; não precisa instalar)
```

O `+crt-static` embute o runtime do Visual C++, então o `.exe` roda numa máquina
Windows limpa, dependendo só de DLLs do próprio sistema (sem `vcruntime140.dll`).

### Compilar a partir do WSL via PowerShell

Há um toolchain Rust MSVC no Windows. A partir do WSL dá para compilar assim
(copiando o código para um caminho no `C:`, porque build em `\\wsl$` é problemático):

```sh
DST="/mnt/c/Users/<voce>/elo-win"
rm -rf "$DST/crates"; cp -r crates "$DST/"; cp Cargo.toml Cargo.lock "$DST/"
powershell.exe -NoProfile -Command "cd 'C:\Users\<voce>\elo-win'; \
  \$env:RUSTFLAGS='-C target-feature=+crt-static'; cargo build -p elo-gui --release"
```

## Usar o app

Rode `elo.exe` nas duas máquinas.

- **Internet (P2P):** na máquina de destino, copie o **endereço Elo** (ou o **ID curto**, se um diretório estiver
  configurado em Configurações → Conexão). Na outra, escolha **Internet**, cole o endereço, digite a senha e
  **Conectar**. **Sem senha**, o dono do destino recebe um pedido para **Permitir/Negar**. Se o destino tem 2FA, uma janela
  pede o código de 6 dígitos.
- **Mesma rede (LAN):** escolha **Rede local**, digite o IP do outro computador (porta 9000 por padrão; use `IP:porta` para outra) e conecte.
- **Relay próprio (opcional, CLI):** veja abaixo.
- **Serviço do Windows (tela bloqueada, Ctrl+Alt+Del):** Configurações → Serviço → Instalar serviço (UAC). Defina e salve a
  senha fixa **antes** de instalar. Veja [acesso-nao-supervisionado.md](acesso-nao-supervisionado.md).
- **Logs:** Configurações → Logs, ou a pasta `%APPDATA%\elo\logs` (serviço: `C:\ProgramData\elo\logs`).

## CLI (headless)

Para testar sem interface:

```sh
# relay/diretório próprio (qualquer SO):
elo-client relay --bind 0.0.0.0:9000

# host direto (Windows), sem relay:
elo-client host --listen 0.0.0.0:9000 --password 123456
# controller (qualquer SO) conectando direto:
elo-client connect --direct 192.168.0.10:9000 --password 123456

# host via relay (Windows) e controller:
elo-client host    --relay RELAY:9000 [--id 123456789] --password 123456
elo-client connect --relay RELAY:9000 --id 123456789 --password 123456

# P2P pela internet:
elo-client host --p2p --password 123456            # (Windows) imprime o endereço Elo
elo-client connect --p2p ENDEREÇO_OU_ID [--directory HOST:PORTA] [--password PW] [--code 123456]
```

- `connect --p2p`: se o alvo for um número de 9 dígitos, é obrigatório `--directory` (resolve o ID curto).
  **`--password` é opcional**: sem ele a CLI envia senha vazia e o host recebe um **pedido de aprovação**
  (a CLI `host --p2p` não tem aprovador; só o app com GUI tem). `--code` é o código 2FA.
- O controller da CLI é headless (imprime estatísticas de quadros e repassa o stdin como chat).
- O host da CLI usa permissões completas, 30 fps e salva arquivos recebidos na pasta atual.

## App Android

O app Android (controller) é construído com `cargo-ndk` + Gradle; instruções completas, pré-requisitos e como
testar no emulador contra o host de teste estão em [android.md](android.md#build). Resumo:

```sh
cd android
./gradlew assembleDebug    # compila crates/android para arm64-v8a, gera os bindings UniFFI e o APK
# -> android/app/build/outputs/apk/debug/app-debug.apk

# host de teste (tela sintética) para o emulador, que conecta em 10.0.2.2:
cargo run --release -p elo-client-core --example mock_host
```

