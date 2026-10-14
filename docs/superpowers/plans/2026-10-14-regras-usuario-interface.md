# Regras por pasta, usuário que apagou e interface estilo Undelete: plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cada pasta protegida ganha regras Incluir/Excluir, a lista mostra quem apagou
(via auditoria global do Windows) e a janela vira o modelo do Undelete (faixa de abas,
árvore, lista com navegação por pasta, ícones Material, temas Rust/Branco/Preto).

**Architecture:** O config passa a ter `[[folders]]` com `include`/`exclude`, traduzidos
para máscaras do driver por uma função pura. Um módulo `audit` liga a auditoria global
(sem tocar arquivos), assina os eventos 4663 do log de Segurança, guarda os que caem nas
pastas protegidas numa tabela e uma thread associa evento ↔ backup. A GUI iced é
reescrita em `gui/` com módulos puros (`tree`, `listing`) testados e views finas.

**Tech Stack:** Rust 2021, iced 0.14 (wgpu + tiny-skia), rusqlite 0.31 (bundled),
crate `windows` 0.58, rfd 0.17, chrono 0.4, `auditpol.exe` e `wevtutil.exe` do Windows.

**Spec:** `docs/superpowers/specs/2026-10-14-regras-usuario-interface-design.md`

## Global Constraints

