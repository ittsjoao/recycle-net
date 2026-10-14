//! "Pastas protegidas": the folder list with its rules and the rule editor.
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use super::home::{cell, group, label, ribbon_bar, ribbon_button, row_button, ROW_H};
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

/// Put an edited rule in the list: `None` appends, `Some(i)` replaces folder `i`.
pub fn apply_rule(folders: &mut Vec<FolderRule>, index: Option<usize>, rule: FolderRule) -> Result<(), String> {
    match index {
        Some(i) => *folders.get_mut(i).ok_or("A pasta editada não está mais na lista.")? = rule,
        None => folders.push(rule),
    }
    Ok(())
}

pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let idle = !app.busy;
    let sel = app.folder_sel.is_some() && idle;
    let mut groups = vec![group(
        p,
        row![
            ribbon_button(p, "create_new_folder", "Adicionar pasta", idle.then_some(Msg::AddFolder), false),
            ribbon_button(p, "rule", "Editar regras", sel.then_some(Msg::EditRules), false),
            ribbon_button(p, "folder_off", "Remover", sel.then_some(Msg::RemoveFolder), true),
        ]
        .spacing(2),
        "Pastas",
    )];
    if app.confirm_remove {
        groups.push(group(p, ribbon_button(p, "delete_forever", "Confirmar", Some(Msg::ConfirmRemove), true), "Remover a pasta?"));
    }
    let ribbon = ribbon_bar(p, groups);

    let w = [Length::FillPortion(4), Length::FillPortion(3), Length::FillPortion(3), Length::Fixed(90.0)];
    let head_cell = |s: &'static str, w: Length| cell(container(label(s, 12).color(p.muted)).center_y(Length::Fixed(ROW_H)), w);
    let head = row![
        head_cell("Pasta (com subpastas)", w[0]),
        head_cell("Incluir", w[1]),
        head_cell("Excluir", w[2]),
        head_cell("Apagados", w[3]),
    ]
    .padding([0, 6])
    .width(Length::Fill);
    let mut list = column![head].spacing(1).width(Length::Fill);
    for (i, f) in app.folders.iter().enumerate() {
        let path = f.path.display().to_string();
        let exclude = if f.exclude.is_empty() { "nada".into() } else { f.exclude.join("; ") };
        list = list.push(row_button(
            p,
            row![
                cell(row![super::icons::icon("folder_special", 17.0, p.acc), label(path.clone(), 13)].spacing(6), w[0]),
                cell(label(f.include.join("; "), 13), w[1]),
                cell(label(exclude, 13), w[2]),
                cell(label(app.tree.info(&path).count.to_string(), 13), w[3]),
            ]
            .align_y(Alignment::Center),
            app.folder_sel == Some(i),
            Msg::SelectFolder(i),
        ));
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
                        button(text("Salvar").size(13)).on_press_maybe(idle.then_some(Msg::RuleSave)),
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

    #[test]
    fn apply_rule_replaces_appends_and_rejects_stale_index() {
        let a = FolderRule::with_defaults(r"D:\A".into());
        let b = FolderRule::with_defaults(r"D:\B".into());
        let mut v = vec![a.clone()];
        apply_rule(&mut v, None, b.clone()).unwrap();
        assert_eq!(v, [a.clone(), b.clone()]);
        apply_rule(&mut v, Some(0), b.clone()).unwrap();
        assert_eq!(v, [b.clone(), b.clone()]);
        assert!(apply_rule(&mut v, Some(2), a).is_err());
        assert_eq!(v, [b.clone(), b]);
    }
}
