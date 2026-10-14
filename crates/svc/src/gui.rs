//! Desktop window (iced): browse/restore/purge the bin, edit the config and
//! install/start/stop the service. Runs on the server itself, elevated.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space};
use iced::{Element, Length, Task};

use crate::config::{Config, FolderRule, ThemeName, CONFIG_PATH};
use crate::index::{Entry, Index};
use crate::install::{self, SvcState};

/// Rows rendered at once; refine the search to see older matches.
const MAX_ROWS: usize = 1000;
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("rnetrbin - Lixeira de rede")
        .window_size((1100.0, 680.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Bin,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortCol {
    Name,
    Folder,
    Size,
    Deleted,
}

#[derive(Debug, Clone, Copy)]
enum Field {
    RecycleDir,
    DbPath,
    Excludes,
    MaxGb,
    MaxAgeDays,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    pub recycle_dir: String,
    pub db_path: String,
    pub folders: Vec<String>,
    pub excludes: String,
    pub max_gb: String,
    pub max_age_days: String,
}

impl Form {
    pub fn from_config(c: &Config) -> Form {
        Form {
            recycle_dir: c.recycle_dir.display().to_string(),
            db_path: c.db_path.display().to_string(),
            folders: c.folders.iter().map(|f| f.path.display().to_string()).collect(),
            excludes: c.folders.first().map(|f| f.exclude.join("; ")).unwrap_or_else(|| "*.tmp; ~$*".into()),
            max_gb: format!("{}", (c.max_bin_bytes as f64 / GB * 100.0).round() / 100.0),
            max_age_days: c.max_age_days.map(|d| d.to_string()).unwrap_or_default(),
        }
    }

    /// Parse the form; validation of paths and limits is left to `Config::save`.
    pub fn to_config(&self) -> Result<Config, String> {
        let gb: f64 = self.max_gb.trim().replace(',', ".").parse().map_err(|_| "Tamanho máximo inválido".to_string())?;
        if !gb.is_finite() || gb <= 0.0 {
            return Err("Tamanho máximo deve ser maior que zero".into());
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
            folders: self
                .folders
                .iter()
                .map(|p| FolderRule { path: PathBuf::from(p), include: vec!["*.*".into()], exclude: split_masks(&self.excludes) })
                .collect(),
            theme: ThemeName::Rust,
            max_bin_bytes: (gb * GB) as u64,
            max_age_days,
        })
    }
}

/// Masks are separated by `;` (names may contain spaces and commas).
pub fn split_masks(s: &str) -> Vec<String> {
    s.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect()
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

fn fmt_time(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%d/%m/%Y %H:%M").to_string(),
        chrono::LocalResult::None => unix.to_string(),
    }
}

pub fn sort_entries(v: &mut [Entry], col: SortCol, asc: bool) {
    v.sort_by(|a, b| {
        let o = match col {
            SortCol::Name => split_path(&a.original_path).0.to_lowercase().cmp(&split_path(&b.original_path).0.to_lowercase()),
            SortCol::Folder => split_path(&a.original_path).1.to_lowercase().cmp(&split_path(&b.original_path).1.to_lowercase()),
            SortCol::Size => a.size.cmp(&b.size),
            SortCol::Deleted => a.deleted_utc.cmp(&b.deleted_utc),
        };
        if asc { o } else { o.reverse() }
    });
}

fn svc_label(s: SvcState) -> &'static str {
    match s {
        SvcState::NotInstalled => "não instalado",
        SvcState::Stopped => "parado",
        SvcState::Running => "rodando",
        SvcState::Busy => "iniciando/parando",
    }
}

#[derive(Debug, Clone)]
enum Msg {
    Tab(Tab),
    Search(String),
    Refresh,
    Toggle(i64, bool),
    ToggleAll(bool),
    Sort(SortCol),
    Restore,
    RestoreTo,
    Purge,
    EmptyBin,
    Field(Field, String),
    AddFolder,
    RemoveFolder(usize),
    Save,
    Install,
    Start,
    Stop,
    Done(Result<String, String>),
}

