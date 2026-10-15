//! "Configurações" tab (settings, service controls, credits).
use std::path::{Path, PathBuf};

use iced::widget::{button, checkbox, column, row, text, text_input, Space};
use iced::{Alignment, Element, Length};

use super::home::{group, ribbon_bar, ribbon_button};
use super::{App, Msg};
use crate::audit::setup::AuditStatus;
use crate::config::{Config, FolderRule, ThemeName, CONFIG_PATH};
use crate::install::SvcState;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

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
    /// `max_bin_bytes` as loaded: kept on save while `max_gb` still shows it.
    pub orig_bytes: u64,
}

fn fmt_gb(bytes: u64) -> String {
    format!("{}", (bytes as f64 / GB * 100.0).round() / 100.0)
}

impl Form {
    pub fn from_config(c: &Config) -> Form {
        Form {
            recycle_dir: c.recycle_dir.display().to_string(),
            db_path: c.db_path.display().to_string(),
            max_gb: fmt_gb(c.max_bin_bytes),
            max_age_days: c.max_age_days.map(|d| d.to_string()).unwrap_or_default(),
            orig_bytes: c.max_bin_bytes,
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
            // The GB text is rounded; an untouched field keeps a hand-set byte value.
            max_bin_bytes: if self.max_gb.trim() == fmt_gb(self.orig_bytes) { self.orig_bytes } else { (gb * GB) as u64 },
            max_age_days,
            theme,
            folders: folders.to_vec(),
        })
    }
}

/// Write only `theme` into the config at `path` (the service ignores it, so no restart).
/// False when the file is missing or invalid: the theme then applies in memory only.
pub fn save_theme(path: &Path, theme: ThemeName) -> bool {
    match Config::load(path) {
        Ok(mut c) => {
            c.theme = theme;
            c.save(path).is_ok()
        }
        Err(_) => false,
    }
}

/// "Auditoria (quem apagou)" line of the settings tab.
pub fn audit_text(a: &AuditStatus) -> &'static str {
    if a.policy_success && a.builtin_users {
        "ativa: exclusões de todos os usuários locais (servidor fora de domínio; mais eventos no log)"
    } else if a.policy_success {
        "ativa: exclusões de Usuários do Domínio, log de Segurança com 512 MB ou mais"
    } else if a.configured {
        "desligada (uma política de grupo pode ter sobrescrito); a coluna Usuário mostrará \"desconhecido\""
    } else {
        "ainda não configurada: o serviço configura ao iniciar"
    }
}

/// Note under the settings: where the config lives, or how to get the service back.
pub fn service_note(svc: SvcState, exe: &str) -> String {
    if svc == SvcState::NotInstalled {
        "Serviço não instalado. Reinstale o rnetrbin pelo setup.".into()
    } else {
        format!(
            "Configuração gravada em {CONFIG_PATH}. Salvar com o serviço rodando o reinicia. \
             O serviço usa {exe}; não mova este arquivo."
        )
    }
}