- Branch `feat/driver-install`. Commit: `tipo(escopo): assunto`, só assunto, **sem corpo e sem `Co-Authored-By`** (o hook do repositório recusa).
- Win32/SCM/driver/auditoria ficam atrás de funções pequenas ou traits; só a lógica pura tem teste automático (convenção do repo).
- Toda máscara enviada ao driver é MAIÚSCULA e ancorada no caminho completo (`FsRtlIsNameInExpression` com IgnoreCase; `*` atravessa `\`).
- Pastas protegidas não podem se aninhar; máscaras não podem conter `\`, NUL ou ser vazias; `include` de cada pasta não vazio.
- Exclusões automáticas sempre presentes: `RECYCLE_DIR\*` e `DB_PATH*`.
- Auditoria: subcategoria "Sistema de Arquivos" `{0CCE921D-69AE-11D9-BED3-505054503030}` com sucesso; SACL global só DELETE (`/access:0x10000`) para Usuários do Domínio (`<SID do domínio>-513`), ou `S-1-5-32-545` fora de domínio; log de Segurança no mínimo 512 MB (536870912 bytes), nunca diminui.
- Associação evento ↔ backup: mesmo caminho (maiúsculas), `|evento - deleted_utc| <= 120 s`, o mais próximo; linhas sem usuário são re-tentadas por 10 min (a thread roda a cada 5 s, o que cobre "ao finalizar" e "a cada 30 s" da spec).
- Lista limitada a 1.000 arquivos por visão, com aviso.
- Tema padrão `rust`; paletas exatamente como na Task 6.
- iced 0.14.2: as chamadas abaixo foram conferidas no código-fonte do crate; se alguma não compilar, ajuste à API do 0.14 mantendo o comportamento.
- Rodar sempre na raiz do repo: `cargo test` e `cargo clippy --all-targets` limpos ao fim de cada task.

## Review Focus

1. **Pasta removida da config com backups antigos** → os backups continuam visíveis e restauráveis (aparecem sob uma raiz da unidade, ex.: `D:`). Teste na Task 7 (`orphans_go_under_drive_root`).
2. **Caixa diferente no caminho** (`d:\dados\A.txt` vs `D:\Dados\a.TXT`) → árvore, lista e associação de usuário tratam como o mesmo caminho. Testes nas Tasks 2 (`events_match_case_insensitively`) e 7 (`tree_is_case_insensitive`).
3. **Evento de auditoria chega depois do backup, ou duas exclusões do mesmo caminho** → cada backup recebe o evento mais próximo dentro de 120 s; sem evento fica "desconhecido" sem erro. Testes na Task 2 (`match_event_picks_nearest_within_window`, `fill_pending_leaves_unknown_without_event`).
4. **Auditoria indisponível** (GPO sobrescreve, sem privilégio, servidor fora de domínio) → captura continua, `audit::spawn` só registra aviso, a janela mostra "Auditoria: desligada". Teste na Task 4 (`status_from_backup_csv`) e comportamento na Task 5 (spawn não retorna erro).
5. **Lixeira grande** (dezenas de milhares de arquivos numa pasta, ou busca ampla) → a lista mostra só 1.000 e diz quantos existem. Teste na Task 7 (`listing_truncates_and_reports_total`).

---

## File Structure

| Arquivo | Responsabilidade |
|---|---|
| `crates/svc/src/config.rs` (modificar) | Modelo `Config` com `FolderRule` e `ThemeName`, validação, `driver_masks` |
| `crates/svc/src/index.rs` (modificar) | Tabelas `delete_events` e `kv`, `DeleteEvent`, `Version`, consultas novas |
| `crates/svc/src/attribution.rs` (criar) | `match_event` e `fill_pending` (evento ↔ backup) |
| `crates/svc/src/audit/mod.rs` (criar) | `spawn`: configura auditoria e sobe as threads |
| `crates/svc/src/audit/event.rs` (criar) | `parse_4663` (XML → `DeleteEvent`), puro |
| `crates/svc/src/audit/setup.rs` (criar) | auditpol/wevtutil/LSA: `ensure`, `status`, `undo` + parsers puros |
| `crates/svc/src/audit/subscribe.rs` (criar) | `EvtSubscribe` no canal Security, bookmark, `under_any` |
| `crates/svc/src/service.rs` (modificar) | `run` chama `audit::spawn` e junta as threads |
| `crates/svc/src/main.rs` (modificar) | `uninstall` chama `audit::setup::undo` |
| `crates/svc/assets/` (criar) | `MaterialIcons-Regular.ttf`, `.codepoints`, `LICENSE` |
| `crates/svc/src/gui/mod.rs` (criar) | `App`, `Msg`, `update`, `run`, ações |
| `crates/svc/src/gui/icons.rs` (criar) | fonte embutida e `glyph(name)` a partir do `.codepoints` |
| `crates/svc/src/gui/theme.rs` (criar) | paletas e estilos de botão/contêiner |
| `crates/svc/src/gui/tree.rs` (criar) | árvore pura da lixeira com agregados |
| `crates/svc/src/gui/listing.rs` (criar) | conteúdo direto da pasta, busca, cópias, ordenação, formatação |
| `crates/svc/src/gui/home.rs` (criar) | aba Início: faixa, árvore, caminho, tabela |
| `crates/svc/src/gui/folders.rs` (criar) | aba Pastas protegidas + editor de regras |
| `crates/svc/src/gui/settings.rs` (criar) | aba Configurações e aba Arquivo |
| `crates/svc/src/gui.rs` → `gui_old.rs` (Task 6), apagado na Task 8 | janela antiga |

---

### Task 0: Commitar o estado atual como base

O trabalho anterior (driver embutido, janela inicial, pastas protegidas) está sem commit.
Esta task só o registra, em três commits, para o plano partir de uma árvore limpa.

**Files:** nenhum arquivo novo.

- [ ] **Step 1: Verificar que tudo passa**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: `test result: ok. 93 passed` na lib e nenhum warning do clippy.

- [ ] **Step 2: Commitar em três partes**

```bash
git add .gitignore crates/svc/build.rs crates/svc/src/install.rs .cargo/config.toml docs/notes/e2e-checklist.md
git commit -m "feat(install): embed netrbin.sys in the exe and install it by default"
git add crates/svc/Cargo.toml crates/svc/src/main.rs crates/svc/src/gui.rs crates/svc/src/lib.rs crates/svc/src/index.rs crates/svc/src/retention.rs
git commit -m "feat(gui): iced window to browse, restore and purge the bin"
git add crates/svc/src/config.rs crates/svc/src/service.rs config.example.toml AGENTS.md
git commit -m "feat(config): protected folders with uppercase driver masks"
```

- [ ] **Step 3: Conferir árvore limpa**

Run: `git status --short`
Expected: saída vazia (o `driver/netrbin.sys` é ignorado pelo `.gitignore`).

---

### Task 1: Config com regras por pasta e tema

**Files:**
- Modify: `crates/svc/src/config.rs`
- Modify: `crates/svc/src/gui.rs` (adaptação mínima; o arquivo é apagado na Task 8)
- Modify: `crates/svc/src/service.rs` (dois `cfg()` de teste)
- Modify: `config.example.toml`

**Interfaces:**
- Produces:
  - `pub struct FolderRule { pub path: PathBuf, pub include: Vec<String>, pub exclude: Vec<String> }` com `FolderRule::with_defaults(path: PathBuf) -> FolderRule`
  - `pub enum ThemeName { Rust, Branco, Preto }` (`Default` = `Rust`, serde lowercase)
  - `Config { recycle_dir, db_path, max_bin_bytes, max_age_days, theme: ThemeName, folders: Vec<FolderRule> }`
  - `pub fn driver_masks(c: &Config) -> (Vec<String>, Vec<String>)`
  - `pub fn nests(parent: &Path, child: &Path) -> bool`

- [ ] **Step 1: Escrever os testes novos**

Em `crates/svc/src/config.rs`, no `mod tests`, substitua `parses_toml`, `empty_folders_is_invalid`, `ok_cfg`, `driver_masks_are_uppercase_full_path_and_exclude_own_files`, `save_then_load_round_trips_and_invalid_is_not_written` e `masks_with_nul_or_empty_are_invalid` por:

```rust
    #[test]
    fn parses_toml_with_folder_tables_and_theme() {
        let t = r#"
recycle_dir = 'E:\Lixeira'
db_path = 'C:\ProgramData\rnetrbin\index.db'
max_bin_bytes = 10737418240
theme = "preto"

[[folders]]
path = 'D:\Dados\Contabil'
include = ["*.*"]
exclude = ["*.bak"]

[[folders]]
path = 'D:\Dados\Fiscal'
include = ["*.xml", "*.pdf"]
"#;
        let c: Config = toml::from_str(t).unwrap();
        assert_eq!(c.theme, ThemeName::Preto);
        assert_eq!(c.folders.len(), 2);
        assert_eq!(c.folders[0].exclude, ["*.bak"]);
        assert!(c.folders[1].exclude.is_empty());
    }
    #[test]
    fn theme_defaults_to_rust_and_unknown_theme_fails() {
        let base = "recycle_dir = 'E:\\L'\ndb_path = 'C:\\x\\i.db'\nmax_bin_bytes = 1\n";
        let c: Config = toml::from_str(base).unwrap();
        assert_eq!(c.theme, ThemeName::Rust);
        assert!(toml::from_str::<Config>(&format!("{base}theme = \"verde\"\n")).is_err());
    }
    fn rule(path: &str, include: &[&str], exclude: &[&str]) -> FolderRule {
        FolderRule {
            path: path.into(),
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn ok_cfg() -> Config {
        Config {
            recycle_dir: "D:\\x".into(), db_path: "D:\\db\\i.db".into(), max_bin_bytes: 1,
            max_age_days: None, theme: ThemeName::Rust,
            folders: vec![rule("D:\\Dados", &["*.*"], &[])],
        }
    }
    #[test]
    fn empty_folders_is_invalid() {
        let mut c = ok_cfg();
        c.folders = vec![];
        assert!(c.validate().is_err());
    }
    #[test]
    fn folder_rules_are_validated() {
        let mut c = ok_cfg(); c.folders = vec![rule("D:\\A", &[], &[])];
        assert!(c.validate().is_err(), "empty include");
        for bad in ["", "a\0b", "sub\\*.txt"] {
            let mut c = ok_cfg(); c.folders = vec![rule("D:\\A", &["*.*"], &[bad])];
            assert!(c.validate().is_err(), "accepted mask {bad:?}");
        }
    }
    #[test]
    fn nested_folders_are_rejected_in_any_order_and_case() {
        for (a, b) in [("D:\\Dados", "d:\\dados\\Fiscal"), ("D:\\Dados\\Fiscal", "D:\\Dados"), ("E:\\", "e:\\x"), ("D:\\A", "d:\\a")] {
            let mut c = ok_cfg();
            c.folders = vec![rule(a, &["*.*"], &[]), rule(b, &["*.*"], &[])];
            c.normalize().unwrap();
            assert!(c.validate().is_err(), "accepted {a} + {b}");
        }
        let mut c = ok_cfg();
        c.folders = vec![rule("D:\\Dados", &["*.*"], &[]), rule("D:\\DadosOld", &["*.*"], &[])];
        c.validate().unwrap();
    }
    #[test]
    fn driver_masks_follow_the_spec_table() {
        let mut c = Config {
            recycle_dir: r"E:\Lixeira".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            folders: vec![
                rule(r"d:/Dados/Contabil/", &["*.*"], &["*.bak", "~$*"]),
                rule(r"D:\Dados\Fiscal", &["*.xml", "Relatorio.pdf"], &[]),
                rule(r"F:\", &["*"], &[]),
            ],
            ..ok_cfg()
        };
        c.normalize().unwrap();
        c.validate().unwrap();
        let (inc, exc) = driver_masks(&c);
        assert_eq!(inc, [
            r"D:\DADOS\CONTABIL\*",
            r"D:\DADOS\FISCAL\*.XML",
            r"D:\DADOS\FISCAL\RELATORIO.PDF", r"D:\DADOS\FISCAL\*\RELATORIO.PDF",
            r"F:\*",
        ]);
        assert_eq!(exc, [
            r"D:\DADOS\CONTABIL\*.BAK",
            r"D:\DADOS\CONTABIL\~$*", r"D:\DADOS\CONTABIL\*\~$*",
            r"E:\LIXEIRA\*", r"C:\PROGRAMDATA\RNETRBIN\INDEX.DB*",
        ]);
    }
    #[test]
    fn save_then_load_round_trips_and_invalid_is_not_written() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("sub").join("config.toml");
        let mut c = Config {
            recycle_dir: "d:/bin/".into(), max_age_days: Some(30), theme: ThemeName::Branco,
            folders: vec![rule("d:/Dados", &["*.*"], &["*.bak"])],
            ..Config::default()
        };
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
        let mut bad = Config { max_bin_bytes: 0, ..Config::default() };
        let q = t.path().join("bad.toml");
        assert!(bad.save(&q).is_err());
        assert!(!q.exists());
    }
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib config::`
Expected: erro de compilação (`FolderRule`, `ThemeName`, campo `theme` não existem).

- [ ] **Step 3: Implementar o modelo**

Em `crates/svc/src/config.rs`, substitua a `struct Config`, o `impl Default for Config` e a `fn driver_masks` (linhas 7–54) por:

```rust
/// One protected folder: a deleted file under `path` (subfolders included) goes to
/// the bin when its name matches some `include` mask and no `exclude` mask.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FolderRule {
    pub path: PathBuf,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl FolderRule {
    /// Rules offered when a folder is added in the GUI.
    pub fn with_defaults(path: PathBuf) -> FolderRule {
        FolderRule { path, include: vec!["*.*".into()], exclude: vec!["*.tmp".into(), "~$*".into()] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    Rust,
    Branco,
    Preto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub recycle_dir: PathBuf,
    pub db_path: PathBuf,
    pub max_bin_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub theme: ThemeName,
    /// Last: TOML writes the `[[folders]]` tables after the plain keys.
    #[serde(default)]
    pub folders: Vec<FolderRule>,
}

impl Default for Config {
    /// Starting values offered by the GUI before the first install.
    fn default() -> Self {
        Config {
            recycle_dir: r"D:\RecycleBinNet".into(),
            db_path: r"C:\ProgramData\rnetrbin\index.db".into(),
            max_bin_bytes: 10 * 1024 * 1024 * 1024,
            max_age_days: None,
            theme: ThemeName::Rust,
            folders: vec![],
        }
    }
}

/// `child` is `parent` or lies under it (case-insensitive, `X:\` roots included).
pub fn nests(parent: &Path, child: &Path) -> bool {
    let p = parent.to_string_lossy().to_lowercase();
    let p = p.trim_end_matches('\\');
    let c = child.to_string_lossy().to_lowercase();
    let c = c.trim_end_matches('\\');
    c == p || c.starts_with(&format!("{p}\\"))
}

/// Driver masks for one rule mask, anchored on folder `f` (spec section 2).
fn anchored(f: &Path, m: &str) -> Vec<String> {
    let f = f.to_string_lossy().trim_end_matches('\\').to_uppercase();
    let m = m.to_uppercase();
    if m == "*" || m == "*.*" {
        vec![format!("{f}\\*")]
    } else if m.starts_with('*') {
        // The leading `*` already crosses `\`, so it covers subfolders.
        vec![format!("{f}\\{m}")]
    } else {
        vec![format!("{f}\\{m}"), format!("{f}\\*\\{m}")]
    }
}

/// The netrbin driver matches each mask with FsRtlIsNameInExpression(IgnoreCase)
/// against the deleted file's full DOS path (`D:\dir\file.ext`): masks must be
/// uppercase and anchored on the whole path. It keeps a file that matches any
/// include and no exclude. Returns (includes, excludes); the recycle dir and the
/// db are always excluded so the service never captures its own writes.
pub fn driver_masks(c: &Config) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for f in &c.folders {
        includes.extend(f.include.iter().flat_map(|m| anchored(&f.path, m)));
        excludes.extend(f.exclude.iter().flat_map(|m| anchored(&f.path, m)));
    }
    excludes.push(format!("{}\\*", c.recycle_dir.to_string_lossy()).to_uppercase());
    excludes.push(format!("{}*", c.db_path.to_string_lossy()).to_uppercase());
    (includes, excludes)
}
```

Em `normalize`, troque o laço das pastas por:

```rust
        // A drive root (`D:\`) protects the whole volume.
        for f in &mut self.folders {
            f.path = normalize_drive_path(&f.path, "folder", true)?;
        }
```

Em `validate`, substitua o bloco que começa em `if self.folders.is_empty()` e termina no fim do laço `for m in self.excludes.iter() { ... }` por:

```rust
        if self.folders.is_empty() {
            return Err(ConfigError::Invalid("at least one protected folder is required".into()));
        }
        for f in &self.folders {
            normalize_drive_path(&f.path, "folder", true)?;
            let shown = f.path.display();
            if f.include.is_empty() {
                return Err(ConfigError::Invalid(format!("folder {shown}: include must not be empty")));
            }
            for m in f.include.iter().chain(f.exclude.iter()) {
                if m.is_empty() || m.contains('\0') || m.contains('\\') {
                    return Err(ConfigError::Invalid(format!(
                        "folder {shown}: invalid mask {m:?} (empty, NUL or backslash; masks are file names)"
                    )));
                }
            }
        }
        // The driver adds every folder's masks into one list, so a nested pair
        // would leak one folder's exclusions into the other.
        for (i, a) in self.folders.iter().enumerate() {
            for b in &self.folders[i + 1..] {
                if nests(&a.path, &b.path) || nests(&b.path, &a.path) {
                    return Err(ConfigError::Invalid(format!(
                        "folders {} and {} overlap; protect only one of them",
                        a.path.display(),
                        b.path.display()
                    )));
                }
            }
        }
```

- [ ] **Step 4: Adaptar o resto do crate**

Em `crates/svc/src/service.rs`, nos dois helpers de teste que montam `Config` (procure `folders: vec![dir.to_path_buf()]`), troque a linha `folders: vec![dir.to_path_buf()],` e a linha seguinte `excludes: vec![],` por:

```rust
            folders: vec![crate::config::FolderRule { path: dir.to_path_buf(), include: vec!["*".into()], exclude: vec![] }],
            theme: Default::default(),
```

Em `crates/svc/src/gui.rs` (janela antiga, só para compilar até a Task 8):
- adicione `FolderRule, ThemeName` ao `use crate::config::{...}`;
- em `Form::from_config`, troque as linhas de `folders` e `excludes` por:

```rust
            folders: c.folders.iter().map(|f| f.path.display().to_string()).collect(),
            excludes: c.folders.first().map(|f| f.exclude.join("; ")).unwrap_or_else(|| "*.tmp; ~$*".into()),
```

- em `Form::to_config`, troque as linhas `folders:` e `excludes:` por:

```rust
            folders: self
                .folders
                .iter()
                .map(|p| FolderRule { path: PathBuf::from(p), include: vec!["*.*".into()], exclude: split_masks(&self.excludes) })
                .collect(),
            theme: ThemeName::Rust,
```

- substitua os testes `form_round_trips_config` e `form_parses_masks_decimal_comma_and_empty_age` por:

```rust
    #[test]
    fn form_round_trips_config() {
        let c = Config {
            folders: vec![FolderRule::with_defaults(r"D:\Dados".into()), FolderRule::with_defaults(r"E:\".into())],
            max_age_days: Some(30),
            ..Config::default()
        };
        assert_eq!(Form::from_config(&c).to_config().unwrap(), c);
    }

    #[test]
    fn form_parses_masks_decimal_comma_and_empty_age() {
        let f = Form {
            folders: vec![r"D:\Dados".into()],
            excludes: " *.docx ;; Relatório *.xlsx ".into(),
            max_gb: "1,5".into(),
            ..Form::from_config(&Config::default())
        };
        let c = f.to_config().unwrap();
        assert_eq!(c.folders[0].exclude, ["*.docx", "Relatório *.xlsx"]);
        assert_eq!(c.max_bin_bytes, (1.5 * GB) as u64);
        assert_eq!(c.max_age_days, None);
        assert!(Form { max_gb: "0".into(), ..f.clone() }.to_config().is_err());
        assert!(Form { max_age_days: "x".into(), ..f }.to_config().is_err());
    }
```

Substitua `config.example.toml` inteiro por:

```toml
# Copy to C:\ProgramData\rnetrbin\config.toml (path is fixed in the binary), or
# let the GUI write it. Paths must be drive-absolute (X:\...); UNC paths are rejected.

# Where the driver writes captured files. Must be empty or already contain the
# .rnetrbin-root marker; the service creates the marker on first run.
recycle_dir = 'E:\Lixeira'

# SQLite index of deleted files. Must NOT be inside recycle_dir.
db_path = 'C:\ProgramData\rnetrbin\index.db'

# Quota: oldest non-restored backups are evicted above this size (bytes).
max_bin_bytes = 10737418240

# Optional: evict backups older than this many days.
# max_age_days = 30

# Window theme: rust | branco | preto
theme = "rust"

# One table per protected folder (subfolders included; folders must not nest).
# A deleted file goes to the bin when its NAME matches some include mask and no
# exclude mask. The recycle dir and the db are always excluded.
[[folders]]
path = 'D:\Dados\Contabil'
include = ["*.*"]
exclude = ["*.bak", "*.tmp", "~$*"]

[[folders]]
path = 'D:\Dados\Fiscal'
include = ["*.xml", "*.pdf"]
exclude = []
```

- [ ] **Step 5: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning.

- [ ] **Step 6: Commit**

```bash
git add crates/svc/src/config.rs crates/svc/src/service.rs crates/svc/src/gui.rs config.example.toml
git commit -m "feat(config): per-folder include and exclude rules and window theme"
```

---

### Task 2: Índice: eventos de exclusão, versões e associação de usuário

**Files:**
- Modify: `crates/svc/src/index.rs`
- Create: `crates/svc/src/attribution.rs`
- Modify: `crates/svc/src/lib.rs`

**Interfaces:**
- Produces (index.rs):
  - `pub struct DeleteEvent { pub path: String, pub user: String, pub sid: String, pub utc: i64 }`
  - `pub struct Version { pub id: i64, pub original_path: String, pub size: i64, pub deleted_utc: i64, pub user_name: Option<String>, pub copies: i64 }`
  - `Index::add_delete_event(&self, e: &DeleteEvent) -> rusqlite::Result<()>`
  - `Index::events_for(&self, path: &str, from: i64, to: i64) -> rusqlite::Result<Vec<DeleteEvent>>`
  - `Index::prune_delete_events(&self, before: i64) -> rusqlite::Result<usize>`
  - `Index::pending_attribution(&self, since: i64) -> rusqlite::Result<Vec<(i64, String, i64)>>`
  - `Index::set_user(&self, id: i64, user: &str, sid: &str) -> rusqlite::Result<()>`
  - `Index::get_kv(&self, k: &str) -> rusqlite::Result<Option<String>>`, `Index::set_kv(&self, k: &str, v: &str) -> rusqlite::Result<()>`
  - `Index::all_versions(&self) -> rusqlite::Result<Vec<Version>>`
- Produces (attribution.rs): `WINDOW_SECS = 120`, `PENDING_SECS = 600`, `match_event(deleted_utc: i64, events: &[DeleteEvent]) -> Option<&DeleteEvent>`, `fill_pending(idx: &Index, now: i64) -> rusqlite::Result<usize>`

- [ ] **Step 1: Escrever os testes do índice**

No `mod tests` de `crates/svc/src/index.rs`, acrescente:

```rust
    fn ev(path: &str, user: &str, utc: i64) -> DeleteEvent {
        DeleteEvent { path: path.into(), user: user.into(), sid: "S-1-5-21-1".into(), utc }
    }
    #[test]
    fn events_match_case_insensitively_and_by_time_range() {
        let idx = Index::open_memory().unwrap();
        idx.add_delete_event(&ev("d:\\dados\\A.txt", "AUSTER\\ana", 100)).unwrap();
        idx.add_delete_event(&ev("D:\\Dados\\a.TXT", "AUSTER\\bia", 500)).unwrap();
        let got = idx.events_for("D:\\DADOS\\A.TXT", 0, 200).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].user, "AUSTER\\ana");
        assert_eq!(idx.prune_delete_events(200).unwrap(), 1);
        assert_eq!(idx.events_for("D:\\Dados\\a.txt", 0, 1000).unwrap().len(), 1);
    }
    #[test]
    fn pending_attribution_and_set_user() {
        let idx = Index::open_memory().unwrap();
        let old = idx.insert(&sample(10, 1)).unwrap();
        let new = idx.insert(&sample(1000, 1)).unwrap();
        assert_eq!(idx.pending_attribution(500).unwrap().iter().map(|r| r.0).collect::<Vec<_>>(), [new]);
        idx.set_user(new, "AUSTER\\ana", "S-1").unwrap();
        assert!(idx.pending_attribution(0).unwrap().iter().all(|r| r.0 == old));
    }
    #[test]
    fn kv_round_trips() {
        let idx = Index::open_memory().unwrap();
        assert_eq!(idx.get_kv("a").unwrap(), None);
        idx.set_kv("a", "1").unwrap();
        idx.set_kv("a", "2").unwrap();
        assert_eq!(idx.get_kv("a").unwrap().as_deref(), Some("2"));
    }
    #[test]
    fn all_versions_counts_copies_per_path_and_skips_restored() {
        let idx = Index::open_memory().unwrap();
        let mut a1 = sample(1, 10); a1.original_path = "D:\\x\\a.txt".into();
        let mut a2 = sample(2, 20); a2.original_path = "d:\\X\\A.txt".into();
        let b = sample(3, 30);
        let r = sample(4, 40);
        idx.insert(&a1).unwrap();
        idx.insert(&a2).unwrap();
        idx.insert(&b).unwrap();
        let rid = idx.insert(&r).unwrap();
        idx.mark_restored(rid).unwrap();
        let mut v = idx.all_versions().unwrap();
        v.sort_by_key(|x| x.deleted_utc);
        assert_eq!(v.iter().map(|x| x.copies).collect::<Vec<_>>(), [2, 2, 1]);
        assert_eq!(v[0].user_name, None);
    }
```

Crie `crates/svc/src/attribution.rs` só com o módulo de testes:

```rust
//! Associate Security-log delete events (4663) with captured backups.
use crate::index::{DeleteEvent, Index};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::DeletedFile;

    fn ev(user: &str, utc: i64) -> DeleteEvent {
        DeleteEvent { path: "D:\\A\\x.txt".into(), user: user.into(), sid: "S".into(), utc }
    }

    #[test]
    fn match_event_picks_nearest_within_window() {
        let evs = [ev("far", 1000 - 121), ev("near", 1000 + 5), ev("mid", 1000 - 60)];
        assert_eq!(match_event(1000, &evs).unwrap().user, "near");
        assert!(match_event(1000, &[ev("late", 1000 + 121)]).is_none());
        assert!(match_event(1000, &[]).is_none());
    }

    fn add(idx: &Index, path: &str, utc: i64) -> i64 {
        idx.insert(&DeletedFile {
            original_path: path.into(), backup_path: format!("E:\\L\\{utc}"), volume: "D:".into(),
            size: 1, deleted_utc: utc, user_sid: None, user_name: None,
        }).unwrap()
    }

    #[test]
    fn fill_pending_sets_user_from_event_case_insensitively() {
        let idx = Index::open_memory().unwrap();
        let id = add(&idx, "D:\\A\\x.txt", 1000);
        idx.add_delete_event(&DeleteEvent { path: "d:\\a\\X.TXT".into(), user: "AUSTER\\ana".into(), sid: "S-1".into(), utc: 1003 }).unwrap();
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 1);
        let v = idx.all_versions().unwrap();
        assert_eq!(v.iter().find(|x| x.id == id).unwrap().user_name.as_deref(), Some("AUSTER\\ana"));
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 0, "already attributed");
    }

    #[test]
    fn fill_pending_leaves_unknown_without_event_and_stops_after_pending_window() {
        let idx = Index::open_memory().unwrap();
        add(&idx, "D:\\A\\x.txt", 1000);
        assert_eq!(fill_pending(&idx, 1010).unwrap(), 0);
        // An event arriving after PENDING_SECS is no longer looked up.
        idx.add_delete_event(&DeleteEvent { path: "D:\\A\\x.txt".into(), user: "u".into(), sid: "s".into(), utc: 1000 }).unwrap();
        assert_eq!(fill_pending(&idx, 1000 + PENDING_SECS + 1).unwrap(), 0);
        assert_eq!(fill_pending(&idx, 1000 + PENDING_SECS).unwrap(), 1);
    }
}
```

Em `crates/svc/src/lib.rs`, acrescente `pub mod attribution;`.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib index:: attribution::`
Expected: erro de compilação (`DeleteEvent`, `add_delete_event`, `match_event` não existem).

- [ ] **Step 3: Implementar no índice**

Em `crates/svc/src/index.rs`:

1. Troque `use rusqlite::{Connection, params};` por `use rusqlite::{params, Connection, OptionalExtension};`.
2. Depois da `struct Entry`, acrescente:

```rust
/// A 4663 Security event with DELETE access (who removed `path`, and when).
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteEvent {
    pub path: String,
    pub user: String,
    pub sid: String,
    pub utc: i64,
}

/// A non-restored backup with how many backups share its original path.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub id: i64,
    pub original_path: String,
    pub size: i64,
    pub deleted_utc: i64,
    pub user_name: Option<String>,
    pub copies: i64,
}
```

3. No fim da constante `SCHEMA`, antes do `"#;`, acrescente:

```sql
CREATE TABLE IF NOT EXISTS delete_events(
  path_upper TEXT NOT NULL, user TEXT NOT NULL, sid TEXT NOT NULL, utc INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS ix_de_path ON delete_events(path_upper, utc);
CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);
```

4. Antes de `pub fn count_by_backup_path`, acrescente:

```rust
    /// Paths are stored uppercased (Rust `to_uppercase`) so lookups ignore case.
    pub fn add_delete_event(&self, e: &DeleteEvent) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO delete_events(path_upper,user,sid,utc) VALUES(?1,?2,?3,?4)",
            params![e.path.to_uppercase(), e.user, e.sid, e.utc],
        )?;
        Ok(())
    }
    pub fn events_for(&self, path: &str, from: i64, to: i64) -> rusqlite::Result<Vec<DeleteEvent>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT path_upper,user,sid,utc FROM delete_events WHERE path_upper=?1 AND utc BETWEEN ?2 AND ?3",
        )?;
        let rows = st.query_map(params![path.to_uppercase(), from, to], |r| {
            Ok(DeleteEvent { path: r.get(0)?, user: r.get(1)?, sid: r.get(2)?, utc: r.get(3)? })
        })?;
        rows.collect()
    }
    pub fn prune_delete_events(&self, before: i64) -> rusqlite::Result<usize> {
        self.c().execute("DELETE FROM delete_events WHERE utc < ?1", [before])
    }
    /// (id, original_path, deleted_utc) of non-restored rows still without a user.
    pub fn pending_attribution(&self, since: i64) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,original_path,deleted_utc FROM deleted_files
             WHERE user_name IS NULL AND is_restored=0 AND deleted_utc >= ?1",
        )?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    pub fn set_user(&self, id: i64, user: &str, sid: &str) -> rusqlite::Result<()> {
        self.c().execute("UPDATE deleted_files SET user_name=?2, user_sid=?3 WHERE id=?1", params![id, user, sid])?;
        Ok(())
    }
    pub fn get_kv(&self, k: &str) -> rusqlite::Result<Option<String>> {
        self.c().query_row("SELECT v FROM kv WHERE k=?1", [k], |r| r.get(0)).optional()
    }
    pub fn set_kv(&self, k: &str, v: &str) -> rusqlite::Result<()> {
        self.c().execute("INSERT OR REPLACE INTO kv(k,v) VALUES(?1,?2)", [k, v])?;
        Ok(())
    }
    /// Every non-restored backup, with its copy count (case-insensitive path).
    pub fn all_versions(&self) -> rusqlite::Result<Vec<Version>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT id,original_path,size,deleted_utc,user_name,
                    COUNT(*) OVER (PARTITION BY lower(original_path))
             FROM deleted_files WHERE is_restored=0",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Version {
                id: r.get(0)?, original_path: r.get(1)?, size: r.get(2)?, deleted_utc: r.get(3)?,
                user_name: r.get(4)?, copies: r.get(5)?,
            })
        })?;
        rows.collect()
    }
```

- [ ] **Step 4: Implementar a associação**

Em `crates/svc/src/attribution.rs`, antes do `#[cfg(test)]`, acrescente:

```rust
/// Max distance between a backup's deleted_utc and its 4663 event.
pub const WINDOW_SECS: i64 = 120;
/// Rows without a user are retried for this long, then stay "desconhecido".
pub const PENDING_SECS: i64 = 600;

/// The event closest in time to `deleted_utc`, within WINDOW_SECS.
pub fn match_event(deleted_utc: i64, events: &[DeleteEvent]) -> Option<&DeleteEvent> {
    events
        .iter()
        .filter(|e| (e.utc - deleted_utc).abs() <= WINDOW_SECS)
        .min_by_key(|e| (e.utc - deleted_utc).abs())
}

/// Give recent rows without a user the user of their matching event.
/// Returns how many rows were filled.
pub fn fill_pending(idx: &Index, now: i64) -> rusqlite::Result<usize> {
    let mut filled = 0;
    for (id, path, utc) in idx.pending_attribution(now - PENDING_SECS)? {
        let events = idx.events_for(&path, utc - WINDOW_SECS, utc + WINDOW_SECS)?;
        if let Some(e) = match_event(utc, &events) {
            idx.set_user(id, &e.user, &e.sid)?;
            filled += 1;
        }
    }
    Ok(filled)
}
```

- [ ] **Step 5: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning.

- [ ] **Step 6: Commit**

```bash
git add crates/svc/src/index.rs crates/svc/src/attribution.rs crates/svc/src/lib.rs
git commit -m "feat(index): delete events, copy counts and user attribution"
```

---

### Task 3: Ler o evento 4663

**Files:**
- Create: `crates/svc/src/audit/mod.rs`
- Create: `crates/svc/src/audit/event.rs`
- Modify: `crates/svc/src/lib.rs`

**Interfaces:**
- Consumes: `crate::index::DeleteEvent` (Task 2)
- Produces: `pub fn parse_4663(xml: &str) -> Option<DeleteEvent>` em `crate::audit::event`

- [ ] **Step 1: Escrever os testes**

Crie `crates/svc/src/audit/mod.rs`:

```rust
//! Who deleted a file: Windows object-access auditing (global SACL, DELETE only)
//! plus a subscription to Security event 4663.
pub mod event;
```

Em `crates/svc/src/lib.rs`, acrescente `pub mod audit;`.

Crie `crates/svc/src/audit/event.rs`:

```rust
use crate::index::DeleteEvent;

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of EvtRender(EvtRenderEventXml) output for 4663 (single quotes).
    fn sample(object_type: &str, mask: &str, name: &str) -> String {
        format!(
            "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
             <Provider Name='Microsoft-Windows-Security-Auditing' Guid='{{54849625-5478-4994-a5ba-3e3b0328c30d}}'/>\
             <EventID>4663</EventID><Version>1</Version><Level>0</Level><Task>12800</Task>\
             <TimeCreated SystemTime='2026-10-14T19:44:07.1234567Z'/><EventRecordID>29778</EventRecordID>\
             <Channel>Security</Channel><Computer>SRV-ARQUIVOS.auster.local</Computer></System>\
             <EventData><Data Name='SubjectUserSid'>S-1-5-21-1-2-3-1104</Data>\
             <Data Name='SubjectUserName'>joao.silva</Data><Data Name='SubjectDomainName'>AUSTER2</Data>\
             <Data Name='SubjectLogonId'>0x4367b</Data><Data Name='ObjectServer'>Security</Data>\
             <Data Name='ObjectType'>{object_type}</Data><Data Name='ObjectName'>{name}</Data>\
             <Data Name='HandleId'>0x1a4</Data><Data Name='AccessList'>%%1537</Data>\
             <Data Name='AccessMask'>{mask}</Data><Data Name='ProcessId'>0x4</Data>\
             <Data Name='ProcessName'></Data><Data Name='ResourceAttributes'>S:AI</Data></EventData></Event>"
        )
    }

    #[test]
    fn parses_user_path_and_time_and_unescapes() {
        let e = parse_4663(&sample("File", "0x10000", "D:\\Dados\\R&amp;D\\a.txt")).unwrap();
        assert_eq!(e.path, "D:\\Dados\\R&D\\a.txt");
        assert_eq!(e.user, "AUSTER2\\joao.silva");
        assert_eq!(e.sid, "S-1-5-21-1-2-3-1104");
        assert_eq!(e.utc, 1792007047);
    }

    #[test]
    fn accepts_double_quotes_and_combined_masks() {
        let xml = sample("File", "0x10080", "D:\\a.txt").replace('\'', "\"");
        assert!(parse_4663(&xml).is_some());
    }

    #[test]
    fn rejects_other_objects_masks_and_events() {
        assert!(parse_4663(&sample("Key", "0x10000", "\\REGISTRY\\x")).is_none());
        assert!(parse_4663(&sample("File", "0x2", "D:\\a.txt")).is_none());
        assert!(parse_4663(&sample("File", "0x10000", "D:\\a.txt").replace("4663", "4660")).is_none());
        assert!(parse_4663("<Event/>").is_none());
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib audit::event`
Expected: erro de compilação (`parse_4663` não existe).

- [ ] **Step 3: Implementar**

Em `crates/svc/src/audit/event.rs`, antes do `#[cfg(test)]`:

```rust
/// DELETE access right.
const DELETE: u32 = 0x10000;

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Text of `<Data Name='name'>...</Data>` (either quote style).
fn data(xml: &str, name: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let open = format!("<Data Name={q}{name}{q}>");
        if let Some(i) = xml.find(&open) {
            let rest = &xml[i + open.len()..];
            return Some(unescape(&rest[..rest.find("</Data>")?]));
        }
    }
    None
}

/// Value of the `SystemTime` attribute of `<TimeCreated .../>`.
fn time_created(xml: &str) -> Option<String> {
    let rest = &xml[xml.find("<TimeCreated")?..];
    let rest = &rest[rest.find("SystemTime=")? + "SystemTime=".len()..];
    let q = rest.chars().next()?;
    let rest = &rest[1..];
    Some(rest[..rest.find(q)?].to_string())
}

/// A file deletion from an event 4663 rendered as XML; None for any other event.
pub fn parse_4663(xml: &str) -> Option<DeleteEvent> {
    if !xml.contains("<EventID>4663</EventID>") || data(xml, "ObjectType")? != "File" {
        return None;
    }
    let mask = data(xml, "AccessMask")?;
    let mask = u32::from_str_radix(mask.trim().trim_start_matches("0x"), 16).ok()?;
    if mask & DELETE == 0 {
        return None;
    }
    let utc = chrono::DateTime::parse_from_rfc3339(&time_created(xml)?).ok()?.timestamp();
    Some(DeleteEvent {
        path: data(xml, "ObjectName")?,
        user: format!("{}\\{}", data(xml, "SubjectDomainName")?, data(xml, "SubjectUserName")?),
        sid: data(xml, "SubjectUserSid").unwrap_or_default(),
        utc,
    })
}
```

- [ ] **Step 4: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning.

- [ ] **Step 5: Commit**

```bash
git add crates/svc/src/audit crates/svc/src/lib.rs
git commit -m "feat(audit): parse security event 4663 into a delete event"
```

---

### Task 4: Configurar a auditoria do Windows

**Files:**
- Create: `crates/svc/src/audit/setup.rs`
- Modify: `crates/svc/src/audit/mod.rs`
- Modify: `crates/svc/Cargo.toml` (features do crate `windows`)
- Modify: `crates/svc/src/main.rs` (`uninstall` desfaz a auditoria)

**Interfaces:**
- Produces (`crate::audit::setup`):
  - `pub const FILE_SYSTEM_SUBCATEGORY: &str`, `pub const MIN_SECURITY_LOG_BYTES: u64`, `pub const STATE_PATH: &str`
  - `pub struct AuditState { pub policy_before: u8, pub sacl_sid: String }`
  - `#[derive(Default, Clone, Copy)] pub struct AuditStatus { pub policy_success: bool, pub configured: bool }`
  - puros: `parse_backup_setting(csv: &str, guid: &str) -> Option<u8>`, `parse_max_size(out: &str) -> Option<u64>`, `audit_users_sid(domain_sid: Option<&str>) -> String`, `policy_args(setting: u8) -> Vec<String>`, `sacl_args(sid: &str) -> Vec<String>`
  - Windows: `ensure() -> Result<(), String>`, `status() -> AuditStatus`, `undo() -> Result<(), String>`

- [ ] **Step 1: Escrever os testes dos parsers**

Crie `crates/svc/src/audit/setup.rs`:

```rust
//! Turn on object-access auditing for deletes without touching any file:
//! "File System" success auditing + a global (resource) SACL for DELETE,
//! restricted to Domain Users. Uses auditpol.exe / wevtutil.exe (they enable the
//! needed privileges themselves) and LSA only to find the domain SID.
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    const BACKUP: &str = "Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value\r\n\
SRV,System,Sistema de Arquivos,{0CCE921D-69AE-11D9-BED3-505054503030},Sem Auditoria,,0\r\n\
SRV,System,Logon,{0CCE9215-69AE-11D9-BED3-505054503030},Êxito,,1\r\n";

    #[test]
    fn status_from_backup_csv() {
        assert_eq!(parse_backup_setting(BACKUP, FILE_SYSTEM_SUBCATEGORY), Some(0));
        assert_eq!(parse_backup_setting(&BACKUP.replace(",,0", ",,3"), FILE_SYSTEM_SUBCATEGORY), Some(3));
        assert_eq!(parse_backup_setting("Machine Name\r\n", FILE_SYSTEM_SUBCATEGORY), None);
    }

    #[test]
    fn parses_wevtutil_max_size() {
        let out = "name: Security\r\nenabled: true\r\nlogging:\r\n  logFileName: %SystemRoot%\\x.evtx\r\n  retention: false\r\n  autoBackup: false\r\n  maxSize: 20971520\r\n";
        assert_eq!(parse_max_size(out), Some(20971520));
        assert_eq!(parse_max_size("name: Security\r\n"), None);
    }

    #[test]
    fn audit_users_are_domain_users_or_builtin_users() {
        assert_eq!(audit_users_sid(Some("S-1-5-21-1-2-3")), "S-1-5-21-1-2-3-513");
        assert_eq!(audit_users_sid(None), "S-1-5-32-545");
    }

    #[test]
    fn command_arguments() {
        assert_eq!(
            policy_args(1),
            ["/set", "/subcategory:{0CCE921D-69AE-11D9-BED3-505054503030}", "/success:enable", "/failure:disable"]
        );
        assert_eq!(policy_args(2)[2..], ["/success:disable", "/failure:enable"]);
        assert_eq!(
            sacl_args("S-1-5-21-1-513"),
            ["/resourceSACL", "/set", "/type:File", "/success", "/user:{S-1-5-21-1-513}", "/access:0x10000"]
        );
    }
}
```

Em `crates/svc/src/audit/mod.rs`, acrescente `pub mod setup;`.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib audit::setup`
Expected: erro de compilação.

- [ ] **Step 3: Implementar as partes puras**

Em `setup.rs`, antes do `#[cfg(test)]`:

```rust
/// "Sistema de Arquivos" / "File System" subcategory (language independent).
pub const FILE_SYSTEM_SUBCATEGORY: &str = "{0CCE921D-69AE-11D9-BED3-505054503030}";
/// Enough for ~290k audited operations at ~1.8 KB each (measured).
pub const MIN_SECURITY_LOG_BYTES: u64 = 512 * 1024 * 1024;
/// What existed before rnetrbin changed the audit policy (read by uninstall).
pub const STATE_PATH: &str = r"C:\ProgramData\rnetrbin\audit-state.toml";
/// DELETE access right, as auditpol's hex access mask.
const DELETE_MASK: &str = "0x10000";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditState {
    /// auditpol "Setting Value" before we changed it: 0 none, 1 success, 2 failure, 3 both.
    pub policy_before: u8,
    /// Account the global SACL entry was created for.
    pub sacl_sid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditStatus {
    /// "File System" success auditing is on right now (a GPO may turn it off).
    pub policy_success: bool,
    /// rnetrbin configured auditing on this machine (state file exists).
    pub configured: bool,
}

/// "Setting Value" (last column) of the `auditpol /backup` CSV line for `guid`.
pub fn parse_backup_setting(csv: &str, guid: &str) -> Option<u8> {
    let guid = guid.to_uppercase();
    csv.lines()
        .find(|l| l.to_uppercase().contains(&guid))
        .and_then(|l| l.trim().rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
}

/// `maxSize:` from `wevtutil gl Security` (keys are not localized).
pub fn parse_max_size(out: &str) -> Option<u64> {
    out.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("maxSize:"))
        .and_then(|v| v.trim().parse().ok())
}

/// Domain Users of the machine's domain, or BUILTIN\Users outside a domain.
pub fn audit_users_sid(domain_sid: Option<&str>) -> String {
    match domain_sid {
        Some(d) => format!("{d}-513"),
        None => "S-1-5-32-545".into(),
    }
}

pub fn policy_args(setting: u8) -> Vec<String> {
    let flag = |on: bool| if on { "enable" } else { "disable" };
    vec![
        "/set".into(),
        format!("/subcategory:{FILE_SYSTEM_SUBCATEGORY}"),
        format!("/success:{}", flag(setting & 1 != 0)),
        format!("/failure:{}", flag(setting & 2 != 0)),
    ]
}

pub fn sacl_args(sid: &str) -> Vec<String> {
    ["/resourceSACL", "/set", "/type:File", "/success"]
        .iter()
        .map(|s| s.to_string())
        .chain([format!("/user:{{{sid}}}"), format!("/access:{DELETE_MASK}")])
        .collect()
}
```

- [ ] **Step 4: Rodar os testes puros**

Run: `cargo test -p rnetrbin-svc --lib audit::setup`
Expected: 4 passed.

- [ ] **Step 5: Implementar as chamadas ao Windows**

Em `crates/svc/Cargo.toml`, na lista `features` do `[dependencies.windows]`, acrescente:

```toml
  "Win32_Security_Authentication_Identity",
  "Win32_Security_Authorization",
  "Win32_System_EventLog",
```

Em `setup.rs`, depois de `sacl_args`:

```rust
fn run(cmd: &str, args: &[String]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        Err(format!("{cmd} {}: {} {}", args.join(" "), text(&out.stdout), text(&out.stderr)))
    }
}

/// Current "Setting Value" of the File System subcategory (0 when not listed).
fn current_setting() -> Result<u8, String> {
    let file = std::env::temp_dir().join(format!("rnetrbin-auditpol-{}.csv", std::process::id()));
    run("auditpol", &["/backup".into(), format!("/file:{}", file.display())])?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    let bytes = bytes?;
    // auditpol may write UTF-16LE (with BOM) or ANSI depending on the Windows build.
    let csv = if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(parse_backup_setting(&csv, FILE_SYSTEM_SUBCATEGORY).unwrap_or(0))
}

/// SID of the machine's primary domain, None in a workgroup.
pub fn primary_domain_sid() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authentication::Identity::{
        LsaClose, LsaFreeMemory, LsaOpenPolicy, LsaQueryInformationPolicy, PolicyPrimaryDomainInformation,
        LSA_HANDLE, LSA_OBJECT_ATTRIBUTES, POLICY_PRIMARY_DOMAIN_INFO, POLICY_VIEW_LOCAL_INFORMATION,
    };
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    unsafe {
        let attrs = LSA_OBJECT_ATTRIBUTES::default();
        let mut policy = LSA_HANDLE::default();
        if LsaOpenPolicy(None, &attrs, POLICY_VIEW_LOCAL_INFORMATION as u32, &mut policy).is_err() {
            return None;
        }
        let mut buf: *mut core::ffi::c_void = std::ptr::null_mut();
        let st = LsaQueryInformationPolicy(policy, PolicyPrimaryDomainInformation, &mut buf);
        let _ = LsaClose(policy);
        if st.is_err() || buf.is_null() {
            return None;
        }
        let info = &*(buf as *const POLICY_PRIMARY_DOMAIN_INFO);
        let mut out = None;
        if !info.Sid.is_invalid() {
            let mut s = PWSTR::null();
            if ConvertSidToStringSidW(info.Sid, &mut s).is_ok() {
                out = s.to_string().ok();
                let _ = LocalFree(HLOCAL(s.0 as _));
            }
        }
        let _ = LsaFreeMemory(Some(buf));
        out
    }
}

/// Idempotent. Records the previous policy once, then turns on success
/// auditing, sets the global DELETE SACL and grows the Security log.
pub fn ensure() -> Result<(), String> {
    let before = current_setting()?;
    let sid = audit_users_sid(primary_domain_sid().as_deref());
    if !Path::new(STATE_PATH).exists() {
        let state = AuditState { policy_before: before, sacl_sid: sid.clone() };
        if let Some(parent) = Path::new(STATE_PATH).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(STATE_PATH, toml::to_string(&state).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    if before & 1 == 0 {
        run("auditpol", &policy_args(before | 1))?;
    }
    run("auditpol", &sacl_args(&sid))?;
    let size = parse_max_size(&run("wevtutil", &["gl".into(), "Security".into()])?).unwrap_or(0);
    if size < MIN_SECURITY_LOG_BYTES {
        run("wevtutil", &["sl".into(), "Security".into(), format!("/ms:{MIN_SECURITY_LOG_BYTES}")])?;
    }
    Ok(())
}

pub fn status() -> AuditStatus {
    AuditStatus {
        policy_success: current_setting().map(|s| s & 1 != 0).unwrap_or(false),
        configured: Path::new(STATE_PATH).exists(),
    }
}

/// Undo only what `ensure` changed; the Security log size is kept.
pub fn undo() -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(STATE_PATH) else { return Ok(()) };
    let state: AuditState = toml::from_str(&text).map_err(|e| e.to_string())?;
    run("auditpol", &policy_args(state.policy_before))?;
    run(
        "auditpol",
        &["/resourceSACL".into(), "/remove".into(), "/type:File".into(), format!("/user:{{{}}}", state.sacl_sid)],
    )?;
    std::fs::remove_file(STATE_PATH).map_err(|e| e.to_string())
}
```

Em `crates/svc/src/main.rs`, em `run_uninstall`, logo antes do `Ok(())` final:

```rust
    if let Err(e) = rnetrbin_svc::audit::setup::undo() {
        eprintln!("warning: could not undo audit settings: {e}");
    }
```

- [ ] **Step 6: Conferir a sintaxe do `/access` no Windows real (manual, admin)**

Num PowerShell como administrador:

```powershell
auditpol /resourceSACL /set /type:File /success /user:"{S-1-5-32-545}" /access:0x10000
auditpol /resourceSACL /type:File /view
auditpol /resourceSACL /remove /type:File /user:"{S-1-5-32-545}"
```

Expected: o `/view` lista uma entrada com a máscara de DELETE para `BUILTIN\Users`, e o `/remove` a apaga. Se o `/set` recusar `0x10000`, troque `DELETE_MASK` pelo formato que ele aceitar e ajuste o teste `command_arguments`.

- [ ] **Step 7: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning.

- [ ] **Step 8: Commit**

```bash
git add crates/svc/Cargo.toml crates/svc/src/audit crates/svc/src/main.rs
git commit -m "feat(audit): enable global delete auditing and undo it on uninstall"
```

---

### Task 5: Assinar os eventos e ligar ao serviço

**Files:**
- Create: `crates/svc/src/audit/subscribe.rs`
- Modify: `crates/svc/src/audit/mod.rs`
- Modify: `crates/svc/src/service.rs` (`run`)

**Interfaces:**
- Consumes: `parse_4663` (Task 3), `setup::ensure` (Task 4), `Index::{add_delete_event, get_kv, set_kv, prune_delete_events}` e `attribution::fill_pending` (Task 2)
- Produces: `pub fn under_any(roots: &[String], path: &str) -> bool`; `pub fn spawn(cfg: &Config, stop: Arc<AtomicBool>) -> Vec<JoinHandle<()>>` em `crate::audit`

- [ ] **Step 1: Escrever o teste do filtro**

Crie `crates/svc/src/audit/subscribe.rs`:

```rust
//! EvtSubscribe on the Security channel: keep 4663 deletes under protected folders.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::nests;
use crate::index::Index;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_any_is_case_insensitive_and_needs_a_real_parent() {
        let roots = vec![r"D:\Dados".to_string(), r"E:\".to_string()];
        assert!(under_any(&roots, r"d:\dados\x\a.txt"));
        assert!(under_any(&roots, r"E:\a.txt"));
        assert!(!under_any(&roots, r"D:\DadosOld\a.txt"));
        assert!(!under_any(&roots, r"C:\Windows\Temp\a.tmp"));
    }
}
```

Em `audit/mod.rs`, acrescente `pub mod subscribe;`.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib audit::subscribe`
Expected: erro de compilação (`under_any` não existe).

- [ ] **Step 3: Implementar a assinatura**

Em `subscribe.rs`, antes do `#[cfg(test)]`:

```rust
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::EventLog::{
    EvtClose, EvtCreateBookmark, EvtRender, EvtRenderBookmark, EvtRenderEventXml, EvtSubscribe,
    EvtSubscribeActionDeliver, EvtSubscribeStartAfterBookmark, EvtSubscribeToFutureEvents, EvtUpdateBookmark,
    EVT_HANDLE, EVT_RENDER_FLAGS, EVT_SUBSCRIBE_NOTIFY_ACTION,
};

const QUERY: &str = "*[System[EventID=4663]]";
const BOOKMARK_KEY: &str = "audit.bookmark";
const SAVE_BOOKMARK_EVERY: Duration = Duration::from_secs(30);

/// The path lies inside one of the protected folders.
pub fn under_any(roots: &[String], path: &str) -> bool {
    let p = std::path::Path::new(path);
    roots.iter().any(|r| nests(std::path::Path::new(r), p))
}

struct Sink {
    tx: mpsc::Sender<String>,
    bookmark: Mutex<EVT_HANDLE>,
}

fn render(h: EVT_HANDLE, flags: EVT_RENDER_FLAGS) -> Option<String> {
    unsafe {
        let (mut used, mut props) = (0u32, 0u32);
        let _ = EvtRender(None, h, flags.0, 0, None, &mut used, &mut props);
        if used == 0 {
            return None;
        }
        let mut buf = vec![0u16; (used as usize).div_ceil(2)];
        EvtRender(None, h, flags.0, (buf.len() * 2) as u32, Some(buf.as_mut_ptr().cast()), &mut used, &mut props).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

unsafe extern "system" fn on_event(
    action: EVT_SUBSCRIBE_NOTIFY_ACTION,
    ctx: *const core::ffi::c_void,
    ev: EVT_HANDLE,
) -> u32 {
    let sink = &*(ctx as *const Sink);
    if action == EvtSubscribeActionDeliver {
        if let Some(xml) = render(ev, EvtRenderEventXml) {
            let _ = sink.tx.send(xml);
        }
        if let Ok(bm) = sink.bookmark.lock() {
            let _ = EvtUpdateBookmark(*bm, ev);
        }
    }
    0
}

fn save_bookmark(idx: &Index, sink: &Sink) {
    if let Ok(bm) = sink.bookmark.lock() {
        if let Some(xml) = render(*bm, EvtRenderBookmark) {
            if let Err(e) = idx.set_kv(BOOKMARK_KEY, &xml) {
                tracing::warn!("audit: cannot save bookmark: {e}");
            }
        }
    }
}

/// Runs until `stop`. Resumes after the saved bookmark, so a short service
/// restart loses no event still in the log.
pub fn run(idx: Arc<Index>, roots: Vec<String>, stop: Arc<AtomicBool>) -> Result<(), String> {
    let saved = idx.get_kv(BOOKMARK_KEY).map_err(|e| e.to_string())?;
    let bookmark = unsafe {
        match &saved {
            Some(xml) => EvtCreateBookmark(&HSTRING::from(xml.as_str())),
            None => EvtCreateBookmark(PCWSTR::null()),
        }
    }
    .map_err(|e| format!("EvtCreateBookmark: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let sink = Box::new(Sink { tx, bookmark: Mutex::new(bookmark) });
    let (start, flags) = match saved {
        Some(_) => (bookmark, EvtSubscribeStartAfterBookmark.0),
        None => (EVT_HANDLE::default(), EvtSubscribeToFutureEvents.0),
    };
    let sub = unsafe {
        EvtSubscribe(
            None,
            None,
            &HSTRING::from("Security"),
            &HSTRING::from(QUERY),
            start,
            Some(&*sink as *const Sink as *const core::ffi::c_void),
            Some(on_event),
            flags,
        )
    }
    .map_err(|e| format!("EvtSubscribe: {e}"))?;
    tracing::info!("audit: subscribed to Security 4663");

    let mut last_save = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(xml) => {
                if let Some(e) = crate::audit::event::parse_4663(&xml) {
                    if under_any(&roots, &e.path) {
                        if let Err(err) = idx.add_delete_event(&e) {
                            tracing::warn!("audit: cannot store event: {err}");
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_save.elapsed() >= SAVE_BOOKMARK_EVERY {
            save_bookmark(&idx, &sink);
            last_save = Instant::now();
        }
    }
    // Close the subscription first: no callback may run after `sink` is dropped.
    unsafe {
        let _ = EvtClose(sub);
    }
    save_bookmark(&idx, &sink);
    unsafe {
        let _ = EvtClose(bookmark);
    }
    drop(sink);
    Ok(())
}
```

- [ ] **Step 4: Implementar o `spawn` e ligar ao serviço**

Substitua `crates/svc/src/audit/mod.rs` por:

```rust
//! Who deleted a file: Windows object-access auditing (global SACL, DELETE only)
//! plus a subscription to Security event 4663.
pub mod event;
pub mod setup;
pub mod subscribe;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::config::Config;
use crate::index::Index;

const FILL_EVERY: Duration = Duration::from_secs(5);
const PRUNE_EVERY_SECS: i64 = 3600;
const KEEP_EVENTS_SECS: i64 = 24 * 3600;

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Configure auditing and start the subscription and attribution threads.
/// Never fails: without auditing the bin keeps working and users show as unknown.
pub fn spawn(cfg: &Config, stop: Arc<AtomicBool>) -> Vec<JoinHandle<()>> {
    if let Err(e) = setup::ensure() {
        tracing::warn!("audit: not configured ({e}); deleted-by will show as unknown");
    }
    if let Some(parent) = cfg.db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let idx = match Index::open(&cfg.db_path) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            tracing::warn!("audit: cannot open index: {e}");
            return vec![];
        }
    };
    let roots: Vec<String> = cfg.folders.iter().map(|f| f.path.display().to_string()).collect();
    let (sub_idx, sub_stop) = (idx.clone(), stop.clone());
    let subscription = std::thread::spawn(move || {
        if let Err(e) = subscribe::run(sub_idx, roots, sub_stop) {
            tracing::warn!("audit: subscription stopped: {e}");
        }
    });
    let attribution = std::thread::spawn(move || {
        let mut last_prune = 0;
        while !stop.load(Ordering::Relaxed) {
            let now = now_secs();
            if let Err(e) = crate::attribution::fill_pending(&idx, now) {
                tracing::warn!("audit: attribution failed: {e}");
            }
            if now - last_prune >= PRUNE_EVERY_SECS {
                if let Err(e) = idx.prune_delete_events(now - KEEP_EVENTS_SECS) {
                    tracing::warn!("audit: prune failed: {e}");
                }
                last_prune = now;
            }
            crate::service::sleep_interruptible(FILL_EVERY, &stop);
        }
    });
    vec![subscription, attribution]
}
```

Em `crates/svc/src/service.rs`, substitua o corpo de `pub fn run(...)` por:

```rust
    let mut driver = WinDriver::new(&cfg);
    let audit = crate::audit::spawn(&cfg, stop.clone());
    let s2 = stop.clone();
    let sleep = move |d: Duration| sleep_interruptible(d, &s2);
    let res = if take_over_original {
        run_with(cfg, stop.clone(), &handback_on_exit, &WinScm, &FileStore::new(STATE_PATH), &mut driver, &sleep)
    } else {
        run_with(cfg, stop.clone(), &handback_on_exit, &NoScm, &NoStore, &mut driver, &sleep)
    };
    // run_with returns on STOP or early exit; make sure the audit threads stop too.
    stop.store(true, Ordering::Relaxed);
    for h in audit {
        if h.join().is_err() {
            tracing::error!("audit thread panicked");
        }
    }
    res
```

- [ ] **Step 5: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning. Se alguma assinatura de `EvtSubscribe`/`EvtRender`/`EvtCreateBookmark` do `windows` 0.58 diferir, ajuste os argumentos ao que o compilador pedir, mantendo a lógica.

- [ ] **Step 6: Verificar em execução (manual, admin)**

Com um `config.toml` válido apontando uma pasta de teste (ex.: `C:\teste-share`), num PowerShell como administrador:

```powershell
.\target\release\rnetrbin-svc.exe --console
```

Em outro terminal: `"x" > C:\teste-share\a.txt; Remove-Item C:\teste-share\a.txt`.
Expected: no log aparece `audit: subscribed to Security 4663`. Depois, `sqlite3 C:\ProgramData\rnetrbin\index.db "select * from delete_events"` mostra `C:\TESTE-SHARE\A.TXT` com o seu usuário. Ctrl+C encerra sem travar.

- [ ] **Step 7: Commit**

```bash
git add crates/svc/src/audit crates/svc/src/service.rs
git commit -m "feat(audit): subscribe to 4663 and attribute users to backups"
```

---

### Task 6: Ícones Material e temas

**Files:**
- Create: `crates/svc/assets/MaterialIcons-Regular.ttf`, `crates/svc/assets/MaterialIcons-Regular.codepoints`, `crates/svc/assets/LICENSE-MaterialIcons.txt`
- Create: `crates/svc/src/gui/icons.rs`, `crates/svc/src/gui/theme.rs`
- Create: `crates/svc/src/gui/mod.rs` (temporário, só declara os módulos)
- Modify: `crates/svc/src/lib.rs`

**Interfaces:**
- Produces:
  - `gui::icons::{FONT_BYTES: &[u8], FONT: Font, USED: &[&str], glyph(name: &str) -> char, icon<'a>(name: &str, size: f32, color: Color) -> Text<'a>}`
  - `gui::theme::{Pal (Copy), pal(t: ThemeName) -> Pal, iced_theme(t: ThemeName) -> Theme, flat_button(p: Pal, selected: bool), tab_button(p: Pal, active: bool, accent: bool), surface(p: Pal, bg: Color)}`

- [ ] **Step 1: Baixar a fonte e a licença**

```bash
mkdir -p crates/svc/assets
curl -sSfL -o crates/svc/assets/MaterialIcons-Regular.ttf https://raw.githubusercontent.com/google/material-design-icons/master/font/MaterialIcons-Regular.ttf
curl -sSfL -o crates/svc/assets/MaterialIcons-Regular.codepoints https://raw.githubusercontent.com/google/material-design-icons/master/font/MaterialIcons-Regular.codepoints
curl -sSfL -o crates/svc/assets/LICENSE-MaterialIcons.txt https://raw.githubusercontent.com/google/material-design-icons/master/LICENSE
head -c 4 crates/svc/assets/MaterialIcons-Regular.ttf | od -An -tx1
grep -c . crates/svc/assets/MaterialIcons-Regular.codepoints
```

Expected: `00 01 00 00` (TrueType) e mais de 2000 linhas de codepoints. A `.gitignore` só ignora `*.sys`, então os assets entram no git (licença Apache 2.0 junto).

- [ ] **Step 2: Escrever os testes**

Crie `crates/svc/src/gui/icons.rs`:

```rust
//! Material Icons (Google, Apache 2.0) embedded in the exe. Names are resolved
//! through the official `.codepoints` file, so no codepoint is typed by hand.
use std::collections::HashMap;
use std::sync::OnceLock;

use iced::widget::{text, Text};
use iced::{Color, Font};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_used_icon_exists_in_the_font() {
        for name in USED {
            let _ = glyph(name);
        }
        assert_eq!(glyph("delete"), '\u{e872}');
        assert_eq!(glyph("restore"), '\u{e8b3}');
    }

    #[test]
    fn font_is_a_truetype_file() {
        assert_eq!(&FONT_BYTES[..4], &[0, 1, 0, 0]);
    }
}
```

Crie `crates/svc/src/gui/theme.rs`:

```rust
//! Rust (default), Branco and Preto palettes, copied from the approved mockup.
use iced::theme::Palette;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};

use crate::config::ThemeName;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_match_the_mockup() {
        assert_eq!(pal(ThemeName::Rust).bg, Color::from_rgb8(0xe1, 0xe1, 0xdb));
        assert_eq!(pal(ThemeName::Rust).acc, Color::from_rgb8(0xb7, 0x41, 0x0e));
        assert_eq!(pal(ThemeName::Branco).bg, Color::WHITE);
        assert_eq!(pal(ThemeName::Preto).bg, Color::from_rgb8(0x0d, 0x11, 0x17));
    }
}
```

Crie `crates/svc/src/gui/mod.rs` (temporário):

```rust
//! Desktop window (iced). Rebuilt in Undelete style; see the spec of 2026-10-14.
pub mod icons;
pub mod theme;
```

A janela antiga sai do caminho do módulo novo (o Rust não aceita `gui.rs` e `gui/mod.rs` juntos):

```bash
git mv crates/svc/src/gui.rs crates/svc/src/gui_old.rs
```

Em `crates/svc/src/lib.rs`, troque `pub mod gui;` por:

```rust
pub mod gui_old;
pub mod gui;
```

e em `crates/svc/src/main.rs` troque `rnetrbin_svc::gui::run()` por `rnetrbin_svc::gui_old::run()` (a janela nova entra na Task 8).

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib gui::`
Expected: erro de compilação (`glyph`, `pal` não existem).

- [ ] **Step 4: Implementar os ícones**

Em `icons.rs`, antes do `#[cfg(test)]`:

```rust
pub const FONT_BYTES: &[u8] = include_bytes!("../../assets/MaterialIcons-Regular.ttf");
const CODEPOINTS: &str = include_str!("../../assets/MaterialIcons-Regular.codepoints");
/// Family name inside the TTF.
pub const FONT: Font = Font::with_name("Material Icons");

/// Every icon the window uses (checked against the font by a test).
pub const USED: &[&str] = &[
    "restore", "drive_file_move", "delete_forever", "history", "refresh", "search",
    "create_new_folder", "rule", "folder_off", "save", "install_desktop", "play_circle",
    "stop_circle", "delete", "folder_special", "folder", "folder_open", "description",
    "arrow_upward", "check_circle", "error", "delete_sweep", "info", "logout",
];

fn map() -> &'static HashMap<&'static str, char> {
    static MAP: OnceLock<HashMap<&'static str, char>> = OnceLock::new();
    MAP.get_or_init(|| {
        CODEPOINTS
            .lines()
            .filter_map(|l| {
                let (name, hex) = l.split_once(' ')?;
                Some((name, char::from_u32(u32::from_str_radix(hex.trim(), 16).ok()?)?))
            })
            .collect()
    })
}

/// The font character for a Material icon name. Panics on an unknown name,
/// which the `every_used_icon_exists_in_the_font` test rules out.
pub fn glyph(name: &str) -> char {
    *map().get(name).unwrap_or_else(|| panic!("unknown Material icon {name:?}"))
}

pub fn icon<'a>(name: &str, size: f32, color: Color) -> Text<'a> {
    text(glyph(name).to_string()).font(FONT).size(size).color(color)
}
```

- [ ] **Step 5: Implementar os temas**

Em `theme.rs`, antes do `#[cfg(test)]`:

```rust
/// Colors used by our own widgets (iced's palette only covers its defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pal {
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub panel: Color,
    pub line: Color,
    pub soft: Color,
    pub acc: Color,
    pub acc_fg: Color,
    pub sel: Color,
    pub ok: Color,
    pub danger: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub fn pal(t: ThemeName) -> Pal {
    match t {
        ThemeName::Rust => Pal {
            bg: hex(0xe1e1db), fg: hex(0x262625), muted: hex(0x6e6b5e), panel: hex(0xecebe5),
            line: hex(0xc4c3b8), soft: hex(0xd4d3cb), acc: hex(0xb7410e), acc_fg: Color::WHITE,
            sel: hex(0xf0d5c2), ok: hex(0x3d5a1e), danger: hex(0xa3260f),
        },
        ThemeName::Branco => Pal {
            bg: Color::WHITE, fg: hex(0x1f2328), muted: hex(0x59636e), panel: hex(0xf6f8fa),
            line: hex(0xd1d9e0), soft: hex(0xeaeef2), acc: hex(0x0969da), acc_fg: Color::WHITE,
            sel: hex(0xddf4ff), ok: hex(0x1a7f37), danger: hex(0xcf222e),
        },
        ThemeName::Preto => Pal {
            bg: hex(0x0d1117), fg: hex(0xe6edf3), muted: hex(0x9198a1), panel: hex(0x151b23),
            line: hex(0x30363d), soft: hex(0x21262d), acc: hex(0x2f81f7), acc_fg: Color::WHITE,
            sel: hex(0x1f3a5f), ok: hex(0x3fb950), danger: hex(0xf85149),
        },
    }
}

/// iced theme so default widgets (text inputs, scrollbars, checkboxes) match.
pub fn iced_theme(t: ThemeName) -> Theme {
    let p = pal(t);
    let name = match t {
        ThemeName::Rust => "Rust",
        ThemeName::Branco => "Branco",
        ThemeName::Preto => "Preto",
    };
    Theme::custom(
        name,
        Palette { background: p.bg, text: p.fg, primary: p.acc, success: p.ok, danger: p.danger, ..Palette::LIGHT },
    )
}

/// Borderless button (ribbon, tree, list rows); `selected` paints the row.
pub fn flat_button(p: Pal, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match (selected, status) {
            (true, _) => Some(p.sel),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(p.soft),
            _ => None,
        };
        button::Style {
            background: bg.map(Background::Color),
            text_color: if matches!(status, button::Status::Disabled) { p.muted } else { p.fg },
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// Tab of the ribbon bar. `accent` is the "Arquivo" tab.
pub fn tab_button(p: Pal, active: bool, accent: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, _| {
        let (bg, fg) = match (accent, active) {
            (true, _) => (p.acc, p.acc_fg),
            (false, true) => (p.bg, p.fg),
            (false, false) => (p.panel, p.fg),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border { color: if active { p.line } else { Color::TRANSPARENT }, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

/// Plain area with a background color and a thin border.
pub fn surface(p: Pal, bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        text_color: Some(p.fg),
        border: Border { color: p.line, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}
```

- [ ] **Step 6: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning.

- [ ] **Step 7: Commit**

```bash
git add -A crates/svc/assets crates/svc/src
git commit -m "feat(gui): embed Material Icons and add Rust, Branco and Preto themes"
```

---

### Task 7: Árvore e conteúdo da pasta (lógica pura)

**Files:**
- Create: `crates/svc/src/gui/tree.rs`
- Create: `crates/svc/src/gui/listing.rs`
- Modify: `crates/svc/src/gui/mod.rs`

**Interfaces:**
- Consumes: `crate::index::Version` (Task 2)
- Produces:
  - `tree::{key(p: &str) -> String, parent(p: &str) -> &str, NodeInfo { count: usize, size: i64, last: i64 }, Tree}` com `Tree::build(roots: &[String], files: &[(String, i64, i64)]) -> Tree`, `children(&self, dir: &str) -> Vec<String>`, `info(&self, dir: &str) -> NodeInfo`, `is_root(&self, dir: &str) -> bool`, `root_of(&self, path: &str) -> Option<String>`
  - `listing::{SortCol { Name, Copies, User, Size, Deleted }, View<'a> { cur, query, all_copies, copies_of, sort, limit }, DirRow { path, name, info }, Listing { dirs, files, total_files }, build(tree: &Tree, rows: &[Version], v: &View) -> Listing, split_path, fmt_size, fmt_time, split_masks}`

- [ ] **Step 1: Escrever os testes da árvore**

Crie `crates/svc/src/gui/tree.rs`:

```rust
//! The bin as a folder tree: protected folders as roots, below them only the
//! subfolders that contain deleted files, with count/size/last-deleted totals.
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, size: i64, t: i64) -> (String, i64, i64) {
        (p.to_string(), size, t)
    }

    fn sample() -> Tree {
        Tree::build(
            &[r"D:\Dados\Contabil".into(), r"D:\Dados\Fiscal".into(), r"E:\".into()],
            &[
                f(r"D:\Dados\Contabil\Clientes\Acme\a.xlsx", 10, 5),
                f(r"D:\Dados\Contabil\Clientes\Beta\b.xlsx", 20, 7),
                f(r"D:\Dados\Contabil\c.xlsx", 1, 9),
                f(r"E:\x\y.txt", 3, 2),
            ],
        )
    }

    #[test]
    fn roots_keep_config_order_and_children_only_with_deleted_files() {
        let t = sample();
        assert_eq!(t.children(""), [r"D:\Dados\Contabil", r"D:\Dados\Fiscal", r"E:"]);
        assert_eq!(t.children(r"D:\Dados\Contabil"), [r"D:\Dados\Contabil\Clientes"]);
        assert_eq!(t.children(r"D:\Dados\Contabil\Clientes"), [r"D:\Dados\Contabil\Clientes\Acme", r"D:\Dados\Contabil\Clientes\Beta"]);
        assert!(t.children(r"D:\Dados\Fiscal").is_empty());
    }

    #[test]
    fn info_aggregates_count_size_and_last() {
        let t = sample();
        assert_eq!(t.info(""), NodeInfo { count: 4, size: 34, last: 9 });
        assert_eq!(t.info(r"D:\Dados\Contabil"), NodeInfo { count: 3, size: 31, last: 9 });
        assert_eq!(t.info(r"D:\Dados\Contabil\Clientes"), NodeInfo { count: 2, size: 30, last: 7 });
        assert_eq!(t.info(r"D:\Dados\Fiscal"), NodeInfo::default());
    }

    #[test]
    fn tree_is_case_insensitive() {
        let t = Tree::build(&[r"D:\Dados".into()], &[f(r"d:\dados\SUB\a.txt", 1, 1), f(r"D:\Dados\sub\b.txt", 1, 2)]);
        assert_eq!(t.children(r"D:\DADOS").len(), 1);
        assert_eq!(t.info(r"d:\dados\sub").count, 2);
    }

    #[test]
    fn orphans_go_under_drive_root() {
        let t = Tree::build(&[r"D:\Dados".into()], &[f(r"D:\Antiga\a.txt", 1, 1), f(r"D:\Dados\b.txt", 1, 2)]);
        assert_eq!(t.children(""), [r"D:\Dados", "D:"]);
        assert_eq!(t.children("D:"), [r"D:\Antiga"]);
        assert_eq!(t.info("D:").count, 1, "configured folder is not counted twice");
        assert_eq!(t.root_of(r"D:\Antiga\a.txt").as_deref(), Some("D:"));
        assert_eq!(t.root_of(r"D:\Dados\b.txt").as_deref(), Some(r"D:\Dados"));
        assert!(t.is_root("d:"));
    }
}
```

- [ ] **Step 2: Escrever os testes da lista**

Crie `crates/svc/src/gui/listing.rs`:

```rust
//! What the list shows for the current folder: its subfolders (from the tree)
//! and the files directly in it; or, with a search, everything below it.
use std::collections::HashMap;

use super::tree::{key, parent, NodeInfo, Tree};
use crate::index::Version;

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: i64, p: &str, utc: i64, user: Option<&str>, copies: i64) -> Version {
        Version { id, original_path: p.into(), size: id * 10, deleted_utc: utc, user_name: user.map(String::from), copies }
    }

    fn rows() -> Vec<Version> {
        vec![
            v(1, r"D:\Dados\a.txt", 5, Some("AUSTER\\ana"), 2),
            v(2, r"D:\Dados\A.TXT", 9, None, 2),
            v(3, r"D:\Dados\Sub\b.txt", 7, Some("AUSTER\\bia"), 1),
            v(4, r"D:\Dados\Sub\Deep\c.txt", 3, None, 1),
        ]
    }

    fn view<'a>(cur: &'a str, query: &'a str) -> View<'a> {
        View { cur, query, all_copies: true, copies_of: None, sort: (SortCol::Deleted, false), limit: 1000 }
    }

    fn tree(rows: &[Version]) -> Tree {
        let files: Vec<_> = rows.iter().map(|r| (r.original_path.clone(), r.size, r.deleted_utc)).collect();
        Tree::build(&[r"D:\Dados".into()], &files)
    }

    fn ids(l: &Listing) -> Vec<i64> {
        l.files.iter().map(|f| f.id).collect()
    }

    #[test]
    fn shows_direct_files_and_subfolders_only() {
        let r = rows();
        let l = build(&tree(&r), &r, &view(r"D:\Dados", ""));
        assert_eq!(ids(&l), [2, 1]);
        assert_eq!(l.dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Sub"]);
        assert_eq!(l.dirs[0].info.count, 2);
        let root = build(&tree(&r), &r, &view("", ""));
        assert!(root.files.is_empty());
        assert_eq!(root.dirs[0].name, r"D:\Dados");
    }

    #[test]
    fn search_looks_below_current_folder_by_path_or_user() {
        let r = rows();
        let t = tree(&r);
        assert_eq!(ids(&build(&t, &r, &view(r"D:\Dados", "c.txt"))), [4]);
        assert_eq!(ids(&build(&t, &r, &view(r"D:\Dados\Sub", "bia"))), [3]);
        assert!(build(&t, &r, &view(r"D:\Dados\Sub", "bia")).dirs.is_empty());
    }

    #[test]
    fn latest_copy_only_and_copies_of() {
        let r = rows();
        let t = tree(&r);
        let mut v1 = view(r"D:\Dados", "");
        v1.all_copies = false;
        assert_eq!(ids(&build(&t, &r, &v1)), [2]);
        let mut v2 = view(r"D:\Dados\Sub", "");
        v2.copies_of = Some(r"d:\dados\a.txt");
        let l = build(&t, &r, &v2);
        assert_eq!(ids(&l), [2, 1]);
        assert!(l.dirs.is_empty());
    }

    #[test]
    fn sorts_by_each_column() {
        let r = rows();
        let t = tree(&r);
        let mut q = view("", "txt");
        q.sort = (SortCol::Size, true);
        assert_eq!(ids(&build(&t, &r, &q)), [1, 2, 3, 4]);
        q.sort = (SortCol::User, true);
        assert_eq!(ids(&build(&t, &r, &q))[..2], [1, 3]);
        q.sort = (SortCol::Name, false);
        assert_eq!(ids(&build(&t, &r, &q))[0], 4);
    }

    #[test]
    fn listing_truncates_and_reports_total() {
        let many: Vec<Version> = (1..=1500).map(|i| v(i, &format!(r"D:\Dados\f{i}.txt"), i, None, 1)).collect();
        let mut q = view(r"D:\Dados", "");
        q.limit = 1000;
        let l = build(&tree(&many), &many, &q);
        assert_eq!(l.files.len(), 1000);
        assert_eq!(l.total_files, 1500);
        assert_eq!(l.files[0].id, 1500, "newest first");
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(split_path(r"D:\a\b\c.txt"), ("c.txt", r"D:\a\b"));
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(split_masks(" *.docx ;; Relatório *.xlsx "), ["*.docx", "Relatório *.xlsx"]);
    }
}
```

Em `gui/mod.rs`, acrescente `pub mod listing;` e `pub mod tree;`.

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib gui::`
Expected: erro de compilação.

- [ ] **Step 4: Implementar a árvore**

Em `tree.rs`, antes do `#[cfg(test)]`:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeInfo {
    pub count: usize,
    pub size: i64,
    pub last: i64,
}