struct App {
    tab: Tab,
    form: Form,
    idx: Option<Arc<Index>>,
    entries: Vec<Entry>,
    total: i64,
    search: String,
    selected: BTreeSet<i64>,
    sort: (SortCol, bool),
    svc: SvcState,
    driver: SvcState,
    status: String,
    busy: bool,
    confirm_empty: bool,
}

/// Run blocking work off the UI thread and report its outcome.
fn work(f: impl FnOnce() -> Result<String, String> + Send + 'static) -> Task<Msg> {
    Task::perform(async move { f() }, Msg::Done)
}

impl App {
    fn new() -> (App, Task<Msg>) {
        let mut app = App {
            tab: Tab::Bin,
            form: Form::from_config(&Config::default()),
            idx: None,
            entries: Vec::new(),
            total: 0,
            search: String::new(),
            selected: BTreeSet::new(),
            sort: (SortCol::Deleted, false),
            svc: SvcState::NotInstalled,
            driver: SvcState::NotInstalled,
            status: String::new(),
            busy: false,
            confirm_empty: false,
        };
        match Config::load(Path::new(CONFIG_PATH)) {
            Ok(c) => {
                app.form = Form::from_config(&c);
                app.open_index(&c);
            }
            Err(e) => {
                app.tab = Tab::Settings;
                app.status = if Path::new(CONFIG_PATH).exists() {
                    format!("Configuração inválida ({e}); corrija e salve.")
                } else {
                    "Primeiro uso: confira as configurações e clique em Instalar.".into()
                };
            }
        }
        app.refresh();
        (app, Task::none())
    }

    fn open_index(&mut self, c: &Config) {
        self.idx = None;
        if let Some(parent) = c.db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match Index::open(&c.db_path) {
            Ok(i) => self.idx = Some(Arc::new(i)),
            Err(e) => self.status = format!("Não abriu o índice {}: {e}", c.db_path.display()),
        }
    }

