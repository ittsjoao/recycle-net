//! Desktop window (iced), Undelete style: ribbon tabs, bin tree, folder listing.
mod folders;
mod messages;
mod home;
mod settings;
pub mod icons;
pub mod listing;
pub mod theme;
pub mod tree;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::widget::{button, column, container, row, stack, text, Space};
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
        // Closing waits for a running job (e.g. the stop+start of Salvar) so the
        // service is never left stopped; see Msg::CloseRequested.
        .exit_on_close_request(false)
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
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
    /// "Excluir": only arms the confirmation.
    Purge,
    ConfirmPurge,
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
    /// "Iniciar o serviço com o Windows" toggled.
    Autostart(bool),
    Done(Result<String, String>),
    /// Hide toast `id` (its timer ran out, or the user closed it).
    ToastClose(u64),
    /// The user closed the window. Only the window exits; the service keeps running.
    CloseRequested,
}

pub struct App {
    tab: Tab,
    theme: ThemeName,
    form: settings::Form,
    folders: Vec<FolderRule>,
    folder_sel: Option<usize>,
    rule_edit: Option<folders::RuleEdit>,
    confirm_remove: bool,
    /// "Excluir" was pressed; "Confirmar exclusão" purges the selection.
    confirm_purge: bool,
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
    /// Service start type is Automatic; None when not installed or unreadable.
    autostart: Option<bool>,
    audit: AuditStatus,
    toast: Option<Toast>,
    toast_seq: u64,
    busy: bool,
    /// Close was requested while busy: exit once the job reports back.
    closing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ok,
    Info,
    Error,
}

/// Outcome message shown bottom-right over the content.
struct Toast {
    id: u64,
    text: String,
    kind: Kind,
}

/// Hide toast `id` after a few seconds (errors and hints stay longer). The
/// wait happens on its own thread so no executor thread is held.
fn expire(id: u64, kind: Kind) -> Task<Msg> {
    let secs = if kind == Kind::Ok { 4 } else { 8 };
    Task::future(async move {
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            let _ = tx.send(());
        });
        let _ = rx.await;
        Msg::ToastClose(id)
    })
}

/// Run blocking work off the UI thread and report its outcome.
fn work(f: impl FnOnce() -> Result<String, String> + Send + 'static) -> Task<Msg> {
    Task::perform(async move { f() }, Msg::Done)
}

fn open_index(db: &Path) -> Result<Arc<Index>, String> {
    if let Some(parent) = db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Index::open(db).map(Arc::new).map_err(|_| {
        format!("Não foi possível abrir o índice em {}; confira o caminho em Configurações.", db.display())
    })
}

