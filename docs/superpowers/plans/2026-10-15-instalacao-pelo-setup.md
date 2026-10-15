# Instalação do serviço e do driver pelo setup

Pedido do usuário (2026-10-15), depois de testar o instalador: "não instalou, deu erro de instalar o
driver, mas essa instalação deve ocorrer assim que instalar a aplicação, não um botão de instalar".

## Decisões

- O setup (Inno) instala tudo: copia o exe, garante uma config e roda `rnetrbin-svc.exe install`
  (driver + serviço) e depois inicia o serviço. Não existe mais botão Instalar na janela.
- Config inicial: se `C:\ProgramData\rnetrbin\config.toml` não existir, o setup cria uma com
  zero pastas protegidas (válido desde 2026-10-14), `db_path` padrão, limite 10 GB e
  `recycle_dir` escolhido numa página do assistente "Pasta da lixeira" (padrão
  `{sd}\RecycleBinNet`, a pasta precisa ficar vazia ou ser nova). Se a config já existir
  (atualização), ela é mantida e a página não aparece.
- Erro na instalação do driver/serviço: o setup mostra a mensagem completa (saída do comando,
  redirecionada para um log em `{tmp}`) num texto legível em pt-BR com o detalhe técnico embaixo,
  e aborta; nada fica pela metade (o setup roda `uninstall` para desfazer o que o `install` fez
  antes de abortar, ignorando erros dele).
- A janela: remove o botão Instalar e a regra R3. Se o serviço não estiver instalado, a barra de
  status mostra "Serviço: não instalado" e Configurações mostra "Reinstale o rnetrbin pelo
  setup.". Primeiro uso (zero pastas): abre em Pastas protegidas.
- `install` da linha de comando continua existindo (VM/manual), mas passa a aceitar config com
  zero pastas sem reclamar (já aceita, pois `validate` aceita zero pastas) e a imprimir erros por
  etapa em pt-BR legível: "copiar o driver", "registrar o driver", "carregar o driver",
  "registrar o serviço", com a causa do Windows (código e texto). Para "carregar o driver" com
  `ERROR_INVALID_IMAGE_HASH` (577) ou `ERROR_DRIVER_BLOCKED` (1275), acrescentar a dica: "O
  Windows bloqueou o driver (assinatura de 2015). Desligue a Integridade de Memória em Segurança
  do Windows → Isolamento de núcleo, reinicie e instale de novo."
- Desinstalação continua como está (aborta se o `uninstall` falhar).