/// Lookup key: case-insensitive, no trailing separator ("" is the whole bin).
pub fn key(p: &str) -> String {
    p.trim_end_matches('\\').to_lowercase()
}

/// Folder of a path ("" when there is none).
pub fn parent(p: &str) -> &str {
    p.rfind('\\').map(|i| &p[..i]).unwrap_or("")
}

#[derive(Debug, Default, Clone)]
pub struct Tree {
    roots: Vec<String>,
    display: BTreeMap<String, String>,
    info: BTreeMap<String, NodeInfo>,
    children: BTreeMap<String, BTreeSet<String>>,
}

impl Tree {
    /// `roots`: protected folders. `files`: (original_path, size, deleted_utc).
    /// A file under no root goes under its drive (`D:`), so backups of a folder
    /// removed from the config stay reachable.
    pub fn build(roots: &[String], files: &[(String, i64, i64)]) -> Tree {
        let mut t = Tree::default();
        for r in roots {
            t.add_root(r.trim_end_matches('\\'));
        }
        for (path, size, utc) in files {
            let root = t.root_of(path).unwrap_or_else(|| {
                let drive = path.split('\\').next().unwrap_or("").to_string();
                t.add_root(&drive);
                drive
            });
            let root_key = key(&root);
            let mut dir = parent(path).to_string();
            loop {
                let k = key(&dir);
                t.display.entry(k.clone()).or_insert_with(|| dir.clone());
                let e = t.info.entry(k.clone()).or_default();
                e.count += 1;
                e.size += size;
                e.last = e.last.max(*utc);
                if k == root_key || dir.is_empty() {
                    break;
                }
                let up = parent(&dir).to_string();
                t.children.entry(key(&up)).or_default().insert(k);
                dir = up;
            }
            let all = t.info.entry(String::new()).or_default();
            all.count += 1;
            all.size += size;
            all.last = all.last.max(*utc);
        }
        t
    }

