# rnetrbin: regras por pasta, usuário que apagou e interface estilo Undelete

Data: 2026-10-14. Branch: `feat/driver-install`.

## Objetivo

O administrador (só ele, no próprio servidor, por RDP) escolhe pastas a proteger e,
para cada uma, quais arquivos vão para a lixeira (Incluir) e quais não vão (Excluir).
Na janela ele vê, por pasta, os arquivos apagados com quem apagou, e restaura.

Critérios de sucesso:

- Com Incluir `*.*` e Excluir `*.bak` numa pasta, todo arquivo apagado nela e nas
  subpastas vai para a lixeira, menos os `.bak`.
- A lista mostra `DOMÍNIO\usuário` de quem apagou, ou "desconhecido".
- Proteger uma pasta com 2 milhões de arquivos não reescreve nada nos arquivos.
- A janela segue o modelo do Undelete Server (faixa de abas, árvore à esquerda,
  lista à direita), com ícones Material Icons, tema Rust por padrão.
- A lista navega como o Explorer: mostra só o que está direto na pasta atual e
  clicar numa subpasta entra nela.

## Fatos que sustentam o design

Lidos do `netrbin.sys` (ver wiki "rnetrbin - driver netrbin e instalação standalone"):

- Para cada arquivo apagado (não diretório) o driver monta o caminho DOS completo
  (`D:\Dados\x\a.docx`) e guarda o arquivo se ele casar com alguma máscara de
  inclusão e com nenhuma de exclusão, nessa ordem.
- A comparação é `FsRtlIsNameInExpression` com IgnoreCase: a máscara precisa estar
  em maiúsculas; `*` atravessa `\`.
- O driver não registra quem apagou (o arquivo de sinalização é vazio).

Medidos neste PC (Windows 10 Pro, `bench-audit.ps1`, 1.200 operações locais, mediana
de 3 rodadas):

- Auditoria de exclusão: +45 µs por operação (+14% a +19% numa carga só de
  excluir/renomear); CPU de lsass e do serviço de log abaixo da resolução de medida.
- 1,83 evento por operação (4663 em toda exclusão e renomeação, 4660 em toda
  exclusão; nenhum 4656), ~985 bytes por evento no `.evtx`.
- Buscar um evento por caminho no log de Segurança: ~220 ms (sem índice).
- O evento 4663 traz `SubjectDomainName\SubjectUserName` (ex.: `AUSTER2\joao.silva`).

Documentado: aplicar uma regra de auditoria herdada a uma pasta reescreve o descritor
de segurança de cada arquivo existente (horas em árvores grandes).

## 1. Configuração

`C:\ProgramData\rnetrbin\config.toml`:

```toml
recycle_dir = 'E:\Lixeira'
db_path = 'C:\ProgramData\rnetrbin\index.db'
max_bin_bytes = 10737418240
max_age_days = 30            # opcional
theme = "rust"               # rust | branco | preto

[[folders]]
path = 'D:\Dados\Contabil'
include = ["*.*"]
exclude = ["*.bak", "*.tmp", "~$*"]