    fn refresh(&mut self) {
        self.svc = install::service_state(install::OWN_SERVICE).unwrap_or(SvcState::NotInstalled);
        self.driver = install::service_state(install::DRIVER_SERVICE).unwrap_or(SvcState::NotInstalled);
        let Some(idx) = &self.idx else { return };
        match idx.search_unrestored(self.search.trim(), MAX_ROWS) {
            Ok((mut v, total)) => {
                sort_entries(&mut v, self.sort.0, self.sort.1);
                self.selected.retain(|id| v.iter().any(|e| e.id == *id));
                self.entries = v;
                self.total = total;
            }
            Err(e) => self.status = format!("Erro ao ler o índice: {e}"),
        }
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        if !matches!(msg, Msg::EmptyBin) {
            self.confirm_empty = false;
        }
        match msg {
            Msg::Tab(t) => self.tab = t,
            Msg::Search(s) => {
                self.search = s;
                self.refresh();
            }
            Msg::Refresh => self.refresh(),
            Msg::Toggle(id, on) => {
                if on {
                    self.selected.insert(id);
                } else {
                    self.selected.remove(&id);
                }
            }
            Msg::ToggleAll(on) => {
                self.selected = if on { self.entries.iter().map(|e| e.id).collect() } else { BTreeSet::new() };
            }
            Msg::Sort(col) => {
                self.sort = if self.sort.0 == col { (col, !self.sort.1) } else { (col, col != SortCol::Deleted) };
                sort_entries(&mut self.entries, self.sort.0, self.sort.1);
            }
            Msg::Restore => return self.restore(None),
            Msg::RestoreTo => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Restaurar em").pick_folder() {
                    return self.restore(Some(dir));
                }
            }
            Msg::Purge => {
                let (Some(idx), ids) = (self.idx.clone(), self.selected.clone()) else { return Task::none() };
                self.busy = true;
                return work(move || {
                    let mut errs = Vec::new();
                    for id in &ids {
                        if let Err(e) = crate::retention::purge(&idx, *id) {
                            errs.push(format!("#{id}: {e}"));
                        }
                    }
                    summary("excluído(s)", ids.len(), errs)
                });
            }
            Msg::EmptyBin => {
                if !self.confirm_empty {
                    self.confirm_empty = true;
                    self.status = "Clique de novo em \"Confirmar esvaziar\" para apagar TODOS os backups.".into();
                    return Task::none();
                }
                self.confirm_empty = false;
                let Some(idx) = self.idx.clone() else { return Task::none() };
                self.busy = true;
                return work(move || {
                    let (purged, skipped) = crate::retention::purge_all(&idx).map_err(|e| e.to_string())?;
                    Ok(if skipped == 0 {
                        format!("Lixeira esvaziada: {purged} backup(s) apagado(s).")
                    } else {
                        format!("{purged} apagado(s); {skipped} em uso, tente de novo depois.")
                    })
                });
            }
            Msg::Field(f, v) => {
                let slot = match f {
                    Field::RecycleDir => &mut self.form.recycle_dir,
                    Field::DbPath => &mut self.form.db_path,
                    Field::Excludes => &mut self.form.excludes,
                    Field::MaxGb => &mut self.form.max_gb,
                    Field::MaxAgeDays => &mut self.form.max_age_days,
                };
                *slot = v;
            }
            Msg::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Pasta a proteger").pick_folder() {
                    let d = dir.display().to_string();
                    if !self.form.folders.iter().any(|f| f.eq_ignore_ascii_case(&d)) {
                        self.form.folders.push(d);
                    }
                }
            }
            Msg::RemoveFolder(i) => {
                if i < self.form.folders.len() {
                    self.form.folders.remove(i);
                }
            }
            Msg::Save => {
                let mut cfg = match self.form.to_config() {
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
                self.form = Form::from_config(&cfg);
                self.open_index(&cfg);
                if self.svc != SvcState::Running {
                    self.status = "Configuração salva.".into();
                    self.refresh();
                    return Task::none();
                }
                self.busy = true;
                return work(|| {
                    install::stop_own_service()?;
                    install::start_service(install::OWN_SERVICE)?;
                    Ok("Configuração salva; serviço reiniciado.".into())
                });
            }
            Msg::Install => {
                let mut cfg = match self.form.to_config() {
                    Ok(c) => c,
                    Err(e) => {
                        self.status = e;
                        return Task::none();
                    }
                };
                if let Err(e) = cfg.save(Path::new(CONFIG_PATH)) {
                    self.status = format!("Não salvou a configuração: {e}");
                    return Task::none();
                }
                self.form = Form::from_config(&cfg);
                self.open_index(&cfg);
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

    /// Restore the selection to its original paths, or into `dir` keeping the file names.
    fn restore(&mut self, dir: Option<PathBuf>) -> Task<Msg> {
        let Some(idx) = self.idx.clone() else { return Task::none() };
        let items: Vec<(i64, String)> = self
            .entries
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .map(|e| (e.id, e.original_path.clone()))
            .collect();
        self.busy = true;
        work(move || {
            let mut errs = Vec::new();
            for (id, orig) in &items {
                let dest = dir.as_ref().map(|d| d.join(split_path(orig).0));
                let r = match &dest {
                    // Restore with an override copies over the target; never clobber here.
                    Some(d) if d.exists() => Err(format!("{} já existe", d.display())),
                    _ => crate::restore::restore(&idx, *id, dest.as_deref()).map(|_| ()).map_err(|e| match e {
                        crate::restore::RestoreError::Exists => {
                            format!("{orig} já existe no local original; use \"Restaurar em...\"")
                        }
                        e => format!("{orig}: {e}"),
                    }),
                };
                if let Err(e) = r {
                    errs.push(e);
                }
            }
            summary("restaurado(s)", items.len(), errs)
        })
    }

    fn view(&self) -> Element<'_, Msg> {
        let tab_btn = |label, t| {
            let b = button(text(label));
            if self.tab == t { b } else { b.style(button::secondary).on_press(Msg::Tab(t)) }
        };
        let header = row![
            tab_btn("Lixeira", Tab::Bin),
            tab_btn("Configurações", Tab::Settings),
            Space::new().width(Length::Fill),
            text(format!("Serviço: {}   Driver: {}", svc_label(self.svc), svc_label(self.driver))),
            button(text("Atualizar")).style(button::secondary).on_press(Msg::Refresh),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let body = match self.tab {
            Tab::Bin => self.view_bin(),
            Tab::Settings => self.view_settings(),
        };
        let status = text(if self.busy { "Trabalhando..." } else { &self.status }).size(13);
        container(column![header, body, status].spacing(10)).padding(12).into()
    }

    fn view_bin(&self) -> Element<'_, Msg> {
        if self.idx.is_none() {
            return text("Sem índice: configure e instale na aba Configurações.").into();
        }
        let arrow = |c: SortCol| if self.sort.0 == c { if self.sort.1 { " ▲" } else { " ▼" } } else { "" };
        let head = |label: &str, c: SortCol| {
            button(text(format!("{label}{}", arrow(c))).size(13)).style(button::text).padding(2).on_press(Msg::Sort(c))
        };
        let all = !self.entries.is_empty() && self.selected.len() == self.entries.len();
        let header = row![
            checkbox(all).on_toggle(Msg::ToggleAll),
            head("Nome", SortCol::Name).width(Length::FillPortion(3)),
            head("Pasta original", SortCol::Folder).width(Length::FillPortion(5)),
            head("Tamanho", SortCol::Size).width(Length::Fixed(90.0)),
            head("Apagado em", SortCol::Deleted).width(Length::Fixed(130.0)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let rows = column(self.entries.iter().map(|e| {
            let (name, folder) = split_path(&e.original_path);
            let id = e.id;
            row![
                checkbox(self.selected.contains(&id)).on_toggle(move |on| Msg::Toggle(id, on)),
                text(name.to_string()).size(13).width(Length::FillPortion(3)),
                text(folder.to_string()).size(13).width(Length::FillPortion(5)),
                text(fmt_size(e.size)).size(13).width(Length::Fixed(90.0)),
                text(fmt_time(e.deleted_utc)).size(13).width(Length::Fixed(130.0)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
        }))
        .spacing(2);

        let sel = !self.selected.is_empty() && !self.busy;
        let act = |label, m: Msg, on: bool| {
            let b = button(text(label));
            if on { b.on_press(m) } else { b }
        };
        let shown = if (self.total as usize) > self.entries.len() {
            format!("{} itens (mostrando os {} mais recentes; refine a busca)", self.total, self.entries.len())
        } else {
            format!("{} itens", self.total)
        };
        let footer = row![
            text(format!("{shown} · {} selecionado(s)", self.selected.len())).size(13),
            Space::new().width(Length::Fill),
            act("Restaurar", Msg::Restore, sel),
            act("Restaurar em...", Msg::RestoreTo, sel),
            act("Excluir", Msg::Purge, sel).style(button::danger),
            act(if self.confirm_empty { "Confirmar esvaziar" } else { "Esvaziar lixeira" }, Msg::EmptyBin, !self.busy && self.total > 0)
                .style(button::danger),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        column![
            text_input("Buscar por nome ou pasta...", &self.search).on_input(Msg::Search),
            header,
            scrollable(rows).height(Length::Fill),
            footer,
        ]
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn view_folders(&self) -> Element<'_, Msg> {
        let list = column(self.form.folders.iter().enumerate().map(|(i, f)| {
            row![
                text(f.clone()).width(Length::Fill),
                button(text("Remover").size(13)).style(button::secondary).padding([2, 8]).on_press(Msg::RemoveFolder(i)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
        }))
        .spacing(4);
        let empty = self.form.folders.is_empty();
        row![
            text("Pastas protegidas\n(com subpastas)").width(Length::Fixed(220.0)),
            column![
                if empty { text("Nenhuma pasta: nada vai para a lixeira.").size(13) } else { text("").size(1) },
                list,
                button(text("Adicionar pasta...")).on_press(Msg::AddFolder),
            ]
            .spacing(6)
            .width(Length::Fill),
        ]
        .spacing(8)
        .into()
    }

    fn view_settings(&self) -> Element<'_, Msg> {
        let field = |label: &str, hint: &str, value: &str, f: Field| {
            row![
                text(label.to_string()).width(Length::Fixed(220.0)),
                text_input(hint, value).on_input(move |v| Msg::Field(f, v)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        };
        let idle = !self.busy;
        let on = |b: iced::widget::Button<'static, Msg>, m: Msg, enabled: bool| if enabled { b.on_press(m) } else { b };
        let actions = match self.svc {
            SvcState::NotInstalled => row![on(button(text("Instalar driver e serviço")), Msg::Install, idle)],
            _ => row![
                on(button(text("Salvar")), Msg::Save, idle),
                on(button(text("Iniciar serviço")).style(button::secondary), Msg::Start, idle && self.svc == SvcState::Stopped),
                on(button(text("Parar serviço")).style(button::secondary), Msg::Stop, idle && self.svc == SvcState::Running),
            ],
        }
        .spacing(8);
        column![
            field("Pasta da lixeira", r"D:\RecycleBinNet", &self.form.recycle_dir, Field::RecycleDir),
            field("Arquivo do índice", r"C:\ProgramData\rnetrbin\index.db", &self.form.db_path, Field::DbPath),
            self.view_folders(),
            field("Nunca guardar (nomes, separe com ;)", "*.tmp; ~$*", &self.form.excludes, Field::Excludes),
            field("Tamanho máximo da lixeira (GB)", "10", &self.form.max_gb, Field::MaxGb),
            field("Apagar backups após (dias)", "vazio = sem limite", &self.form.max_age_days, Field::MaxAgeDays),
            text(format!("Configuração gravada em {CONFIG_PATH}. Ao salvar com o serviço rodando, ele é reiniciado.")).size(13),
            actions,
        ]
        .spacing(10)
        .into()
    }
}

fn summary(verb: &str, n: usize, errs: Vec<String>) -> Result<String, String> {
    if errs.is_empty() {
        Ok(format!("{n} {verb}."))
    } else {
        Err(format!("{} de {n} falharam: {}", errs.len(), errs.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: i64, p: &str, size: i64, t: i64) -> Entry {
        Entry { id, original_path: p.into(), size, deleted_utc: t }
    }

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

    #[test]
    fn split_path_and_sizes() {
        assert_eq!(split_path(r"D:\a\b\c.txt"), ("c.txt", r"D:\a\b"));
        assert_eq!(split_path("c.txt"), ("c.txt", ""));
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn sorts_by_each_column() {
        let mut v = vec![e(1, r"D:\z\b.txt", 30, 2), e(2, r"D:\a\C.txt", 10, 3), e(3, r"D:\m\a.txt", 20, 1)];
        let ids = |v: &[Entry]| v.iter().map(|e| e.id).collect::<Vec<_>>();
        sort_entries(&mut v, SortCol::Name, true);
        assert_eq!(ids(&v), [3, 1, 2]);
        sort_entries(&mut v, SortCol::Folder, true);
        assert_eq!(ids(&v), [2, 3, 1]);
        sort_entries(&mut v, SortCol::Size, false);
        assert_eq!(ids(&v), [1, 3, 2]);
        sort_entries(&mut v, SortCol::Deleted, false);
        assert_eq!(ids(&v), [2, 1, 3]);
    }
}
