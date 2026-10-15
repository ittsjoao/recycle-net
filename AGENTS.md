# rnetrbin

## O que e

Lixeira de rede para shares SMB em Windows: servico Rust user-mode (`rnetrbin-svc`) que instala e
conversa com o minifilter de terceiro `netrbin.sys` (produto licenciado) pela porta `\CaptureFileMonitorPort`.

## Como rodar

- build: `cargo build --release` -> `target/release/rnetrbin-svc.exe` (embute o driver; exige
  `driver/netrbin.sys` ou `RNETRBIN_DRIVER`, copiado de `C:\Windows\System32\drivers`)
- testes: `cargo test` (unitarios, sem driver/SCM) e `cargo clippy --all-targets`
- testes com driver: `cargo test -p rnetrbin-svc --test port_live -- --ignored` (so VM)
- instalar (admin): config em `C:\ProgramData\rnetrbin\config.toml` (ver `config.example.toml`),
  `rnetrbin-svc.exe install` (`--driver <sys>` sobrescreve o embutido), `sc.exe start rnetrbin-svc`;
  remover com `uninstall [--keep-driver]`
- janela: abrir o exe sem argumentos (pede UAC). Abas Inicio (arvore da lixeira e lista), Pastas
  protegidas (regras Incluir/Excluir) e Configuracoes (instalar/iniciar/parar, tema). SCM: `--service`
- console: `--console` (sem tocar SCM) ou `--console --takeover` (sempre 1o argumento)

## Convencoes que nao se deduzem do codigo

- SCM/driver/sistema ficam atras de traits (`Scm`, `Driver`, `Host`) com mock nos testes; a
  implementacao Win32 nao tem teste automatico.
- Registro do driver (Altitude 370150, Group Filter, Start 0) vem do instalador v6.1.1.3; confira com `fltmc`.
- Se `SNetRBin` (original) existir, o servico assume o lugar dele e devolve no STOP; `uninstall` mantem o driver.
- Quem apagou vem da auditoria do Windows (SACL global so DELETE para Usuarios do Dominio ou
  `S-1-5-32-545` fora de dominio + evento 4663), ligada pelo servico ao iniciar e desfeita no
  `uninstall` conforme `C:\ProgramData\rnetrbin\audit-state.toml` (entrada ja existente,
  `sacl_existed`, fica intocada). Precisa de admin; sem auditoria o usuario fica "desconhecido".
- Pastas protegidas nao aninham (o driver junta as mascaras numa lista so); zero pastas e valido
  (nada vai para a lixeira), mas "Instalar" exige ao menos uma.

## Armadilhas

- `netrbin.sys` nunca vai para o git (`*.sys` no .gitignore): binario de terceiro. O build falha
  sem ele (build.rs); o exe gerado nao pode ser publicado.
- Porta do driver aceita UM cliente: com SNetRBin rodando, nada conecta. Desinstalar o original apaga o driver.
- Assinatura do driver cross-signed 2015 (expirou 2016): pode cair com HVCI (Win 11 / Server 2022).
- Mascaras do driver casam com o caminho DOS completo em MAIUSCULAS: use
  `config::driver_masks`, nunca a mascara crua. Layout de paths da copia nao confirmado em runtime.

## Tarefa, branch e commit

- Branch: `feat|fix|chore|docs/<assunto-em-kebab-case>`; nao commitar em main/master (publicada)
- Commit: `tipo(escopo): assunto`, so assunto, sem corpo

## Wiki

Conhecimento deste repo vive em `C:\Users\joao.silva\Documents\wiki\pessoal-wiki`, dominio `projects`.
Consulte pelo `llms.txt` antes de perguntar; escreva la o que a sessao apurou, seguindo o AGENTS.md da wiki.