[[folders]]
path = 'D:\Dados\Fiscal'
include = ["*.xml", "*.pdf"]
exclude = []
```

Substitui os campos atuais `folders` (lista de caminhos) e `excludes`. Não há
instalação em produção, então não há migração do formato antigo.

Validação (`Config::validate`), além das regras atuais de `recycle_dir`/`db_path`:

- pelo menos uma pasta; cada `path` em forma `X:\...` (raiz `X:\` permitida);
- `include` de cada pasta não vazio;
- máscara não vazia, sem NUL e **sem `\`** (máscara é de nome de arquivo);
- pastas não podem se aninhar (`D:\Dados` e `D:\Dados\Fiscal` juntas é erro), porque
  no driver as regras de pastas diferentes se somam;
- `theme` é um dos três valores (padrão `rust` quando ausente).

## 2. Máscaras enviadas ao driver

`config::driver_masks(&Config) -> (Vec<String>, Vec<String>)`, função pura. Para cada
pasta `F` (maiúsculas, sem `\` final) e cada máscara `m` (maiúsculas):

| `m` | Máscaras geradas |
|---|---|
| `*` ou `*.*` | `F\*` |
| começa com `*` (ex.: `*.BAK`) | `F\m` (o `*` inicial já cobre subpastas) |
| outra (ex.: `~$*`, `RELATORIO.XLSX`) | `F\m` e `F\*\m` |

Inclusões viram máscaras de inclusão, exclusões viram de exclusão. Sempre se somam
às exclusões: `RECYCLE_DIR\*` e `DB_PATH*`.

Limitação documentada na interface (texto de ajuda do diálogo de regras): curinga no
meio do nome (`relat*.xlsx`) também casa com nomes de subpasta, porque o `*` do driver
atravessa `\`.

## 3. Quem apagou

Novo módulo `audit.rs`, com trait `Audit` (padrão do repo: Win32 atrás de trait,
mock nos testes).

### 3.1 Configuração do Windows (no início do serviço e ao salvar a config)

1. Liga sucesso na subcategoria "Sistema de Arquivos"
   (`{0CCE921D-69AE-11D9-BED3-505054503030}`).
2. Auditoria global de arquivos (`auditpol /resourceSACL /set /type:File /success`)
   para o acesso DELETE, restrita a **Usuários do Domínio** (SID do domínio + `-513`).
   Em servidor fora de domínio, usa `BUILTIN\Users` (mais ruidoso; avisado na janela).
   Nenhum arquivo é tocado. A sintaxe exata de `/access` para DELETE é conferida com
   `auditpol /resourceSACL /view` como primeiro passo da implementação.
3. Se o log de Segurança tiver menos de 512 MB, aumenta para 512 MB (nunca diminui).
4. Grava em `C:\ProgramData\rnetrbin\audit-state.json` o que existia antes (política
   da subcategoria, entrada global), para o `uninstall` desfazer só o que o rnetrbin
   mudou. O tamanho do log não é revertido.

Se uma GPO sobrescrever a política, o serviço registra no log e a janela mostra
"Auditoria: desligada"; a captura de arquivos continua normal.

### 3.2 Coleta dos eventos

- Thread do serviço com `EvtSubscribe` no canal Security, consulta: evento 4663 com
  `AccessMask` contendo DELETE (0x10000).
- Para cada evento: `parse_4663(xml) -> Option<DeleteEvent { path, user, sid, utc }>`
  (função pura). Descarta o que não estiver dentro de uma pasta protegida.
- Grava em nova tabela `delete_events(path_upper TEXT, user TEXT, sid TEXT, utc INT)`
  com índice em `path_upper`; apaga registros com mais de 24 h a cada hora.
- Retoma pelo bookmark do `EvtSubscribe` salvo no banco, para não perder eventos com
  o serviço parado por pouco tempo.

### 3.3 Associação evento ↔ backup

- Ao finalizar um backup (`finalize`), procura em `delete_events` o mesmo caminho
  com `utc` em `deleted_utc ± 120 s`, o mais próximo. Achou: grava `user_name` e
  `user_sid` na linha de `deleted_files`.
- O evento pode chegar depois do backup: a cada 30 s o serviço repete a busca para
  linhas sem usuário apagadas nos últimos 10 min. Depois disso, fica "desconhecido".
- Função pura `match_event(backup, eventos) -> Option<&DeleteEvent>` com a regra acima.

## 4. Interface (iced, mesmo exe)

Mockup aprovado: `.superpowers/brainstorm/13276-1792010063/content/layout-undelete-v2.html`
(fora do git; `.superpowers/` está no `.gitignore`).

Título da janela: `rnetrbin - Lixeira de rede - <NOME DO SERVIDOR>`.

Abas da faixa:

- **Arquivo**: Sobre, Sair.
- **Início**:
  - campo "Buscar na lixeira": procura na pasta atual **e abaixo** (nome, subpasta
    ou usuário) e mostra o caminho relativo de cada arquivo achado;
  - grupo Ações: Restaurar, Restaurar em..., Excluir, Ver cópias;
  - Atualizar;
  - grupo Mostrar: [x] Todas as cópias.
- **Pastas protegidas**: tabela Pasta / Incluir / Excluir / Apagados; Adicionar pasta
  (seletor do Windows; regras iniciais Incluir `*.*`, Excluir `*.tmp; ~$*`),
  Editar regras, Remover. Salvar reinicia o serviço.
- **Configurações**: pasta da lixeira, arquivo do índice, limite (GB), retenção
  (dias), tema; Instalar / Iniciar / Parar serviço; estado da auditoria.

Corpo da aba Início:

- Esquerda, "Lixeira": árvore com raiz "Lixeira (N)", as pastas protegidas em negrito
  e, abaixo de cada uma, só as subpastas que têm arquivos apagados, com contagem.
  Expandir/recolher por clique no marcador; clicar no nome navega para a pasta.
  Montada por função pura `tree::build(paths) -> Tree` a partir dos caminhos
  originais não restaurados.
- Direita, "Arquivos recuperáveis", navegação como o Explorer:
  - acima da lista, o caminho clicável (`Lixeira › D:\Dados\Contabil › Clientes`) e
    o botão "Acima" (sobe um nível);
  - a lista mostra só o conteúdo **direto** da pasta atual: primeiro as subpastas que
    têm algo apagado dentro (nome, quantidade de apagados, tamanho somado, data da
    última exclusão), depois os arquivos daquele nível; na raiz "Lixeira", as
    subpastas são as pastas protegidas;
  - clicar numa subpasta entra nela; a árvore expande e marca o mesmo lugar;
  - colunas Nome, Cópias, Usuário, Tamanho, Apagado em; ordenação por clique no
    cabeçalho (subpastas sempre antes dos arquivos);
  - clique seleciona arquivo, Ctrl+clique alterna, Shift+clique seleciona intervalo;
    subpastas não são selecionáveis;
  - "Cópias" = quantas versões não restauradas do mesmo caminho existem. Com "Todas
    as cópias" desmarcado, mostra só a mais recente de cada caminho; "Ver cópias"
    lista todas as versões do arquivo selecionado.
- Barra de status: Serviço, Driver, Auditoria, uso da lixeira, quantidade selecionada.
- Lista limitada a 1.000 linhas por visão, com aviso para refinar a busca.

Primeiro uso (serviço não instalado): abre em Configurações com os campos mínimos
(pasta da lixeira e a primeira pasta protegida) e o botão Instalar.

Temas: Rust (padrão; paleta do tema "Rust" do mdBook: fundo `#e1e1db`, painel
`#ecebe5`, destaque `#b7410e`), Branco e Preto, com as cores do mockup. O tema
escolhido fica em `config.theme`.