    fn add_root(&mut self, r: &str) {
        let k = key(r);
        if self.roots.iter().any(|x| key(x) == k) {
            return;
        }
        self.roots.push(r.to_string());
        self.display.insert(k, r.to_string());
    }

    /// Subfolders with deleted files ("" lists the roots, in config order).
    pub fn children(&self, dir: &str) -> Vec<String> {
        if dir.is_empty() {
            return self.roots.clone();
        }
        let mut v: Vec<String> = self
            .children
            .get(&key(dir))
            .into_iter()
            .flatten()
            .filter(|k| !self.roots.iter().any(|r| &key(r) == *k))
            .map(|k| self.display[k].clone())
            .collect();
        v.sort_by_key(|d| d.to_lowercase());
        v
    }

    pub fn info(&self, dir: &str) -> NodeInfo {
        self.info.get(&key(dir)).copied().unwrap_or_default()
    }

    pub fn is_root(&self, dir: &str) -> bool {
        self.roots.iter().any(|r| key(r) == key(dir))
    }

    /// The deepest root containing `path` (a configured folder beats its drive).
    pub fn root_of(&self, path: &str) -> Option<String> {
        let k = key(path);
        self.roots
            .iter()
            .filter(|r| {
                let rk = key(r);
                k == rk || k.starts_with(&format!("{rk}\\"))
            })
            .max_by_key(|r| r.len())
            .cloned()
    }
}
```

- [ ] **Step 5: Implementar a lista**

Em `listing.rs`, antes do `#[cfg(test)]`:

