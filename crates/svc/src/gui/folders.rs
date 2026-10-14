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