Ícones: fonte **Material Icons** (regular) do Google, a mesma usada pelo
`iced-material-icon-browser`. O arquivo `MaterialIcons-Regular.ttf` (licença
Apache 2.0, do repositório `google/material-design-icons`) fica em `crates/svc/assets/`
com o texto da licença ao lado, é embutido com `include_bytes!` e carregado na
inicialização do iced; cada ícone é um `text` com o codepoint na fonte. Ícones usados
(nome Material → onde):

| Ícone | Onde |
|---|---|
| `restore`, `drive_file_move`, `delete_forever`, `history`, `refresh` | Início: Restaurar, Restaurar em..., Excluir, Ver cópias, Atualizar |
| `search` | campo de busca |
| `create_new_folder`, `rule`, `folder_off` | Pastas protegidas: Adicionar, Editar regras, Remover |
| `save`, `install_desktop`, `play_circle`, `stop_circle` | Configurações: Salvar, Instalar, Iniciar, Parar |
| `delete` | raiz "Lixeira" na árvore e no caminho |
| `folder_special` | pasta protegida |
| `folder` / `folder_open` | subpasta / subpasta atual |
| `description` | arquivo na lista |
| `arrow_upward` | botão Acima |
| `check_circle` / `error` | estado na barra de status (ok / problema) |
| `delete_sweep` | título da janela |

Os codepoints ficam em constantes num módulo `gui/icons.rs`, conferidos contra o
`MaterialIcons-Regular.codepoints` do mesmo repositório.

Restaurar e excluir mantêm as regras atuais: restaurar no local original nunca
sobrescreve; "Restaurar em..." recusa destino existente; excluir apaga arquivo e linha.

## 5. Organização do código

- `config.rs`: novo modelo, validação, `driver_masks`.
- `audit.rs` (novo): trait `Audit` + `WinAudit`; `parse_4663`; assinatura de eventos.
- `index.rs`: tabela `delete_events`; preencher usuário; contagem de cópias; listagem
  do conteúdo direto de uma pasta (subpastas com agregados + arquivos do nível),
  busca na pasta e abaixo, todas/últimas cópias.
- `service.rs`: inicia a assinatura e o preenchimento periódico de usuário.
- `gui/` (substitui `gui.rs`): `mod.rs` (estado e update), `theme.rs`, `icons.rs`,
  `tree.rs` (árvore pura), `home.rs` (faixa, árvore, caminho, lista), `folders.rs`,
  `settings.rs`.
- `crates/svc/assets/`: `MaterialIcons-Regular.ttf` + `LICENSE` (Apache 2.0).
- `install.rs`: `uninstall` desfaz a auditoria conforme `audit-state.json`.

## 6. Testes

Unitários (sem Windows real):

- `driver_masks`: `*.*`, `*.bak`, `~$*`, raiz `X:\`, maiúsculas, exclusões automáticas.
- Validação: pastas aninhadas, máscara com `\`, `include` vazio, tema inválido.
- `parse_4663` com XML real de 4663 (copiado do log deste PC) e com XML incompleto.
- `match_event`: dentro e fora da janela de 120 s, o mais próximo entre vários.
- `tree::build`: contagens agregadas, só subpastas com apagados, pastas protegidas na
  raiz.
- Índice: cópias por caminho; conteúdo direto de uma pasta (subpastas com quantidade,
  tamanho e última exclusão; arquivos só do nível); busca na pasta e abaixo;
  todas/últimas cópias.
- Ícones: cada constante de `gui/icons.rs` bate com o codepoint do arquivo
  `.codepoints` (teste lê o arquivo de `assets/`).

Win32 (`WinAudit`, SCM, driver) sem teste automático, como hoje. Manual no servidor:
apagar pelo SMB em pasta protegida e conferir arquivo na lixeira, coluna Usuário,
exclusões `.bak`/`.tmp`, e que uma pasta grande fica protegida na hora.

## Fora do escopo

- Driver próprio (anotado como melhoria futura).
- Coluna "Data de modificação".
- Acesso pela rede ou por outros usuários.
