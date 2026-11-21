# Acesso não supervisionado, identidade fixa, pareamento e favoritos

Funções no estilo das do AnyDesk para acessar uma máquina ligada sem ninguém
presente (ex.: seu PC do trabalho, a partir de casa), com camadas de segurança
configuráveis.

## Endereço fixo (identidade persistente)

- Cada máquina tem um **endereço Elo** fixo: a chave pública do nó iroh, salva em
  `%APPDATA%\elo\node.key`. Não muda entre execuções.
- A **chave Noise do host** também é persistida (`%APPDATA%\elo\host.key`, 64 bytes
  = privada ‖ pública), então o **fingerprint** do host é estável. É mostrado em **Configurações → Sobre**.
- Mostrado na tela inicial em "Este computador"; botões **Copiar endereço** (e **Copiar ID** quando há diretório).

## Quem entra e como (regras de acesso)

Ordem de decisão no host (detalhes em [protocolo.md](protocolo.md), seção 5):

1. **Dispositivo pareado** → entra sem senha e sem aprovação.
2. **Acesso não supervisionado** (`open_access`) → entra qualquer um.
3. **Senha correta** (fixa ou da sessão) → entra sem aprovação.
4. **Sem senha** + **Pedidos de aprovação** ligados (`allow_requests`) → o dono vê um pedido
   "Permitir/Negar".
5. Caso contrário → recusado. **5 falhas em 60 s** bloqueiam novas tentativas por 1 min.

Se o **2FA** estiver ligado, o código do autenticador também é pedido nos casos 1, 2 e 3 (nunca no 4).

### Senha de conexão (fixa)

- **Configurações → Segurança → Senha de conexão**. Fica em `config.json` (`unattended_password`) e passa a
  ser a senha exigida; quem a tem entra sem aprovação.
- Sem senha fixa, o host usa uma **senha aleatória de 6 dígitos por sessão** (mostrada na tela inicial,
  com o botão **Nova** para gerar outra).
- **Senha de uso único** (`otp`): ignora a senha fixa e usa a senha da sessão, **trocada a cada sessão
  bem-sucedida** (ao ligar a opção, também gera uma nova na hora).
- A troca é **ao vivo**: o host lê a configuração atual a cada nova conexão, sem reiniciar.

### Acesso não supervisionado (`open_access`)

Sem senha e sem aprovação: o endereço vira a única proteção. A tela inicial mostra uma **faixa vermelha** de aviso
enquanto estiver ligado. Desligado por padrão.

### Pedidos de aprovação

Quem conecta **sem digitar senha** (senha vazia) gera um modal "Pedido de conexão" no host, com o nome e o
fingerprint do dispositivo e a caixa **"Confiar neste dispositivo (próximas vezes entra sem pedir)"**.
**Permitir** / **Negar**; sem resposta em 120 s = negado. Pedidos aprovados não passam por 2FA.
Ligado por padrão (`allow_requests`). A CLI `connect` também pode pedir aprovação (omita `--password`).

### Dispositivos pareados

Marcando "confiar" ao aprovar, o dispositivo (nome, fingerprint hex da chave Noise, data `AAAA-MM-DD`) entra em
`trusted` no `config.json`. Em **Segurança → Dispositivos pareados** dá para ver e **Remover**. Pareados entram
sem senha nem aprovação (o 2FA ainda é pedido, se ligado).

### 2FA (TOTP)

**Segurança → Autenticação em duas etapas**: ao ligar, a UI gera um segredo aleatório (20 bytes) e mostra um
**QR code** (URI `otpauth://`) e a chave em base32; o usuário escaneia num app (Google/Microsoft Authenticator,
Authy…) e digita o código de 6 dígitos para **Ativar** (só então é salvo: `twofa = true`, `totp_secret` em
base32). Desligar apaga o segredo. Detalhes do TOTP em [protocolo.md](protocolo.md) (5b).
Quem conecta recebe uma janela "Verificação em duas etapas" para digitar o código e a conexão é refeita.

### Controle (o que a pessoa conectada pode fazer)

`allow_control` (mouse/teclado), `show_pointer` (mostrar o ponteiro e o nome do operador na sua tela quando ele só vê),
`allow_clipboard`, `allow_audio`, `allow_files`. Mais o botão **Rodar como administrador (UAC)** (Windows; reinicia
o Elo elevado; com o serviço isso já vale).

## Favoritos (salvar endereço + senha)

- Preencha endereço + senha, dê um nome e clique **Salvar**. Há `lan` (true = IP da rede local, false = Internet).
- Os favoritos aparecem como cartões com um ponto **online / offline / verificando…**
  (verificação a cada ~45 s; ver [p2p.md](p2p.md)); **Conectar** reconecta com a senha salva; **Remover** apaga.
  Persistidos em `config.json` (a senha em texto claro).

## Iniciar com o sistema (autostart)

