# Interface gráfica e identidade

O produto é o `elo-gui` (`elo.exe`), uma janela única feita com **eframe/egui** (1180×760, mínimo 900×600).
A sessão assíncrona roda numa runtime Tokio de fundo; a thread de UI drena os frames decodificados e os
eventos do host a cada repaint e dispara o input de volta sem bloquear.

## Logo

A marca do Elo são **dois elos de corrente interligados** — "elo" em português é
justamente um elo de corrente / vínculo, a metáfora do que o app faz (ligar duas
máquinas). É desenhada vetorialmente por `paint_logo` (dois anéis traçados), e o ícone da janela/`.exe` é
gerado por `logo_pixels`.

## Tema e paleta

Dois temas semânticos (`theme::palette`), escolhidos em Configurações → Geral e aplicados a cada frame:

| Token | Escuro | Claro | Uso |
|-------|--------|-------|-----|
| `bg` | `#0F141B` | `#F5F7FA` | fundo |
| `bar` / `surface` / `elevated` | `#121821` / `#181F29` / `#202935` | `#FFFFFF` / `#FFFFFF` / `#F0F2F6` | barra lateral, cartões, blocos |
| `accent` | teal `#2DD4BF` | `#12B3A6` | destaque, botão Conectar |
| `accent2` | `#8B97F8` | `#4F55E0` | cartões informativos |
| `warn` / `danger` / `ok` | `#ECB44E` / `#F26D72` / `#4ADE80` | `#C87A12` / `#D1444A` / `#1FA25A` | avisos, perigo, online |

## Estrutura: barra lateral + páginas

- **Barra lateral** (212 px, `draw_shell`): logo + "Elo"; navegação **Início**, **Favoritos**,
  **Configurações**; no rodapé, ponto de status (online / iniciando…), "serviço do Windows ativo" (quando
  aplicável) e a versão.
- **Faixa de sessão do host** (topo, vermelha, quando alguém está conectado a *este* computador):
  "**X está acessando este computador · N min**", fingerprint curto, botões **Chat** e **Desconectar**.
- **Início** (`page_home`):
  - **Faixa de aviso vermelha** quando o **acesso não supervisionado** está ligado.
  - **Conectar a outro computador**: alternador **Internet / Rede local**, campo de endereço (ID/endereço Elo, ou IP),
    senha ("senha (opcional)" — sem senha, o outro computador recebe um pedido de aprovação), **Conectar**.
    Mostra "conectando… (se pediu aprovação, aguarde o outro lado aceitar)" e erros amigáveis.
  - **Este computador** (ou "…(serviço do Windows)"): com diretório configurado mostra o **ID curto** de 9 dígitos
    (com **Copiar ID**) e o **endereço completo** (node id) com **Copiar endereço**; linha de senha — senha fixa
    definida / **Senha desta sessão** (ou **Senha de uso único**) com o botão **Nova**; no modo serviço mostra
    um texto explicando como se entra; e "Rede local: IP:9000".
  - Três cartões informativos (acesso não supervisionado, Internet sem servidor, dispositivos pareados).
- **Favoritos** (`page_favorites`): formulário (nome, Internet/Rede local, endereço, senha, **Salvar**) e a lista de
  cartões com ponto **online/offline/verificando…** (presença a cada ~45 s), **Conectar** e **Remover**.