```rust
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortCol {
    Name,
    Copies,
    User,
    Size,
    Deleted,
}

pub struct View<'a> {
    /// Current folder ("" = the bin's root).
    pub cur: &'a str,
    /// Non-empty: search `cur` and everything below it.
    pub query: &'a str,
    /// false: only the newest backup of each original path.
    pub all_copies: bool,
    /// Some: every backup of this one original path ("Ver cópias").
    pub copies_of: Option<&'a str>,
    /// (column, ascending)
    pub sort: (SortCol, bool),
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirRow {
    pub path: String,
    pub name: String,
    pub info: NodeInfo,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub dirs: Vec<DirRow>,
    pub files: Vec<Version>,
    /// Files before truncation to `limit`.
    pub total_files: usize,
}

pub fn build(tree: &Tree, rows: &[Version], v: &View) -> Listing {
    let q = v.query.trim().to_lowercase();
    let cur = key(v.cur);
    let below = |p: &str| cur.is_empty() || key(p).starts_with(&format!("{cur}\\"));
    let mut files: Vec<Version> = rows
        .iter()
        .filter(|r| match (v.copies_of, q.is_empty()) {
            (Some(p), _) => key(&r.original_path) == key(p),
            (None, false) => {
                below(&r.original_path)
                    && (r.original_path.to_lowercase().contains(&q)
                        || r.user_name.as_deref().unwrap_or("").to_lowercase().contains(&q))
            }
            (None, true) => key(parent(&r.original_path)) == cur,
        })
        .cloned()
        .collect();
    if !v.all_copies && v.copies_of.is_none() {
        let mut newest: HashMap<String, (i64, i64)> = HashMap::new();
        for f in &files {
            let e = newest.entry(key(&f.original_path)).or_insert((f.deleted_utc, f.id));
            *e = (*e).max((f.deleted_utc, f.id));
        }
        files.retain(|f| newest[&key(&f.original_path)] == (f.deleted_utc, f.id));
    }
    sort(&mut files, v.sort);
    let total_files = files.len();
    files.truncate(v.limit);
    let dirs = if q.is_empty() && v.copies_of.is_none() {
        tree.children(v.cur)
            .into_iter()
            .map(|p| DirRow {
                name: if v.cur.is_empty() { p.clone() } else { split_path(&p).0.to_string() },
                info: tree.info(&p),
                path: p,
            })
            .collect()
    } else {
        Vec::new()
    };
    Listing { dirs, files, total_files }
}

fn sort(files: &mut [Version], (col, asc): (SortCol, bool)) {
    files.sort_by(|a, b| {
        let o = match col {
            SortCol::Name => split_path(&a.original_path).0.to_lowercase().cmp(&split_path(&b.original_path).0.to_lowercase()),
            SortCol::Copies => a.copies.cmp(&b.copies),
            SortCol::User => a.user_name.as_deref().unwrap_or("~").to_lowercase().cmp(&b.user_name.as_deref().unwrap_or("~").to_lowercase()),
            SortCol::Size => a.size.cmp(&b.size),
            SortCol::Deleted => (a.deleted_utc, a.id).cmp(&(b.deleted_utc, b.id)),
        };
        if asc { o } else { o.reverse() }
    });
}

/// (file name, folder) of a Windows path.
pub fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('\\') {
        Some(i) => (&p[i + 1..], &p[..i]),
        None => (p, ""),
    }
}

pub fn fmt_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    match b {
        b if b >= GB => format!("{:.1} GB", b / GB),
        b if b >= 1024.0 * 1024.0 => format!("{:.1} MB", b / 1024.0 / 1024.0),
        b if b >= 1024.0 => format!("{:.0} KB", b / 1024.0),
        b => format!("{b} B"),
    }
}

pub fn fmt_time(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%d/%m/%Y %H:%M").to_string(),
        chrono::LocalResult::None => unix.to_string(),
    }
}

/// Masks typed in the GUI are separated by `;` (names may contain spaces).
pub fn split_masks(s: &str) -> Vec<String> {
    s.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect()
}
```

