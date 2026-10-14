# rnetrbin

## O que e

Lixeira de rede para shares SMB em Windows: servico Rust user-mode (`rnetrbin-svc`)
que instala e conversa com o minifilter de terceiro `netrbin.sys` (do produto
"Network Shared Recycle Bin", licenciado) pela porta `\CaptureFileMonitorPort`.

## Como rodar

- build: `cargo build --release` -> `target/release/rnetrbin-svc.exe`
- testes: `cargo test` (unitarios, sem driver/SCM) e `cargo clippy --all-targets`
- testes com driver: `cargo test -p rnetrbin-svc --test port_live -- --ignored` (so VM)
- instalar (admin): config em `C:\ProgramData\rnetrbin\config.toml` (ver
  `config.example.toml`), depois `rnetrbin-svc.exe install --driver <netrbin.sys>`
  e `sc.exe start rnetrbin-svc`; remover com `uninstall [--keep-driver]`
- console: `--console` (sem tocar SCM) ou `--console --takeover`

## Convencoes que nao se deduzem do codigo

- Acesso a SCM/driver/sistema fica atras de traits (`Scm`, `Driver`, `Host`) com
  mock nos testes; a implementacao Win32 nao tem teste automatico.
- Valores de registro do driver (Altitude 370150, Group Filter, Start 0) copiados do
  instalador original v6.1.1.3; nao mudar sem conferir com `fltmc`.
- Se `SNetRBin` (servico original) existir, o servico assume o lugar dele e devolve
  no STOP; `uninstall` nao remove o driver nesse caso.

## Armadilhas

- `netrbin.sys` nunca vai para o git (`*.sys` no .gitignore): binario de terceiro.
- Porta do driver aceita UM cliente: com SNetRBin rodando, nada conecta.
- Desinstalar o produto original apaga o driver junto.
- Assinatura do driver e cross-signed de 2015 (expirou 2016): pode ser bloqueada
  com HVCI (Windows 11 / Server 2022).
- Protocolo e layout de paths ainda nao confirmados em runtime; ver `docs/notes/`.

## Tarefa, branch e commit

- Branch: `feat|fix|chore|docs/<assunto-em-kebab-case>`
- Nao commitar direto em: main, master — main e a branch publicada
- Commit: `tipo(escopo): assunto`, so assunto, sem corpo

## Wiki

Conhecimento deste repo vive em `C:\Users\joao.silva\Documents\wiki\pessoal-wiki`, dominio `projects`.
Consulte pelo `llms.txt` antes de perguntar; escreva la o que a
sessao apurou, seguindo o AGENTS.md da wiki.