- **Configurações** (`page_settings`): abas à esquerda e conteúdo à direita; **Salvar** persiste e **Desfazer** reverte
  (aviso "alterações não salvas").
  - **Geral**: *Conta* (avatar, usuário do SO, nome do computador, **Nome exibido** — o que o dono do host vê no
    pedido de aprovação), *Aparência* (tema Claro/Escuro, aplicado na hora), *Inicialização* (iniciar com o sistema).
  - **Segurança**:
    - *Acesso*: **Acesso não supervisionado** (com aviso), **Senha de conexão**, **Senha de uso único**, **Pedidos de
      aprovação**, **Autenticação em duas etapas (2FA)** com cadastro por **QR code** + chave base32 + confirmação do
      código de 6 dígitos (aplicado imediatamente).
    - *Dispositivos pareados*: lista (nome, fingerprint curto, data) com **Remover** (imediato).
    - *Controle*: **Permitir controlar mouse e teclado**, **Mostrar o ponteiro de quem está vendo**, **Área de
      transferência compartilhada**, **Transmitir o som deste computador**, **Receber arquivos**, e (Windows)
      **Rodar como administrador (UAC)** → *Reiniciar como admin*.
  - **Conexão**: P2P automático (nada a configurar), **ID curto (diretório)** (servidor `host:porta`; reiniciar o
    Elo ao mudar) e a porta LAN (TCP 9000).
  - **Serviço** (Windows): estado Instalado/Não instalado, **Instalar serviço** / **Desinstalar serviço** (UAC) e **Logs do
    serviço**. Em outros SOs: "disponível apenas no Windows".
  - **Logs**: as últimas linhas, **Abrir pasta**, **Copiar**, **Limpar visualização** (arquivos diários, 14 dias).
  - **Sobre**: versão, descrição, **impressão digital** deste computador (chave Noise do host — a do serviço, se instalado),
    **Abrir pasta de dados** e **Abrir pasta de logs**.

## Modais, avisos e sobreposições

- **Pedido de conexão** (modal): avatar + "X quer acessar este computador.", fingerprint do dispositivo, caixa
  **"Confiar neste dispositivo (próximas vezes entra sem pedir)"**, **Negar** / **Permitir**. Vários pedidos entram numa fila.
- **Verificação em duas etapas** (modal no lado que conecta): aparece quando o host pede 2FA e nenhum código foi dado;
  campo de 6 dígitos, **Cancelar** / **Conectar** (refaz a conexão com o código).
- **Toasts** (canto inferior direito, 6 s): "X conectou a este computador.", "X desconectou.", "Acesso recusado: X (senha
  incorreta | código 2FA incorreto | muitas tentativas (bloqueado por 1 min) | recusado)", "Arquivo recebido: …", etc.
- **Ponteiro do operador**: quando o operador **não pode controlar** (view-only) e `show_pointer` está ligado, o host
  desenha uma janela transparente, sem decoração, sempre no topo e que ignora o mouse sobre o monitor capturado, com a
  seta do operador e o **nome** dele (some após 4 s sem atualização).
- **Chat do host**: janela "Chat — nome" aberta pela faixa ou ao receber mensagem.

## Sessão ativa (`drive_session`)

Quando uma conexão de saída fica ativa (a barra lateral some; a janela mostra a tela remota):

- **Barra de ferramentas**: ponto verde, rótulo do alvo, **fingerprint** do computador remoto (dica: conferir com
  Configurações → Sobre lá), "somente visualização" se não houver controle, seletor de **Monitor** (aparece com mais de 1 monitor),
  **Ctrl+Alt+Del** (só com controle; requer o serviço do Windows no outro computador), **Som: ligado/mudo**,
  **Tela cheia** (também **F11**; o Esc não sai da tela cheia, é enviado ao remoto), **Chat**, **Desconectar**.
- **Chat** (painel direito) e aviso de "arraste arquivos para a tela para enviar" — **arrastar e soltar** envia o arquivo
  (cada arquivo com um id crescente; lido inteiro em memória).
- **Central**: a tela remota como textura (BGRA→RGBA), escalada para caber. Com controle: mouse/roda/teclas viram
  `InputEvent` (modificadores Shift/Ctrl/Alt por diferença de estado, teclas por VK via `keymap.rs`, texto não ASCII como
  `Char`). Sem controle: o ponteiro local é enviado ao host (`Pointer`, no máximo a cada ~40 ms).
- Ao receber `ScreenChanged`/`Monitors`, a barra e o tamanho do quadro acompanham; o tamanho usado no mapeamento do mouse é
  o do último quadro recebido.

## Papéis na mesma janela

Ao abrir, o app já é **host**: gera/mostra o endereço e a senha, detecta o IP da LAN, escuta TCP 9000 e aceita P2P.
Ao mesmo tempo pode agir como **controller**. Com o serviço instalado, o host passa a ser o worker do serviço e esta
janela é seu painel. Um binário, os dois papéis.

> Nota de identidade: a estrutura segue convenções comuns do gênero, mas a marca, a logo e as cores são
> próprias do Elo. Não reproduzimos a identidade visual de nenhum produto de terceiros.