- [ ] **Step 6: Rodar os testes**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning (se o clippy acusar `dead_code` em itens ainda não usados pela janela, adicione `#![allow(dead_code)]` no topo de `gui/mod.rs` e remova-o na Task 8).

- [ ] **Step 7: Commit**

```bash
git add crates/svc/src/gui
git commit -m "feat(gui): bin tree and folder listing with search, copies and sorting"
```

---

### Task 8: A janela nova

**Files:**
- Modify: `crates/svc/src/gui/mod.rs` (substitui o conteúdo temporário)
- Create: `crates/svc/src/gui/home.rs`, `crates/svc/src/gui/folders.rs`, `crates/svc/src/gui/settings.rs`
- Delete: `crates/svc/src/gui_old.rs` (renomeado na Task 6)
- Modify: `crates/svc/src/lib.rs`, `crates/svc/src/main.rs`

**Interfaces:**
- Consumes: tudo das Tasks 1–7 (`Config`, `FolderRule`, `ThemeName`, `CONFIG_PATH`; `Index::{open, all_versions}`, `Version`; `restore::restore`, `retention::{purge}`; `install::{install, start_service, stop_own_service, service_state, SvcState, WinHost, EMBEDDED_DRIVER, OWN_SERVICE, DRIVER_SERVICE}`; `audit::setup::{status, AuditStatus}`; `tree`, `listing`, `icons`, `theme`)
- Produces: `pub fn gui::run() -> iced::Result`; `settings::Form::{from_config, to_config}`

- [ ] **Step 1: Escrever os testes do formulário**

Crie `crates/svc/src/gui/settings.rs` com:

```rust
//! "Configurações" and "Arquivo" tabs.
use std::path::PathBuf;

use iced::widget::{button, column, container, row, text, text_input, Space};
use iced::{Alignment, Element, Length};

use super::theme;
use super::{App, Msg};
use crate::config::{Config, FolderRule, ThemeName, CONFIG_PATH};
use crate::install::SvcState;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_round_trips_config() {
        let c = Config {
            folders: vec![FolderRule::with_defaults(r"D:\Dados".into())],
            max_age_days: Some(30),
            theme: ThemeName::Preto,
            ..Config::default()
        };
        assert_eq!(Form::from_config(&c).to_config(&c.folders, c.theme).unwrap(), c);
    }

    #[test]
    fn form_parses_decimal_comma_and_empty_age_and_rejects_bad_numbers() {
        let f = Form { max_gb: "1,5".into(), max_age_days: " ".into(), ..Form::from_config(&Config::default()) };
        let c = f.to_config(&[], ThemeName::Rust).unwrap();
        assert_eq!(c.max_bin_bytes, (1.5 * GB) as u64);
        assert_eq!(c.max_age_days, None);
        assert!(Form { max_gb: "0".into(), ..f.clone() }.to_config(&[], ThemeName::Rust).is_err());
        assert!(Form { max_age_days: "x".into(), ..f }.to_config(&[], ThemeName::Rust).is_err());
    }
}
```

Substitua `crates/svc/src/gui/mod.rs` por (o resto do arquivo entra no Step 3):

```rust
//! Desktop window (iced), Undelete style: ribbon tabs, bin tree, folder listing.
mod folders;
mod home;
mod settings;
pub mod icons;
pub mod listing;
pub mod theme;
pub mod tree;
```

Em `lib.rs`, remova a linha `pub mod gui_old;`. Em `main.rs`, volte para `rnetrbin_svc::gui::run()`. Apague a janela antiga: `git rm crates/svc/src/gui_old.rs`.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p rnetrbin-svc --lib gui::settings`
Expected: erro de compilação (`Form`, `App`, `Msg` não existem).

- [ ] **Step 3: Escrever `gui/mod.rs`**

Acrescente ao `gui/mod.rs`, depois das declarações de módulo:

```rust
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::{column, container, row, text, Space};
use iced::{keyboard, Alignment, Element, Length, Subscription, Task, Theme};

use crate::audit::setup::{self as audit_setup, AuditStatus};
use crate::config::{Config, FolderRule, ThemeName, CONFIG_PATH};
use crate::index::{Index, Version};
use crate::install::{self, SvcState};
use listing::{Listing, SortCol, View};
use tree::Tree;

/// Files shown at once in the list; refine the search to see more.
const MAX_ROWS: usize = 1000;

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .font(icons::FONT_BYTES)
        .window_size((1150.0, 720.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Arquivo,
    Inicio,
    Pastas,
    Config,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Tab(Tab),
    Modifiers(keyboard::Modifiers),
    // Início
    Search(String),
    Go(String),
    ToggleNode(String),
    Click(i64),
    Sort(SortCol),
    AllCopies(bool),
    ShowCopies,
    CloseCopies,
    Restore,
    RestoreTo,
    Purge,
    Refresh,
    // Pastas protegidas
    SelectFolder(usize),
    AddFolder,
    EditRules,
    RuleInclude(String),
    RuleExclude(String),
    RuleSave,
    RuleCancel,
    RemoveFolder,
    ConfirmRemove,
    // Configurações
    Field(settings::Field, String),
    Theme(ThemeName),
    Save,
    Install,
    Start,
    Stop,
    // Arquivo
    Exit,
    Done(Result<String, String>),
}

pub struct App {
    tab: Tab,
    theme: ThemeName,
    form: settings::Form,
    folders: Vec<FolderRule>,
    folder_sel: Option<usize>,
    rule_edit: Option<folders::RuleEdit>,
    confirm_remove: bool,
    idx: Option<Arc<Index>>,
    rows: Vec<Version>,
    tree: Tree,
    listing: Listing,
    cur: String,
    expanded: BTreeSet<String>,
    query: String,
    all_copies: bool,
    copies_of: Option<String>,
    sort: (SortCol, bool),
    selected: BTreeSet<i64>,
    anchor: Option<i64>,
    mods: keyboard::Modifiers,
    svc: SvcState,
    driver: SvcState,
    audit: AuditStatus,
    status: String,
    busy: bool,
}

/// Run blocking work off the UI thread and report its outcome.
fn work(f: impl FnOnce() -> Result<String, String> + Send + 'static) -> Task<Msg> {
    Task::perform(async move { f() }, Msg::Done)
}

fn open_index(db: &Path) -> Result<Arc<Index>, String> {
    if let Some(parent) = db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Index::open(db).map(Arc::new).map_err(|e| format!("Não abriu o índice {}: {e}", db.display()))
}

fn summary(verb: &str, n: usize, errs: Vec<String>) -> Result<String, String> {
    if errs.is_empty() {
        Ok(format!("{n} {verb}."))
    } else {
        Err(format!("{} de {n} falharam: {}", errs.len(), errs.join("; ")))
    }
}

impl App {
    fn new() -> (App, Task<Msg>) {
        let mut app = App {
            tab: Tab::Inicio,
            theme: ThemeName::default(),
            form: settings::Form::from_config(&Config::default()),
            folders: Vec::new(),
            folder_sel: None,
            rule_edit: None,
            confirm_remove: false,
            idx: None,
            rows: Vec::new(),
            tree: Tree::default(),
            listing: Listing::default(),
            cur: String::new(),
            expanded: BTreeSet::from([String::new()]),
            query: String::new(),
            all_copies: true,
            copies_of: None,
            sort: (SortCol::Deleted, false),
            selected: BTreeSet::new(),
            anchor: None,
            mods: keyboard::Modifiers::default(),
            svc: SvcState::NotInstalled,
            driver: SvcState::NotInstalled,
            audit: AuditStatus::default(),
            status: String::new(),
            busy: false,
        };
        match Config::load(Path::new(CONFIG_PATH)) {
            Ok(c) => app.apply_config(&c),
            Err(e) => {
                app.status = if Path::new(CONFIG_PATH).exists() {
                    format!("Configuração inválida ({e}); corrija em Configurações.")
                } else {
                    "Primeiro uso: adicione as pastas em Pastas protegidas e instale em Configurações.".into()
                };
            }
        }
        app.refresh();
        if app.svc == SvcState::NotInstalled {
            app.tab = Tab::Config;
        }
        (app, Task::none())
    }

    fn title(&self) -> String {
        let host = std::env::var("COMPUTERNAME").unwrap_or_default();
        format!("rnetrbin - Lixeira de rede - {host}")
    }

    fn theme(&self) -> Theme {
        theme::iced_theme(self.theme)
    }

    fn subscription(&self) -> Subscription<Msg> {
        iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Msg::Modifiers(m)),
            _ => None,
        })
    }

    fn pal(&self) -> theme::Pal {
        theme::pal(self.theme)
    }

    fn apply_config(&mut self, c: &Config) {
        self.theme = c.theme;
        self.form = settings::Form::from_config(c);
        self.folders = c.folders.clone();
        match open_index(&c.db_path) {
            Ok(i) => self.idx = Some(i),
            Err(e) => self.status = e,
        }
    }

    /// Re-read service state, audit state and the whole index.
    fn refresh(&mut self) {
        self.svc = install::service_state(install::OWN_SERVICE).unwrap_or(SvcState::NotInstalled);
        self.driver = install::service_state(install::DRIVER_SERVICE).unwrap_or(SvcState::NotInstalled);
        self.audit = audit_setup::status();
        if let Some(idx) = &self.idx {
            match idx.all_versions() {
                Ok(v) => self.rows = v,
                Err(e) => self.status = format!("Erro ao ler o índice: {e}"),
            }
        }
        let roots: Vec<String> = self.folders.iter().map(|f| f.path.display().to_string()).collect();
        let files: Vec<(String, i64, i64)> =
            self.rows.iter().map(|r| (r.original_path.clone(), r.size, r.deleted_utc)).collect();
        self.tree = Tree::build(&roots, &files);
        self.relist();
    }

    /// Recompute the list from the loaded rows (no disk access).
    fn relist(&mut self) {
        self.listing = listing::build(
            &self.tree,
            &self.rows,
            &View {
                cur: &self.cur,
                query: &self.query,
                all_copies: self.all_copies,
                copies_of: self.copies_of.as_deref(),
                sort: self.sort,
                limit: MAX_ROWS,
            },
        );
        let visible: BTreeSet<i64> = self.listing.files.iter().map(|f| f.id).collect();
        self.selected.retain(|id| visible.contains(id));
    }

    /// Navigate to a folder and open its branch in the tree.
    fn go(&mut self, path: String) {
        let mut p = path.clone();
        loop {
            self.expanded.insert(tree::key(&p));
            if p.is_empty() || self.tree.is_root(&p) {
                break;
            }
            p = tree::parent(&p).to_string();
        }
        self.expanded.insert(String::new());
        self.cur = path;
        self.query.clear();
        self.copies_of = None;
        self.selected.clear();
        self.anchor = None;
        self.relist();
    }

    fn click(&mut self, id: i64) {
        if self.mods.control() {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(id);
        } else if self.mods.shift() && self.anchor.is_some() {
            let pos = |x: i64| self.listing.files.iter().position(|f| f.id == x);
            if let (Some(a), Some(b)) = (pos(self.anchor.unwrap_or(id)), pos(id)) {
                let (lo, hi) = (a.min(b), a.max(b));
                self.selected = self.listing.files[lo..=hi].iter().map(|f| f.id).collect();
            }
        } else {
            self.selected = BTreeSet::from([id]);
            self.anchor = Some(id);
        }
    }

    fn selected_versions(&self) -> Vec<Version> {
        self.listing.files.iter().filter(|f| self.selected.contains(&f.id)).cloned().collect()
    }

    /// Validate and write the config; restart the service if it is running.
    fn save_config(&mut self) -> Task<Msg> {
        let mut cfg = match self.form.to_config(&self.folders, self.theme) {
            Ok(c) => c,
            Err(e) => {
                self.status = e;
                return Task::none();
            }
        };
        if let Err(e) = cfg.save(Path::new(CONFIG_PATH)) {
            self.status = format!("Não salvou: {e}");
            return Task::none();
        }
        self.apply_config(&cfg);
        if self.svc != SvcState::Running {
            self.status = "Configuração salva.".into();
            self.refresh();
            return Task::none();
        }
        self.busy = true;
        work(|| {
            install::stop_own_service()?;
            install::start_service(install::OWN_SERVICE)?;
            Ok("Configuração salva; serviço reiniciado.".into())
        })
    }

    fn restore(&mut self, dir: Option<PathBuf>) -> Task<Msg> {
        let Some(idx) = self.idx.clone() else { return Task::none() };
        let items = self.selected_versions();
        self.busy = true;
        work(move || {
            let mut errs = Vec::new();
            for v in &items {
                let dest = dir.as_ref().map(|d| d.join(listing::split_path(&v.original_path).0));
                let r = match &dest {
                    // restore() with an override copies over the target; never clobber here.
                    Some(d) if d.exists() => Err(format!("{} já existe", d.display())),
                    _ => crate::restore::restore(&idx, v.id, dest.as_deref()).map(|_| ()).map_err(|e| match e {
                        crate::restore::RestoreError::Exists => {
                            format!("{} já existe no local original; use \"Restaurar em...\"", v.original_path)
                        }
                        e => format!("{}: {e}", v.original_path),
                    }),
                };
                if let Err(e) = r {
                    errs.push(e);
                }
            }
            summary("restaurado(s)", items.len(), errs)
        })
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        if !matches!(msg, Msg::ConfirmRemove | Msg::Modifiers(_)) {
            self.confirm_remove = false;
        }
        match msg {
            Msg::Tab(t) => self.tab = t,
            Msg::Modifiers(m) => self.mods = m,
            Msg::Search(q) => {
                self.query = q;
                self.copies_of = None;
                self.relist();
            }
            Msg::Go(p) => self.go(p),
            Msg::ToggleNode(p) => {
                let k = tree::key(&p);
                if !self.expanded.remove(&k) {
                    self.expanded.insert(k);
                }
            }
            Msg::Click(id) => self.click(id),
            Msg::Sort(col) => {
                self.sort = if self.sort.0 == col {
                    (col, !self.sort.1)
                } else {
                    (col, matches!(col, SortCol::Name | SortCol::User))
                };
                self.relist();
            }
            Msg::AllCopies(on) => {
                self.all_copies = on;
                self.relist();
            }
            Msg::ShowCopies => {
                if let Some(v) = self.selected_versions().first() {
                    self.copies_of = Some(v.original_path.clone());
                    self.selected.clear();
                    self.relist();
                }
            }
            Msg::CloseCopies => {
                self.copies_of = None;
                self.relist();
            }
            Msg::Restore => return self.restore(None),
            Msg::RestoreTo => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Restaurar em").pick_folder() {
                    return self.restore(Some(dir));
                }
            }
            Msg::Purge => {
                let Some(idx) = self.idx.clone() else { return Task::none() };
                let ids: Vec<i64> = self.selected.iter().copied().collect();
                self.busy = true;
                return work(move || {
                    let errs: Vec<String> = ids
                        .iter()
                        .filter_map(|id| crate::retention::purge(&idx, *id).err().map(|e| format!("#{id}: {e}")))
                        .collect();
                    summary("excluído(s) de vez", ids.len(), errs)
                });
            }
            Msg::Refresh => {
                self.refresh();
                self.status = "Lista atualizada.".into();
            }
            Msg::SelectFolder(i) => self.folder_sel = Some(i),
            Msg::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Pasta a proteger").pick_folder() {
                    self.rule_edit = Some(folders::RuleEdit::new(None, &FolderRule::with_defaults(dir)));
                }
            }
            Msg::EditRules => {
                if let Some(i) = self.folder_sel {
                    self.rule_edit = Some(folders::RuleEdit::new(Some(i), &self.folders[i]));
                }
            }
            Msg::RuleInclude(s) => {
                if let Some(r) = &mut self.rule_edit {
                    r.include = s;
                }
            }
            Msg::RuleExclude(s) => {
                if let Some(r) = &mut self.rule_edit {
                    r.exclude = s;
                }
            }
            Msg::RuleCancel => self.rule_edit = None,
            Msg::RuleSave => {
                let Some(edit) = self.rule_edit.take() else { return Task::none() };
                let before = self.folders.clone();
                let rule = edit.to_rule();
                match edit.index {
                    Some(i) => self.folders[i] = rule,
                    None => self.folders.push(rule),
                }
                let task = self.save_config();
                if self.status.starts_with("Não salvou") || self.form.to_config(&self.folders, self.theme).is_err() {
                    // Keep the dialog open with what was typed; undo the list change.
                    self.folders = before;
                    self.rule_edit = Some(edit);
                }
                return task;
            }
            Msg::RemoveFolder => self.confirm_remove = self.folder_sel.is_some(),
            Msg::ConfirmRemove => {
                if let Some(i) = self.folder_sel.take() {
                    self.folders.remove(i);
                    return self.save_config();
                }
            }
            Msg::Field(f, v) => self.form.set(f, v),
            Msg::Theme(t) => self.theme = t,
            Msg::Save => return self.save_config(),
            Msg::Install => {
                let task = self.save_config();
                if !self.status.starts_with("Configuração salva") {
                    return task;
                }
                self.busy = true;
                return work(|| {
                    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                    install::install(&install::WinHost, install::EMBEDDED_DRIVER, &exe).map_err(|e| e.to_string())?;
                    install::start_service(install::OWN_SERVICE)?;
                    Ok(format!("Instalado e iniciado. O serviço usa {}; não mova este arquivo.", exe.display()))
                });
            }
            Msg::Start => {
                self.busy = true;
                return work(|| install::start_service(install::OWN_SERVICE).map(|_| "Serviço iniciado.".into()));
            }
            Msg::Stop => {
                self.busy = true;
                return work(|| install::stop_own_service().map(|_| "Serviço parado.".into()));
            }
            Msg::Exit => return iced::exit(),
            Msg::Done(r) => {
                self.busy = false;
                self.status = match r {
                    Ok(s) => s,
                    Err(e) => format!("Erro: {e}"),
                };
                self.refresh();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        let p = self.pal();
        let tab = |label: &'static str, t: Tab| {
            iced::widget::button(text(label).size(13))
                .padding([5, 14])
                .style(theme::tab_button(p, self.tab == t, t == Tab::Arquivo))
                .on_press(Msg::Tab(t))
        };
        let tabs = container(row![
            tab("Arquivo", Tab::Arquivo),
            tab("Início", Tab::Inicio),
            tab("Pastas protegidas", Tab::Pastas),
            tab("Configurações", Tab::Config),
        ])
        .width(Length::Fill)
        .style(theme::surface(p, p.panel));
        let body = match self.tab {
            Tab::Arquivo => settings::view_file(self),
            Tab::Inicio => home::view(self),
            Tab::Pastas => folders::view(self),
            Tab::Config => settings::view(self),
        };
        container(column![tabs, container(body).height(Length::Fill), self.status_bar()])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::surface(p, p.bg))
            .into()
    }

    fn status_bar(&self) -> Element<'_, Msg> {
        let p = self.pal();
        let state = |label: &'static str, ok: bool, word: &'static str| {
            row![
                icons::icon(if ok { "check_circle" } else { "error" }, 15.0, if ok { p.ok } else { p.danger }),
                text(format!("{label}: {word}")).size(12),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
        };
        let svc_word = match self.svc {
            SvcState::Running => "rodando",
            SvcState::Stopped => "parado",
            SvcState::Busy => "iniciando/parando",
            SvcState::NotInstalled => "não instalado",
        };
        let used = self.tree.info("").size;
        let limit = self.form.max_gb.replace(',', ".").parse::<f64>().unwrap_or(0.0);
        let msg = if self.busy { "Trabalhando..." } else { self.status.as_str() };
        container(
            row![
                state("Serviço", self.svc == SvcState::Running, svc_word),
                state("Driver", self.driver == SvcState::Running, if self.driver == SvcState::Running { "carregado" } else { "não carregado" }),
                state("Auditoria", self.audit.policy_success, if self.audit.policy_success { "ativa" } else { "desligada" }),
                text(format!("Lixeira: {} de {limit} GB", listing::fmt_size(used))).size(12),
                text(format!("{} selecionado(s)", self.selected.len())).size(12),
                Space::new().width(Length::Fill),
                text(msg.to_string()).size(12),
            ]
            .spacing(18)
            .align_y(Alignment::Center),
        )
        .padding([4, 10])
        .width(Length::Fill)
        .style(theme::surface(p, p.panel))
        .into()
    }
}
```

- [ ] **Step 4: Escrever a aba Início (`gui/home.rs`)**

```rust
//! "Início": ribbon, bin tree, breadcrumb and the folder listing.
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space};
use iced::{font, Alignment, Element, Font, Length};