- **Configurações → Geral → Iniciar o Elo com o sistema**.
- Implementado via um valor `Elo` em `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
  apontando para o executável atual (`reg add`/`reg delete`). Só tem efeito no Windows.

## Acesso com a tela bloqueada — serviço do Windows

- **Recuperação de captura:** a captura DXGI trata `DXGI_ERROR_ACCESS_LOST` recriando a duplicação, e a
  thread de captura se **fixa no desktop de entrada** (`ensure_input_desktop`: `OpenInputDesktop`/`SetThreadDesktop`).
- **Serviço (`EloHost`, LocalSystem)**: **Configurações → Serviço → Instalar serviço** (pede UAC). O serviço é um
  **supervisor** que lança `elo.exe --session-worker` como **SYSTEM na sessão ativa do console**
  (`CreateProcessAsUser`), e por isso o worker **segue o desktop seguro** (tela de bloqueio, UAC). Inicia com o Windows, antes do login.
  Arquitetura completa: [arquitetura.md](arquitetura.md).
- **Estado do serviço** em `C:\ProgramData\elo`: `node.key`, `host.key`, `config.json`, `ipc.token`, `logs\`, `recebidos\`.
  Ao instalar, o Elo **copia** a sua identidade e a configuração (incluindo a senha fixa) para lá — por isso **defina a
  senha fixa e salve ANTES de instalar o serviço**. O worker **relê `config.json` a cada conexão**; a GUI grava a
  configuração nos dois lugares (a instalação dá permissão de modificação ao grupo Users).
- A instalação também define `SoftwareSASGeneration = 3` (permite o **Ctrl+Alt+Del remoto**) e configura
  **reinício em caso de falha** (`restart/5000`).
- Com o serviço instalado, **a GUI vira o painel**: pedidos de aprovação, a faixa "X está acessando este computador",
  o ponteiro do operador, chat e **Desconectar** chegam por IPC local (`127.0.0.1:47199`). **A GUI precisa estar aberta
  para aprovar pedidos**; sem ela, pedidos são negados. Sem senha fixa, o serviço não tem senha de sessão: entra
  quem for pareado, tiver a senha fixa, acesso aberto, ou for aprovado. (No serviço não há tela para mostrar uma
  senha de sessão, então `otp` é ignorado lá e vale a senha fixa.)
- Arquivos recebidos no serviço vão para `C:\ProgramData\elo\recebidos`.
- Comandos equivalentes (elevados): `elo.exe --install-service` / `elo.exe --uninstall-service`; o serviço em si roda
  via `elo.exe --service`.
- **Ressalva honesta (não verificado em execução):** o serviço, o worker na sessão do console, o acesso com a tela
  bloqueada e o Ctrl+Alt+Del são implementados e **compilam**, mas **não foram exercitados numa máquina Windows real**.
  Teste na sua máquina.

## Configuração (`config.json`)

Todos os campos têm padrão (`#[serde(default)]`); arquivos antigos continuam carregando.

| Campo | Tipo / padrão | Significado |
|-------|---------------|-------------|
| `unattended_password` | string, `""` | Senha de conexão fixa. Vazio = usa a senha da sessão. |
| `otp` | bool, `false` | Senha de uso único (a senha da sessão muda a cada acesso; ignora a fixa). |
| `open_access` | bool, `false` | Acesso não supervisionado (sem senha, sem aprovação). |
| `allow_requests` | bool, `true` | Aceitar pedidos de aprovação (conexão sem senha). |
| `twofa` | bool, `false` | 2FA ligado. |
| `totp_secret` | string base32, `""` | Segredo TOTP. |
| `trusted` | lista de `{name, fingerprint, added}` | Dispositivos pareados. |
| `allow_control` | bool, `true` | Permitir mouse/teclado. |
| `allow_clipboard` | bool, `true` | Sincronizar a área de transferência. |
| `allow_audio` | bool, `true` | Transmitir o som. |
| `allow_files` | bool, `true` | Receber arquivos. |
| `show_pointer` | bool, `true` | Mostrar o ponteiro do operador (sessão só de visualização). |
| `autostart` | bool, `false` | Iniciar com o sistema. |
| `theme` | `"dark"` \| `"light"`, `"dark"` | Tema. |
| `directory` | string, `""` | Servidor de diretório (`host:porta`) para IDs curtos. |
| `short_id` | string de 9 dígitos | ID curto desta máquina (gerado na primeira execução). |
| `display_name` | string, `""` | Nome enviado aos hosts (vazio = usuário do SO). |
| `favorites` | lista de `{name, address, lan, password}` | Favoritos. |

(Um campo antigo `approval_required`, se existir num arquivo, é ignorado.)

## Onde ficam os dados

- `%APPDATA%\elo\` (Windows) / `~/.config/elo/`: `node.key`, `host.key`, `config.json`, `logs\`.
- `C:\ProgramData\elo\`: o mesmo, para o serviço, mais `ipc.token` e `recebidos\`.
- **Logs**: um arquivo por dia, `logs\elo-AAAA-MM-DD.log`; ao iniciar, arquivos com mais de **14 dias** na pasta de logs
  são apagados. A aba **Configurações → Logs** mostra as últimas 2000 linhas (WARN em amarelo, ERROR em vermelho), com
  **Copiar**, **Limpar visualização** e **Abrir pasta**.
- Arquivos recebidos na GUI vão para a pasta Downloads do usuário (ou a pasta atual se não existir).
