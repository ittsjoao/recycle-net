# recycle-net

## O que é

Lixeira de rede para shares SMB em Windows: servico Rust user-mode (`recycle-net`) que instala e
conversa com o minifilter `netrbin.sys` pela porta `\CaptureFileMonitorPort`.

## Como rodar

- build: `cargo build --release` -> `target/release/recycle-net.exe` (embute o driver; exige
  `driver/netrbin.sys` ou `RECYCLE_NET_DRIVER`, copiado de `C:\Windows\System32\drivers`)
- testes: `cargo test` e `cargo clippy --all-targets` (sem driver/SCM); com driver (so VM): `cargo test -p recycle-net --test port_live -- --ignored`
- instalar: o setup instala driver+servico (pede a pasta da lixeira, roda `init-config`, `install`
  e `sc start` no PrepareToInstall); manual (admin): `init-config --recycle-dir <dir>`, `install
  [--driver <sys>] [--exe <exe>]`, `sc.exe start recycle-net`; remover com `uninstall [--keep-driver]`
- janela: exe sem argumentos (pede UAC); abas Inicio, Pastas protegidas (Incluir/Excluir) e Configuracoes
  (servico, iniciar com o Windows, tema). SCM: `--service`; console: `--console [--takeover]` (1o argumento)
- instalador: `installer/build.ps1` -> `target\installer\recycle-net-setup-<versao>.exe`

## Convencoes que nao se deduzem do codigo

- SCM/driver/sistema ficam atras de traits (`Scm`, `Driver`, `Host`) com mock nos testes; a
  implementacao Win32 nao tem teste automatico.
- Registro do driver (Altitude 370150, Group Filter, Start 0) vem do instalador v6.1.1.3; confira com `fltmc`.
- Se `SNetRBin` (original) existir, o servico assume o lugar dele e devolve no STOP; `uninstall` mantem o driver.
- Quem apagou vem da auditoria do Windows (SACL global so DELETE para Usuarios do Dominio ou
  `S-1-5-32-545` fora de dominio + eventos 4663 e 4659 (Delete On Close, apagar pelo SMB)), ligada pelo servico ao iniciar e desfeita no
  `uninstall` conforme `C:\ProgramData\recycle-net\audit-state.toml` (entrada ja existente,
  `sacl_existed`, fica intocada). Precisa de admin; sem auditoria o usuario fica "desconhecido".
- Pastas protegidas nao aninham (o driver junta as mascaras numa lista so); zero pastas e valido
  (nada vai para a lixeira); o driver ja instalado (SNetRBin) e reaproveitado, nunca sobrescrito.
- Ate v0.1.0 o nome era rnetrbin (servico `rnetrbin-svc`, `C:\ProgramData\rnetrbin`, marcador
  `.rnetrbin-root`): o setup remove esse servico (`uninstall --keep-driver` do exe antigo) e copia o
  config; o marcador antigo segue aceito. Release: push de tag `vX.Y.Z` publica so o setup.
- Exe e GUI subsystem (sem console piscando); a CLI escreve no console de quem chamou
  (`AttachConsole`), mas cmd/PowerShell nao esperam nem leem o exit code: o setup usa pipes.

## Armadilhas

- Porta do driver aceita UM cliente: com SNetRBin rodando, nada conecta. Desinstalar o original apaga o driver.
- Driver casa mascaras com `\E:\dir\arq` (barra na frente, MAIUSCULAS): use `config::driver_masks`.
  Nao ve apagar local no Server 2019+ (classe 64); contrato da copia em `docs/notes/port-protocol-confirmation.md`.

## Tarefa, branch e commit

- Branch: `feat|fix|chore|docs/<assunto-em-kebab-case>`; nao commitar em main/master (publicada)
- Commit: `tipo(escopo): assunto`, so assunto, sem corpo

## Wiki

Conhecimento deste repo vive em `C:\Users\joao.silva\Documents\wiki\pessoal-wiki`, dominio `projects`.
Consulte pelo `llms.txt` antes de perguntar; escreva la o que a sessao apurou, seguindo o AGENTS.md da wiki.