use super::listing::{self, SortCol};
use super::theme::{self, Pal};
use super::tree;
use super::{icons, App, Msg};

const BOLD: Font = Font { weight: font::Weight::Bold, ..Font::DEFAULT };

/// Ribbon button: icon above the label; no message = disabled.
pub(super) fn ribbon_button<'a>(p: Pal, icon: &str, label: &'a str, msg: Option<Msg>, danger: bool) -> Element<'a, Msg> {
    let color = if msg.is_none() { p.muted } else if danger { p.danger } else { p.acc };
    let content = column![icons::icon(icon, 26.0, color), text(label).size(12).align_x(Alignment::Center)]
        .spacing(2)
        .align_x(Alignment::Center)
        .width(Length::Fixed(72.0));
    let b = button(content).padding([4, 4]).style(theme::flat_button(p, false));
    match msg {
        Some(m) => b.on_press(m).into(),
        None => b.into(),
    }
}

/// A labeled group of the ribbon, separated by a vertical line.
pub(super) fn group<'a>(p: Pal, content: Element<'a, Msg>, caption: &'a str) -> Element<'a, Msg> {
    container(column![container(content).height(Length::Fixed(58.0)), text(caption).size(11).color(p.muted)].align_x(Alignment::Center))
        .padding([4, 10])
        .style(move |_| iced::widget::container::Style {
            border: iced::Border { color: p.line, width: 0.0, radius: 0.0.into() },
            ..Default::default()
        })
        .into()
}