/// `done` like "arquivo(s) restaurado(s)"; each error is already a sentence.
fn summary(done: &str, n: usize, errs: Vec<String>) -> Result<String, String> {
    match errs.len() {
        0 => Ok(format!("{n} {done}.")),
        1 if n == 1 => Err(errs.join("")),
        k => Err(format!("{k} de {n} não deram certo. {}", errs.join(" "))),
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
            confirm_purge: false,
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
            autostart: None,
            audit: AuditStatus::default(),
            toast: None,
            toast_seq: 0,
            busy: false,
            closing: false,
        };
        match Config::load(Path::new(CONFIG_PATH)) {
            Ok(c) => app.apply_config(&c),
            Err(e) if Path::new(CONFIG_PATH).exists() => {
                app.notify(format!("{} Corrija em Configurações.", messages::config(&e)), Kind::Error)
            }
            Err(_) => app.notify(
                "Primeiro uso: adicione as pastas em Pastas protegidas e instale em Configurações.",
                Kind::Info,
            ),
        }
        app.refresh();
        if app.svc == SvcState::NotInstalled {
            app.tab = Tab::Config;
        }
        let task = app.toast.as_ref().map_or_else(Task::none, |t| expire(t.id, t.kind));
        (app, task)
    }

    /// Show a toast; `update` schedules its expiry.
    fn notify(&mut self, text: impl Into<String>, kind: Kind) {
        self.toast_seq += 1;
        self.toast = Some(Toast { id: self.toast_seq, text: text.into(), kind });
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
            iced::Event::Window(iced::window::Event::CloseRequested) => Some(Msg::CloseRequested),
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
            Err(e) => self.notify(e, Kind::Error),
        }
    }

    /// Re-read service state, audit state and the whole index.
    fn refresh(&mut self) {
        self.svc = install::service_state(install::OWN_SERVICE).unwrap_or(SvcState::NotInstalled);
        self.driver = install::service_state(install::DRIVER_SERVICE).unwrap_or(SvcState::NotInstalled);
        self.autostart = match self.svc {
            SvcState::NotInstalled => None,
            _ => install::service_autostart(install::OWN_SERVICE).ok(),
        };
        self.audit = audit_setup::status();
        if let Some(idx) = &self.idx {
            match idx.all_versions() {
                Ok(v) => self.rows = v,
                Err(_) => self.notify("Não foi possível ler o índice; tente Atualizar.", Kind::Error),
            }
        }
        let roots: Vec<String> = self.folders.iter().map(|f| f.path.display().to_string()).collect();
        let files: Vec<(String, i64, i64)> =
            self.rows.iter().map(|r| (r.original_path.clone(), r.size, r.deleted_utc)).collect();
        self.tree = Tree::build(&roots, &files);
        // The folder may be gone (orphan root emptied, or no longer protected).
        if !self.cur.is_empty() && self.tree.root_of(&self.cur).is_none() {
            self.cur.clear();
        }
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
        } else if let (true, Some(Some(a)), Some(b)) = (self.mods.shift(), self.anchor.map(|x| self.pos(x)), self.pos(id)) {
            let (lo, hi) = (a.min(b), a.max(b));
            self.selected = self.listing.files[lo..=hi].iter().map(|f| f.id).collect();
        } else {
            self.selected = BTreeSet::from([id]);
            self.anchor = Some(id);
        }
    }

    /// Row of a file in the current listing.
    fn pos(&self, id: i64) -> Option<usize> {
        self.listing.files.iter().position(|f| f.id == id)
    }

    fn selected_versions(&self) -> Vec<Version> {
        self.listing.files.iter().filter(|f| self.selected.contains(&f.id)).cloned().collect()
    }

    /// Validate and write the config; restart the service if it is running.
    /// None = nothing was written (the reason is in the toast).
    fn save_config(&mut self) -> Option<Task<Msg>> {
        let mut cfg = match self.form.to_config(&self.folders, self.theme) {
            Ok(c) => c,
            Err(e) => {
                self.notify(format!("{e}."), Kind::Error);
                return None;
            }
        };
        if let Err(e) = cfg.save(Path::new(CONFIG_PATH)) {
            self.notify(format!("Não salvou. {}", messages::config(&e)), Kind::Error);
            return None;
        }
        let seq = self.toast_seq;
        self.apply_config(&cfg);
        if self.svc != SvcState::Running {
            // Keep an index error from apply_config visible instead.
            if self.toast_seq == seq {
                self.notify("Configuração salva.", Kind::Ok);
            }
            self.refresh();
            return Some(Task::none());
        }
        self.busy = true;
        Some(work(|| {
            install::stop_own_service().map_err(|e| messages::service(&e))?;
            install::start_service(install::OWN_SERVICE).map_err(|e| messages::service(&e))?;
            Ok("Configuração salva; serviço reiniciado.".into())
        }))
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
                    Some(d) if d.exists() => {
                        Err(format!("Já existe {} na pasta escolhida; nada foi sobrescrito.", d.display()))
                    }
                    _ => crate::restore::restore(&idx, v.id, dest.as_deref())
                        .map(|_| ())
                        .map_err(|e| messages::restore(&e, &v.original_path)),
                };
                if let Err(e) = r {
                    errs.push(e);
                }
            }
            summary("arquivo(s) restaurado(s)", items.len(), errs)
        })
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        let seq = self.toast_seq;
        let task = self.handle(msg);
        match &self.toast {
            Some(t) if self.toast_seq != seq => Task::batch([task, expire(t.id, t.kind)]),
            _ => task,
        }
    }

    fn handle(&mut self, msg: Msg) -> Task<Msg> {
        if !matches!(msg, Msg::ConfirmRemove | Msg::Modifiers(_) | Msg::ToastClose(_) | Msg::CloseRequested) {
            self.confirm_remove = false;
        }
        if !matches!(msg, Msg::ConfirmPurge | Msg::Modifiers(_) | Msg::ToastClose(_) | Msg::CloseRequested) {
            self.confirm_purge = false;
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
            Msg::Purge => self.confirm_purge = !self.selected.is_empty(),
            Msg::ConfirmPurge => {
                if self.busy || self.selected.is_empty() {
                    return Task::none();
                }
                let Some(idx) = self.idx.clone() else { return Task::none() };
                let items = self.selected_versions();
                self.busy = true;
                return work(move || {
                    let errs: Vec<String> = items
                        .iter()
                        .filter_map(|v| {
                            crate::retention::purge(&idx, v.id).err().map(|e| messages::purge(&e, &v.original_path))
                        })
                        .collect();
                    summary("arquivo(s) excluído(s) de vez", items.len(), errs)
                });
            }
            Msg::Refresh => {
                let seq = self.toast_seq;
                self.refresh();
                // Keep an index error from refresh() visible instead.
                if self.toast_seq == seq {
                    self.notify("Lista atualizada.", Kind::Ok);
                }
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
                if self.busy {
                    return Task::none();
                }
                let Some(edit) = self.rule_edit.take() else { return Task::none() };
                let before = self.folders.clone();
                if let Err(e) = folders::apply_rule(&mut self.folders, edit.index, edit.to_rule()) {
                    self.notify(e, Kind::Error);
                    return Task::none();
                }
                match self.save_config() {
                    Some(task) => return task,
                    None => {
                        // Keep the dialog open with what was typed; undo the list change.
                        self.folders = before;
                        self.rule_edit = Some(edit);
                    }
                }
            }
            Msg::RemoveFolder => self.confirm_remove = self.folder_sel.is_some(),
            Msg::ConfirmRemove => {
                if let Some(i) = self.folder_sel.take() {
                    // An open editor's index would point at the wrong folder now.
                    self.rule_edit = None;
                    let before = self.folders.clone();
                    self.folders.remove(i);
                    match self.save_config() {
                        Some(task) => return task,
                        None => {
                            self.folders = before;
                            self.folder_sel = Some(i);
                        }
                    }
                }
            }
            Msg::Field(f, v) => self.form.set(f, v),
            Msg::Theme(t) => self.theme = t,
            Msg::Save => return self.save_config().unwrap_or_else(Task::none),
            Msg::Install => {
                let Some(task) = self.save_config() else { return Task::none() };
                if self.busy {
                    // A service restart is in flight; never install over it.
                    return task;
                }
                self.busy = true;
                return work(|| {
                    let exe = std::env::current_exe().map_err(|e| {
                        format!("Não foi possível localizar o executável: {}.", messages::clean(&e.to_string()))
                    })?;
                    install::install(&install::WinHost, install::EMBEDDED_DRIVER, &exe)
                        .map_err(|e| messages::install(&e))?;
                    install::start_service(install::OWN_SERVICE).map_err(|e| messages::service(&e))?;
                    Ok(format!("Instalado e iniciado. O serviço usa {}; não mova este arquivo.", exe.display()))
                });
            }
            Msg::Start => {
                self.busy = true;
                return work(|| {
                    install::start_service(install::OWN_SERVICE)
                        .map(|_| "Serviço iniciado.".into())
                        .map_err(|e| messages::service(&e))
                });
            }
            Msg::Stop => {
                self.busy = true;
                return work(|| {
                    install::stop_own_service().map(|_| "Serviço parado.".into()).map_err(|e| messages::service(&e))
                });
            }
            Msg::Autostart(on) => {
                if self.busy {
                    return Task::none();
                }
                self.busy = true;
                return work(move || {
                    install::set_service_autostart(install::OWN_SERVICE, on).map_err(|e| messages::service(&e))?;
                    Ok(settings::autostart_done(on).into())
                });
            }
            Msg::Done(r) => {
                self.busy = false;
                if self.closing {
                    return iced::exit();
                }
                match r {
                    Ok(s) => self.notify(s, Kind::Ok),
                    Err(e) => self.notify(e, Kind::Error),
                }
                self.refresh();
            }
            Msg::ToastClose(id) => {
                if self.toast.as_ref().is_some_and(|t| t.id == id) {
                    self.toast = None;
                }
            }
            // Nothing here touches the SCM: the service is its own process.
            Msg::CloseRequested => {
                if !self.busy {
                    return iced::exit();
                }
                self.closing = true;
                self.notify("Terminando a operação em andamento; a janela fecha em seguida.", Kind::Info);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        let p = self.pal();
        let tab = |label: &'static str, t: Tab| {
            button(text(label).size(13).wrapping(text::Wrapping::None))
                .padding([5, 14])
                .style(theme::tab_button(p, self.tab == t))
                .on_press(Msg::Tab(t))
        };
        let tabs = container(row![
            tab("Início", Tab::Inicio),
            tab("Pastas protegidas", Tab::Pastas),
            tab("Configurações", Tab::Config),
        ])
        .width(Length::Fill)
        .style(theme::surface(p, p.panel));
        let body = match self.tab {
            Tab::Inicio => home::view(self),
            Tab::Pastas => folders::view(self),
            Tab::Config => settings::view(self),
        };
        let main = container(column![tabs, container(body).height(Length::Fill), self.status_bar()])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::surface(p, p.bg));
        match &self.toast {
            None => main.into(),
            Some(t) => stack![
                main,
                container(self.toast_box(t))
                    .align_right(Length::Fill)
                    .align_bottom(Length::Fill)
                    .padding(iced::Padding { top: 0.0, right: 14.0, bottom: 38.0, left: 0.0 }),
            ]
            .into(),
        }
    }

    fn toast_box<'a>(&self, t: &'a Toast) -> Element<'a, Msg> {
        let p = self.pal();
        let (icon, color) = match t.kind {
            Kind::Ok => ("check_circle", p.ok),
            Kind::Info => ("info", p.acc),
            Kind::Error => ("error", p.danger),
        };
        container(
            row![
                icons::icon(icon, 20.0, color),
                text(t.text.as_str()).size(13).width(Length::Fill),
                button(icons::icon("close", 16.0, p.muted))
                    .padding(2)
                    .style(theme::flat_button(p, false))
                    .on_press(Msg::ToastClose(t.id)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([10, 12])
        .width(Length::Fixed(420.0))
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(p.panel)),
            text_color: Some(p.fg),
            border: iced::Border { color, width: 1.0, radius: 4.0.into() },
            shadow: iced::Shadow {
                color: iced::Color { a: 0.25, ..iced::Color::BLACK },
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..Default::default()
        })
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
        container(
            row![
                state("Serviço", self.svc == SvcState::Running, svc_word),
                state("Driver", self.driver == SvcState::Running, if self.driver == SvcState::Running { "carregado" } else { "não carregado" }),
                state("Auditoria", self.audit.policy_success, if self.audit.policy_success { "ativa" } else { "desligada" }),
                text(format!("Lixeira: {} de {limit} GB", listing::fmt_size(used))).size(12),
                text(format!("{} selecionado(s)", self.selected.len())).size(12),
                Space::new().width(Length::Fill),
                text(if self.busy { "Trabalhando..." } else { "" }).size(12),
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