/// Toast after "Iniciar o serviço com o Windows" was applied.
pub fn autostart_done(on: bool) -> &'static str {
    if on {
        "O serviço vai iniciar junto com o Windows."
    } else {
        "O serviço não vai mais iniciar sozinho; use Iniciar."
    }
}

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let idle = !app.busy;
    let ribbon = ribbon_bar(
        p,
        vec![
            group(p, ribbon_button(p, "save", "Salvar", idle.then_some(Msg::Save), false), "Configuração"),
            group(
                p,
                row![
                    ribbon_button(p, "play_circle", "Iniciar", (idle && app.svc == SvcState::Stopped).then_some(Msg::Start), false),
                    ribbon_button(p, "stop_circle", "Parar", (idle && app.svc == SvcState::Running).then_some(Msg::Stop), true),
                ]
                .spacing(2),
                "Serviço",
            ),
        ],
    );

    let field = |label: &'static str, hint: &'static str, value: &str, f: Field| {
        row![text(label).size(13).width(Length::Fixed(230.0)), text_input(hint, value).on_input(move |v| Msg::Field(f, v))]
            .align_y(Alignment::Center)
    };
    let theme_btn = |label: &'static str, t: ThemeName| {
        button(text(label).size(13))
            .style(if app.theme == t { iced::widget::button::primary } else { iced::widget::button::secondary })
            .on_press(Msg::Theme(t))
    };
    let audit = audit_text(&app.audit);
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
            row![
                text("Inicialização").size(13).width(Length::Fixed(230.0)),
                checkbox(app.autostart.unwrap_or(false))
                    .label("Iniciar o serviço com o Windows")
                    .text_size(13)
                    .on_toggle_maybe((idle && app.autostart.is_some()).then_some(Msg::Autostart)),
            ]
            .align_y(Alignment::Center),
            row![text("Auditoria (quem apagou)").size(13).width(Length::Fixed(230.0)), text(audit).size(13)],
            text(service_note(app.svc, &exe)).size(12).color(if app.svc == SvcState::NotInstalled { p.danger } else { p.muted }),
            text("Fechar esta janela não para o serviço; a proteção continua rodando.").size(12).color(p.muted),
            Space::new().height(Length::Fill),
            text(format!("rnetrbin {} · Ícones: Material Icons, Google (Apache 2.0)", env!("CARGO_PKG_VERSION")))
                .size(11)
                .color(p.muted),
        ]
        .spacing(10)
        .padding(12),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_installed_service_points_to_the_setup() {
        assert_eq!(service_note(SvcState::NotInstalled, r"C:\x.exe"), "Serviço não instalado. Reinstale o rnetrbin pelo setup.");
        let n = service_note(SvcState::Running, r"C:\Arquivos\rnetrbin-svc.exe");
        assert!(n.contains(r"C:\Arquivos\rnetrbin-svc.exe"));
        assert!(!n.contains("Instalar"));
    }

    #[test]
    fn autostart_toast_says_what_happens_next() {
        assert_eq!(autostart_done(true), "O serviço vai iniciar junto com o Windows.");
        assert_eq!(autostart_done(false), "O serviço não vai mais iniciar sozinho; use Iniciar.");
    }

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

    #[test]
    fn form_keeps_a_hand_set_byte_limit_unless_the_gb_text_changes() {
        let c = Config { max_bin_bytes: 10_000_000_123, ..Config::default() };
        let f = Form::from_config(&c);
        assert_eq!(f.to_config(&[], ThemeName::Rust).unwrap().max_bin_bytes, 10_000_000_123);
        let mut g = f.clone();
        g.set(Field::MaxGb, "2".into());
        assert_eq!(g.to_config(&[], ThemeName::Rust).unwrap().max_bin_bytes, (2.0 * GB) as u64);
    }

    #[test]
    fn audit_text_warns_about_workgroup_auditing() {
        let on = AuditStatus { policy_success: true, configured: true, builtin_users: false };
        assert!(audit_text(&on).contains("Usuários do Domínio"));
        assert_eq!(
            audit_text(&AuditStatus { builtin_users: true, ..on }),
            "ativa: exclusões de todos os usuários locais (servidor fora de domínio; mais eventos no log)"
        );
        assert!(audit_text(&AuditStatus { policy_success: false, ..on }).starts_with("desligada"));
        assert!(audit_text(&AuditStatus::default()).starts_with("ainda não configurada"));
    }

    #[test]
    fn save_theme_writes_only_the_theme_and_never_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(!save_theme(&path, ThemeName::Preto));
        assert!(!path.exists());
        let mut c = Config { max_age_days: Some(7), folders: vec![FolderRule::with_defaults(r"D:\Dados".into())], ..Config::default() };
        c.save(&path).unwrap();
        assert!(save_theme(&path, ThemeName::Preto));
        assert_eq!(Config::load(&path).unwrap(), Config { theme: ThemeName::Preto, ..c });
        std::fs::write(&path, "not toml [").unwrap();
        assert!(!save_theme(&path, ThemeName::Branco));
    }
}