fn ribbon(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let has_sel = !app.selected.is_empty() && !app.busy;
    let on = |m: Msg, ok: bool| ok.then_some(m);
    container(
        row![
            group(
                p,
                row![
                    icons::icon("search", 18.0, p.muted),
                    text_input("Buscar nesta pasta e abaixo", &app.query).on_input(Msg::Search).width(Length::Fixed(220.0)),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
                .into(),
                "Buscar na lixeira",
            ),
            group(
                p,
                row![
                    ribbon_button(p, "restore", "Restaurar", on(Msg::Restore, has_sel), false),
                    ribbon_button(p, "drive_file_move", "Restaurar em...", on(Msg::RestoreTo, has_sel), false),
                    ribbon_button(p, "delete_forever", "Excluir", on(Msg::Purge, has_sel), true),
                    ribbon_button(p, "history", "Ver cópias", on(Msg::ShowCopies, has_sel), false),
                ]
                .into(),
                "Ações",
            ),
            group(p, ribbon_button(p, "refresh", "Atualizar", on(Msg::Refresh, !app.busy), false), " "),
            group(p, checkbox(app.all_copies).label("Todas as cópias").on_toggle(Msg::AllCopies).into(), "Mostrar"),
        ]
        .align_y(Alignment::Start),
    )
    .width(Length::Fill)
    .style(theme::surface(p, p.bg))
    .into()
}

fn tree_nodes<'a>(app: &'a App, path: &str, depth: usize, out: &mut Vec<Element<'a, Msg>>) {
    let p = app.pal();
    let kids = app.tree.children(path);
    let open = app.expanded.contains(&tree::key(path));
    let current = tree::key(path) == tree::key(&app.cur);
    let is_root = app.tree.is_root(path);
    let marker = if kids.is_empty() { " " } else if open { "▾" } else { "▸" };
    let icon = if path.is_empty() {
        "delete"
    } else if is_root {
        "folder_special"
    } else if current {
        "folder_open"
    } else {
        "folder"
    };
    let label = if path.is_empty() {
        "Lixeira".to_string()
    } else if is_root {
        path.to_string()
    } else {
        listing::split_path(path).0.to_string()
    };
    let name = text(label).size(13).font(if is_root || path.is_empty() { BOLD } else { Font::DEFAULT });
    let line = row![
        Space::new().width(Length::Fixed(depth as f32 * 14.0)),
        button(text(marker).size(12)).padding([0, 3]).style(theme::flat_button(p, false)).on_press(Msg::ToggleNode(path.to_string())),
        button(
            row![icons::icon(icon, 16.0, p.acc), name, text(format!("({})", app.tree.info(path).count)).size(11).color(p.muted)]
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .padding([1, 4])
        .style(theme::flat_button(p, current))
        .on_press(Msg::Go(path.to_string())),
    ]
    .align_y(Alignment::Center);
    out.push(line.into());
    if open {
        for k in kids {
            tree_nodes(app, &k, depth + 1, out);
        }
    }
}

fn crumbs(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let link = |label: String, to: String| {
        button(text(label).size(13).color(p.acc)).padding([0, 2]).style(theme::flat_button(p, false)).on_press(Msg::Go(to))
    };
    let sep = || text("›").size(13).color(p.muted);
    if let Some(path) = &app.copies_of {
        return row![
            button(text("Voltar").size(13)).style(theme::flat_button(p, false)).on_press(Msg::CloseCopies),
            text(format!("Cópias de {path}")).size(13).font(BOLD),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into();
    }
    if app.cur.is_empty() {
        return row![icons::icon("delete", 16.0, p.acc), text("Lixeira").size(13).font(BOLD)]
            .spacing(4)
            .align_y(Alignment::Center)
            .into();
    }
    let root = app.tree.root_of(&app.cur).unwrap_or_default();
    let up = if tree::key(&app.cur) == tree::key(&root) { String::new() } else { tree::parent(&app.cur).to_string() };
    let mut r = row![
        button(icons::icon("arrow_upward", 16.0, p.acc)).padding([1, 3]).style(theme::flat_button(p, false)).on_press(Msg::Go(up)),
        link("Lixeira".into(), String::new()),
        sep(),
        link(root.clone(), root.clone()),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let mut acc = root.clone();
    for part in app.cur[root.len().min(app.cur.len())..].split('\\').filter(|s| !s.is_empty()) {
        acc = format!("{acc}\\{part}");
        r = r.push(sep()).push(link(part.to_string(), acc.clone()));
    }
    r.into()
}

const W_NAME: Length = Length::FillPortion(5);
const W_COPIES: Length = Length::Fixed(90.0);
const W_USER: Length = Length::FillPortion(3);
const W_SIZE: Length = Length::Fixed(90.0);
const W_TIME: Length = Length::Fixed(130.0);

fn header(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let h = |label: &'static str, col: SortCol, w: Length| {
        let arrow = if app.sort.0 == col { if app.sort.1 { " ▲" } else { " ▼" } } else { "" };
        button(text(format!("{label}{arrow}")).size(12).color(p.muted))
            .padding([3, 6])
            .width(w)
            .style(theme::flat_button(p, false))
            .on_press(Msg::Sort(col))
    };
    row![
        h("Nome", SortCol::Name, W_NAME),
        h("Cópias", SortCol::Copies, W_COPIES),
        h("Usuário", SortCol::User, W_USER),
        h("Tamanho", SortCol::Size, W_SIZE),
        h("Apagado em", SortCol::Deleted, W_TIME),
    ]
    .into()
}

fn rows(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let cell = |s: String, w: Length| text(s).size(13).width(w);
    let mut col = column![].spacing(1);
    for d in &app.listing.dirs {
        let icon = if app.cur.is_empty() { "folder_special" } else { "folder" };
        col = col.push(
            button(
                row![
                    row![icons::icon(icon, 17.0, p.acc), text(d.name.clone()).size(13).font(BOLD)].spacing(6).width(W_NAME),
                    cell(format!("{} apagado(s)", d.info.count), W_COPIES).color(p.muted),
                    cell(String::new(), W_USER),
                    cell(listing::fmt_size(d.info.size), W_SIZE).color(p.muted),
                    cell(if d.info.count > 0 { listing::fmt_time(d.info.last) } else { String::new() }, W_TIME).color(p.muted),
                ]
                .align_y(Alignment::Center),
            )
            .padding([2, 6])
            .style(theme::flat_button(p, false))
            .on_press(Msg::Go(d.path.clone())),
        );
    }
    for f in &app.listing.files {
        let label = if app.query.trim().is_empty() && app.copies_of.is_none() {
            listing::split_path(&f.original_path).0.to_string()
        } else if app.cur.is_empty() {
            f.original_path.clone()
        } else {
            f.original_path.get(app.cur.len() + 1..).unwrap_or(&f.original_path).to_string()
        };
        let user = match &f.user_name {
            Some(u) => cell(u.clone(), W_USER),
            None => cell("desconhecido".into(), W_USER).color(p.muted),
        };
        col = col.push(
            button(
                row![
                    row![icons::icon("description", 17.0, p.acc), text(label).size(13)].spacing(6).width(W_NAME),
                    cell(if f.copies > 1 { f.copies.to_string() } else { "-".into() }, W_COPIES),
                    user,
                    cell(listing::fmt_size(f.size), W_SIZE),
                    cell(listing::fmt_time(f.deleted_utc), W_TIME),
                ]
                .align_y(Alignment::Center),
            )
            .padding([2, 6])
            .style(theme::flat_button(p, app.selected.contains(&f.id)))
            .on_press(Msg::Click(f.id)),
        );
    }
    if app.listing.dirs.is_empty() && app.listing.files.is_empty() {
        col = col.push(text("Nada apagado aqui.").size(13).color(p.muted));
    }
    if app.listing.total_files > app.listing.files.len() {
        col = col.push(
            text(format!("Mostrando {} de {}; refine a busca.", app.listing.files.len(), app.listing.total_files))
                .size(12)
                .color(p.muted),
        );
    }
    scrollable(col).height(Length::Fill).into()
}

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let mut nodes = Vec::new();
    tree_nodes(app, "", 0, &mut nodes);
    let title = |s: &'static str| text(s).size(15).color(p.acc);
    let left = column![title("Lixeira"), scrollable(column(nodes).spacing(1)).height(Length::Fill)]
        .spacing(6)
        .padding(8)
        .width(Length::Fixed(270.0));
    let right = column![title("Arquivos recuperáveis"), crumbs(app), header(app), rows(app)].spacing(6).padding(8);
    column![ribbon(app), row![container(left).style(theme::surface(p, p.bg)), right].height(Length::Fill)].into()
}
```

- [ ] **Step 5: Escrever a aba Pastas protegidas (`gui/folders.rs`)**

```rust
//! "Pastas protegidas": the folder list with its rules and the rule editor.
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use super::home::{group, ribbon_button};
use super::listing::split_masks;
use super::theme;
use super::{App, Msg};
use crate::config::FolderRule;

/// Rule dialog state: `index` None = new folder.
#[derive(Debug, Clone)]
pub struct RuleEdit {
    pub index: Option<usize>,
    pub path: std::path::PathBuf,
    pub include: String,
    pub exclude: String,
}

impl RuleEdit {
    pub fn new(index: Option<usize>, r: &FolderRule) -> RuleEdit {
        RuleEdit { index, path: r.path.clone(), include: r.include.join("; "), exclude: r.exclude.join("; ") }
    }
    pub fn to_rule(&self) -> FolderRule {
        FolderRule { path: self.path.clone(), include: split_masks(&self.include), exclude: split_masks(&self.exclude) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_edit_round_trips_and_splits_masks() {
        let r = FolderRule::with_defaults(r"D:\Dados".into());
        assert_eq!(RuleEdit::new(None, &r).to_rule(), r);
        let e = RuleEdit { index: Some(0), path: r"D:\X".into(), include: "*.xml ; *.pdf".into(), exclude: "".into() };
        assert_eq!(e.to_rule().include, ["*.xml", "*.pdf"]);
        assert!(e.to_rule().exclude.is_empty());
    }
}

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let idle = !app.busy;
    let sel = app.folder_sel.is_some() && idle;
    let mut actions = row![
        ribbon_button(p, "create_new_folder", "Adicionar pasta", idle.then_some(Msg::AddFolder), false),
        ribbon_button(p, "rule", "Editar regras", sel.then_some(Msg::EditRules), false),
        ribbon_button(p, "folder_off", "Remover", sel.then_some(Msg::RemoveFolder), true),
    ];
    if app.confirm_remove {
        actions = actions.push(
            button(text("Confirmar remoção").size(12)).style(iced::widget::button::danger).on_press(Msg::ConfirmRemove),
        );
    }
    let ribbon = container(row![group(p, actions.align_y(Alignment::Center).into(), "Pastas")])
        .width(Length::Fill)
        .style(theme::surface(p, p.bg));

    let w = [Length::FillPortion(4), Length::FillPortion(3), Length::FillPortion(3), Length::Fixed(90.0)];
    let head = row![
        text("Pasta (com subpastas)").size(12).color(p.muted).width(w[0]),
        text("Incluir").size(12).color(p.muted).width(w[1]),
        text("Excluir").size(12).color(p.muted).width(w[2]),
        text("Apagados").size(12).color(p.muted).width(w[3]),
    ]
    .padding([3, 6]);
    let mut list = column![head].spacing(1);
    for (i, f) in app.folders.iter().enumerate() {
        let path = f.path.display().to_string();
        list = list.push(
            button(
                row![
                    row![super::icons::icon("folder_special", 17.0, p.acc), text(path.clone()).size(13)].spacing(6).width(w[0]),
                    text(f.include.join("; ")).size(13).width(w[1]),
                    text(if f.exclude.is_empty() { "nada".into() } else { f.exclude.join("; ") }).size(13).width(w[2]),
                    text(app.tree.info(&path).count.to_string()).size(13).width(w[3]),
                ]
                .align_y(Alignment::Center),
            )
            .padding([3, 6])
            .style(theme::flat_button(p, app.folder_sel == Some(i)))
            .on_press(Msg::SelectFolder(i)),
        );
    }
    if app.folders.is_empty() {
        list = list.push(text("Nenhuma pasta protegida: nada vai para a lixeira.").size(13).color(p.muted));
    }

    let mut body = column![scrollable(list).height(Length::Fill)].spacing(10).padding(10);
    if let Some(e) = &app.rule_edit {
        body = body.push(
            container(
                column![
                    text(format!("Regras de {}", e.path.display())).size(14),
                    row![
                        text("Incluir (separe com ;)").size(13).width(Length::Fixed(190.0)),
                        text_input("*.*", &e.include).on_input(Msg::RuleInclude),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        text("Excluir (separe com ;)").size(13).width(Length::Fixed(190.0)),
                        text_input("*.bak; *.tmp; ~$*", &e.exclude).on_input(Msg::RuleExclude),
                    ]
                    .align_y(Alignment::Center),
                    text(
                        "Primeiro vale o Incluir, depois o Excluir: Incluir *.* e Excluir *.bak guarda tudo menos .bak, \
                         nesta pasta e nas subpastas. Curinga no meio do nome (relat*.xlsx) também pode casar com nomes \
                         de subpasta.",
                    )
                    .size(12)
                    .color(p.muted),
                    row![
                        button(text("Salvar").size(13)).on_press(Msg::RuleSave),
                        button(text("Cancelar").size(13)).style(iced::widget::button::secondary).on_press(Msg::RuleCancel),
                        Space::new().width(Length::Fill),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            )
            .padding(10)
            .style(theme::surface(p, p.panel)),
        );
    }
    column![ribbon, body].into()
}
```

- [ ] **Step 6: Completar `gui/settings.rs`**

Acrescente antes do `#[cfg(test)]`:

```rust
#[derive(Debug, Clone, Copy)]
pub enum Field {
    RecycleDir,
    DbPath,
    MaxGb,
    MaxAgeDays,
}

/// Text fields of the settings tab (folders and theme live in the App).
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub recycle_dir: String,
    pub db_path: String,
    pub max_gb: String,
    pub max_age_days: String,
}

impl Form {
    pub fn from_config(c: &Config) -> Form {
        Form {
            recycle_dir: c.recycle_dir.display().to_string(),
            db_path: c.db_path.display().to_string(),
            max_gb: format!("{}", (c.max_bin_bytes as f64 / GB * 100.0).round() / 100.0),
            max_age_days: c.max_age_days.map(|d| d.to_string()).unwrap_or_default(),
        }
    }

    pub fn set(&mut self, f: Field, v: String) {
        *match f {
            Field::RecycleDir => &mut self.recycle_dir,
            Field::DbPath => &mut self.db_path,
            Field::MaxGb => &mut self.max_gb,
            Field::MaxAgeDays => &mut self.max_age_days,
        } = v;
    }

    /// Parse numbers here; paths, folders and limits are validated by `Config::save`.
    pub fn to_config(&self, folders: &[FolderRule], theme: ThemeName) -> Result<Config, String> {
        let gb: f64 = self.max_gb.trim().replace(',', ".").parse().map_err(|_| "Limite da lixeira inválido".to_string())?;
        if !gb.is_finite() || gb <= 0.0 {
            return Err("Limite da lixeira deve ser maior que zero".into());
        }
        let age = self.max_age_days.trim();
        let max_age_days = if age.is_empty() {
            None
        } else {
            Some(age.parse().map_err(|_| "Dias de retenção inválido".to_string())?)
        };
        Ok(Config {
            recycle_dir: PathBuf::from(self.recycle_dir.trim()),
            db_path: PathBuf::from(self.db_path.trim()),
            max_bin_bytes: (gb * GB) as u64,
            max_age_days,
            theme,
            folders: folders.to_vec(),
        })
    }
}

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let idle = !app.busy;
    let rb = super::home::ribbon_button;
    let ribbon = container(row![
        super::home::group(p, rb(p, "save", "Salvar", idle.then_some(Msg::Save), false), " "),
        super::home::group(
            p,
            row![
                rb(p, "install_desktop", "Instalar", (idle && app.svc == SvcState::NotInstalled).then_some(Msg::Install), false),
                rb(p, "play_circle", "Iniciar", (idle && app.svc == SvcState::Stopped).then_some(Msg::Start), false),
                rb(p, "stop_circle", "Parar", (idle && app.svc == SvcState::Running).then_some(Msg::Stop), true),
            ]
            .into(),
            "Serviço",
        ),
    ])
    .width(Length::Fill)
    .style(theme::surface(p, p.bg));

    let field = |label: &'static str, hint: &'static str, value: &str, f: Field| {
        row![text(label).size(13).width(Length::Fixed(230.0)), text_input(hint, value).on_input(move |v| Msg::Field(f, v))]
            .align_y(Alignment::Center)
    };
    let theme_btn = |label: &'static str, t: ThemeName| {
        button(text(label).size(13))
            .style(if app.theme == t { iced::widget::button::primary } else { iced::widget::button::secondary })
            .on_press(Msg::Theme(t))
    };
    let audit = if app.audit.policy_success {
        "ativa: exclusões de Usuários do Domínio, log de Segurança com 512 MB ou mais"
    } else if app.audit.configured {
        "desligada (uma política de grupo pode ter sobrescrito); a coluna Usuário mostrará \"desconhecido\""
    } else {
        "ainda não configurada: o serviço configura ao iniciar"
    };
    let exe = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_default();
    column![
        ribbon,
        column![
            field("Pasta da lixeira", r"E:\Lixeira", &app.form.recycle_dir, Field::RecycleDir),
            field("Arquivo do índice", r"C:\ProgramData\rnetrbin\index.db", &app.form.db_path, Field::DbPath),
            field("Limite da lixeira (GB)", "10", &app.form.max_gb, Field::MaxGb),
            field("Apagar backups após (dias)", "vazio = sem limite", &app.form.max_age_days, Field::MaxAgeDays),
            row![
                text("Tema").size(13).width(Length::Fixed(230.0)),
                theme_btn("Rust", ThemeName::Rust),
                theme_btn("Branco", ThemeName::Branco),
                theme_btn("Preto", ThemeName::Preto),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![text("Auditoria (quem apagou)").size(13).width(Length::Fixed(230.0)), text(audit).size(13)],
            text(format!(
                "Configuração gravada em {CONFIG_PATH}. Salvar com o serviço rodando o reinicia. \
                 Instalar registra o serviço apontando para {exe}; não mova este arquivo depois."
            ))
            .size(12)
            .color(p.muted),
            Space::new().height(Length::Fill),
        ]
        .spacing(10)
        .padding(12),
    ]
    .into()
}

pub(super) fn view_file(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    column![
        row![super::icons::icon("info", 22.0, p.acc), text(format!("rnetrbin {}", env!("CARGO_PKG_VERSION"))).size(16)]
            .spacing(8)
            .align_y(Alignment::Center),
        text("Lixeira de rede para compartilhamentos SMB, sobre o driver netrbin.sys.").size(13),
        text("Ícones: Material Icons, Google (Apache 2.0).").size(12).color(p.muted),
        button(row![super::icons::icon("logout", 18.0, p.danger), text("Sair").size(13)].spacing(6).align_y(Alignment::Center))
            .style(theme::flat_button(p, false))
            .on_press(Msg::Exit),
    ]
    .spacing(12)
    .padding(16)
    .into()
}
```

- [ ] **Step 7: Rodar os testes e compilar**

Run: `cargo test` e `cargo clippy --all-targets`
Expected: todos passam; nenhum warning (remova o `#![allow(dead_code)]` se foi adicionado na Task 7). Erros de API do iced: ajuste à assinatura do 0.14 que o compilador indicar, sem mudar o comportamento.

- [ ] **Step 8: Conferir a janela (manual)**

Run: `cargo build --release` e abra `target\release\rnetrbin-svc.exe` (aceite o UAC).
Expected, conferindo contra o mockup `layout-undelete-v2.html`:
- **Sem config:** abre em Configurações, tema Rust, barra de status com "Serviço: não instalado"; Instalar fica desabilitado até haver pasta em Pastas protegidas.
- **Pastas protegidas:** Adicionar pasta abre o seletor do Windows e o editor com `*.*` e `*.tmp; ~$*`; Salvar grava o `config.toml`.
- **Início:** árvore com "Lixeira" e a pasta protegida; clicar numa subpasta na lista entra nela e marca a árvore; "Acima" sobe; a busca mostra arquivos de subpastas com caminho relativo.
- **Temas:** Branco e Preto trocam as cores na hora.
- **Ícones:** todos aparecem (nenhum quadrado vazio).

Feche a janela.

- [ ] **Step 9: Commit**

```bash
git add -A crates/svc/src
git commit -m "feat(gui): Undelete-style window with tree navigation, rules editor and themes"
```

---

### Task 9: Documentação e verificação final

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/notes/e2e-checklist.md`
- Modify (fora do repo): `C:\Users\joao.silva\Documents\wiki\pessoal-wiki\projetos\conhecimento\rnetrbin - driver netrbin e instalação standalone.md`

- [ ] **Step 1: Atualizar o AGENTS.md**

Na seção "Como rodar", troque a linha da janela por:

```markdown
- janela: abrir o exe sem argumentos (pede UAC). Abas Arquivo, Início (árvore da
  lixeira e lista por pasta), Pastas protegidas (regras Incluir/Excluir) e
  Configurações (instalar/iniciar/parar, tema). O SCM roda o exe com `--service`
```

Na seção "Convenções que não se deduzem do código", acrescente:

```markdown
- Quem apagou vem da auditoria do Windows (SACL global só DELETE para Usuários do
  Domínio + evento 4663), configurada pelo serviço ao iniciar e desfeita no
  `uninstall` conforme `C:\ProgramData\rnetrbin\audit-state.toml`. Sem auditoria a
  captura continua e o usuário fica "desconhecido".
- Pastas protegidas não podem se aninhar: no driver as máscaras de todas as pastas
  formam uma lista só.
```

- [ ] **Step 2: Acrescentar casos ao checklist e2e**

No fim da seção "Cases" de `docs/notes/e2e-checklist.md`:

```markdown
16. Rules: folder with Include `*.*` and Exclude `*.bak`; deleting `a.docx` and `a.bak`
    over SMB keeps only `a.docx`, also in a subfolder.
17. Deleted-by: the list shows `DOMAIN\user` of the SMB user within ~10 s of the delete;
    with File System auditing turned off by `auditpol`, the backup still appears and the
    user shows "desconhecido".
18. Large folder: protecting a folder with ~1M files is immediate (no per-file ACL
    rewrite; `auditpol /resourceSACL /type:File /view` shows the DELETE entry).
19. `uninstall` restores the previous File System audit setting and removes the global
    SACL entry; the Security log size stays.
```

- [ ] **Step 3: Atualizar a wiki**

Na página da wiki, substitua a seção "Janela (substitui o snetrbin.exe original)" por:

```markdown
## Janela (substitui o snetrbin.exe original)

Modelo do Undelete Server: faixa de abas (Arquivo, Início, Pastas protegidas,
Configurações), árvore da lixeira à esquerda e lista à direita que navega como o
Explorer (subpastas primeiro; clicar entra). Ícones Material Icons embutidos no exe
(Apache 2.0); temas Rust (padrão), Branco e Preto.

Cada pasta protegida tem Incluir e Excluir (Incluir vale primeiro). Pastas não podem
se aninhar porque o driver soma as máscaras de todas numa lista só.

Quem apagou: auditoria global do Windows (`auditpol /resourceSACL`, só DELETE, só
Usuários do Domínio) + assinatura do evento 4663. Não reescreve ACL de arquivo nenhum
(propagar SACL em 2 milhões de arquivos levaria horas). Medido: ~45 µs e ~1,8 KB de
log por operação; o serviço aumenta o log de Segurança para 512 MB.
```

Depois: `cd C:\Users\joao.silva\Documents\wiki\pessoal-wiki; python gerar_indices.py; git add -A projetos; git commit -m "wiki(projetos): rnetrbin com regras por pasta, auditoria e janela Undelete"`.

- [ ] **Step 4: Verificação final**

Run: `cargo test`, `cargo clippy --all-targets` e `cargo build --release`
Expected: tudo verde; `target\release\rnetrbin-svc.exe` gerado com o driver e a fonte embutidos.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md docs/notes/e2e-checklist.md
git commit -m "docs: rules, deleted-by auditing and new window"
```
