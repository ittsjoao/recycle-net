//! "Configurações" tab (settings, service controls, credits).
use std::path::PathBuf;

use iced::widget::{button, column, row, text, text_input, Space};
use iced::{Alignment, Element, Length};

use super::home::{group, ribbon_bar, ribbon_button};
use super::{App, Msg};
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
    // Installing with no protected folder would start a service that guards nothing.
    let can_install = idle && app.svc == SvcState::NotInstalled && !app.folders.is_empty();
    let ribbon = ribbon_bar(
        p,
        vec![
            group(p, ribbon_button(p, "save", "Salvar", idle.then_some(Msg::Save), false), "Configuração"),
            group(
                p,
                row![
                    ribbon_button(p, "install_desktop", "Instalar", can_install.then_some(Msg::Install), false),
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
