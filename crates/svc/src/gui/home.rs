//! "Início": ribbon, bin tree, breadcrumb and the folder listing.
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space};
use iced::{font, Alignment, Element, Font, Length};

use super::listing::{self, SortCol};
use super::theme::{self, Pal};
use super::tree;
use super::{icons, App, Msg};

const BOLD: Font = Font { weight: font::Weight::Bold, ..Font::DEFAULT };

/// Every ribbon button has this box, so hover fills the same area everywhere.
const BTN_W: f32 = 92.0;
const BTN_H: f32 = 58.0;
/// Height of one list/tree row.
pub(super) const ROW_H: f32 = 26.0;

/// One-line label: never wraps, so rows and buttons keep their fixed box.
pub(super) fn label<'a>(s: impl text::IntoFragment<'a>, size: impl Into<iced::Pixels>) -> iced::widget::Text<'a> {
    text(s).size(size).wrapping(text::Wrapping::None)
}

/// Ribbon button: icon above the label in a fixed box; no message = disabled.
pub(super) fn ribbon_button<'a>(p: Pal, icon: &str, caption: &'a str, msg: Option<Msg>, danger: bool) -> Element<'a, Msg> {
    let color = if msg.is_none() { p.muted } else if danger { p.danger } else { p.acc };
    let content = column![icons::icon(icon, 24.0, color), label(caption, 12)]
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::Fill);
    button(container(content).center_y(Length::Fill))
        .width(Length::Fixed(BTN_W))
        .height(Length::Fixed(BTN_H))
        .padding([4, 2])
        .clip(true)
        .style(theme::flat_button(p, false))
        .on_press_maybe(msg)
        .into()
}

/// A ribbon group: content centered vertically in a fixed-height band, the
/// caption under it on the same baseline as every other group's.
pub(super) fn group<'a>(p: Pal, content: impl Into<Element<'a, Msg>>, caption: &'a str) -> Element<'a, Msg> {
    column![container(content).center_y(Length::Fixed(BTN_H)), label(caption, 11).font(BOLD).color(p.muted)]
        .spacing(2)
        .align_x(Alignment::Center)
        .padding([4, 10])
        .into()
}

/// The ribbon: groups separated by thin vertical lines, same on every tab.
pub(super) fn ribbon_bar<'a>(p: Pal, groups: Vec<Element<'a, Msg>>) -> Element<'a, Msg> {
    let mut r = row![].align_y(Alignment::Center);
    for (i, g) in groups.into_iter().enumerate() {
        if i > 0 {
            r = r.push(
                container(Space::new())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(BTN_H + 12.0))
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(p.line)),
                        ..Default::default()
                    }),
            );
        }
        r = r.push(g);
    }
    container(r).padding([2, 6]).width(Length::Fill).style(theme::surface(p, p.bg)).into()
}

/// A list/tree row: fixed height, full width, so hover and selection paint the whole box.
pub(super) fn row_button<'a>(p: Pal, content: impl Into<Element<'a, Msg>>, selected: bool, msg: Msg) -> Element<'a, Msg> {
    button(container(content).width(Length::Fill).center_y(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(ROW_H))
        .padding([0, 6])
        .clip(true)
        .style(theme::flat_button(p, selected))
        .on_press(msg)
        .into()
}

/// A table cell of width `w` that clips what does not fit.
pub(super) fn cell<'a>(content: impl Into<Element<'a, Msg>>, w: Length) -> Element<'a, Msg> {
    container(content).width(w).clip(true).into()
}

/// Part of `cur` below `root` ("" when `root` is not a byte prefix of it).
pub(super) fn crumb_tail<'a>(cur: &'a str, root: &str) -> &'a str {
    cur.get(root.len()..).unwrap_or("")
}

pub(super) fn confirm_purge_caption(n: usize) -> String {
    format!("Confirmar exclusão ({n})")
}

fn ribbon(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let has_sel = !app.selected.is_empty() && !app.busy;
    let on = |m: Msg, ok: bool| ok.then_some(m);
    let mut groups = vec![
            group(
                p,
                row![
                    icons::icon("search", 18.0, p.muted),
                    text_input("Buscar nesta pasta e abaixo", &app.query).on_input(Msg::Search).width(Length::Fixed(240.0)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
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
                .spacing(2),
                "Ações",
            ),
            group(p, ribbon_button(p, "refresh", "Atualizar", on(Msg::Refresh, !app.busy), false), "Lista"),
            group(p, checkbox(app.all_copies).label("Todas as cópias").on_toggle(Msg::AllCopies), "Mostrar"),
        ];
    if app.confirm_purge {
        let confirm = button(
            row![icons::icon("delete_forever", 20.0, p.danger), label(confirm_purge_caption(app.selected.len()), 12).color(p.danger)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .padding([4, 8])
        .style(theme::flat_button(p, false))
        .on_press_maybe(has_sel.then_some(Msg::ConfirmPurge));
        groups.push(group(
            p,
            column![confirm, label("não dá para desfazer", 11).color(p.muted)].spacing(2).align_x(Alignment::Center),
            "Excluir de vez?",
        ));
    }
    ribbon_bar(p, groups)
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
    let name = if path.is_empty() {
        "Lixeira".to_string()
    } else if is_root {
        path.to_string()
    } else {
        listing::split_path(path).0.to_string()
    };
    let name = label(name, 13).font(if is_root || path.is_empty() { BOLD } else { Font::DEFAULT });
    let line = row![
        Space::new().width(Length::Fixed(depth as f32 * 14.0)),
        button(container(label(marker, 12)).center(Length::Fill))
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(ROW_H))
            .padding(0)
            .style(theme::flat_button(p, false))
            .on_press(Msg::ToggleNode(path.to_string())),
        row_button(
            p,
            row![icons::icon(icon, 16.0, p.acc), name, label(format!("({})", app.tree.info(path).count), 11).color(p.muted)]
                .spacing(5)
                .align_y(Alignment::Center),
            current,
            Msg::Go(path.to_string()),
        ),
    ]
    .width(Length::Fill)
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
    for part in crumb_tail(&app.cur, &root).split('\\').filter(|s| !s.is_empty()) {
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
    let h = |name: &'static str, col: SortCol, w: Length| {
        let arrow = if app.sort.0 == col { if app.sort.1 { " ▲" } else { " ▼" } } else { "" };
        button(container(label(format!("{name}{arrow}"), 12).color(p.muted)).center_y(Length::Fill))
            .padding(0)
            .width(w)
            .height(Length::Fixed(ROW_H))
            .clip(true)
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
    // Same horizontal padding as `row_button`, so the columns line up.
    .padding([0, 6])
    .width(Length::Fill)
    .into()
}

fn rows(app: &App) -> Element<'_, Msg> {
    let p = app.pal();
    let txt = |s: String, w: Length| cell(label(s, 13), w);
    let dim = |s: String, w: Length| cell(label(s, 13).color(p.muted), w);
    let mut col = column![].spacing(1).width(Length::Fill);
    for d in &app.listing.dirs {
        let icon = if app.cur.is_empty() { "folder_special" } else { "folder" };
        col = col.push(row_button(
            p,
            row![
                cell(row![icons::icon(icon, 17.0, p.acc), label(d.name.clone(), 13).font(BOLD)].spacing(6), W_NAME),
                dim(format!("{} apagado(s)", d.info.count), W_COPIES),
                txt(String::new(), W_USER),
                dim(listing::fmt_size(d.info.size), W_SIZE),
                dim(if d.info.count > 0 { listing::fmt_time(d.info.last) } else { String::new() }, W_TIME),
            ]
            .align_y(Alignment::Center),
            false,
            Msg::Go(d.path.clone()),
        ));
    }
    for f in &app.listing.files {
        let name = if app.query.trim().is_empty() && app.copies_of.is_none() {
            listing::split_path(&f.original_path).0.to_string()
        } else if app.cur.is_empty() {
            f.original_path.clone()
        } else {
            f.original_path.get(app.cur.len() + 1..).unwrap_or(&f.original_path).to_string()
        };
        let user = match &f.user_name {
            Some(u) => txt(u.clone(), W_USER),
            None => dim("desconhecido".into(), W_USER),
        };
        col = col.push(row_button(
            p,
            row![
                cell(row![icons::icon("description", 17.0, p.acc), label(name, 13)].spacing(6), W_NAME),
                txt(if f.copies > 1 { f.copies.to_string() } else { "-".into() }, W_COPIES),
                user,
                txt(listing::fmt_size(f.size), W_SIZE),
                txt(listing::fmt_time(f.deleted_utc), W_TIME),
            ]
            .align_y(Alignment::Center),
            app.selected.contains(&f.id),
            Msg::Click(f.id),
        ));
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
    let left = column![title("Lixeira"), scrollable(column(nodes).spacing(1).width(Length::Fill)).height(Length::Fill)]
        .spacing(6)
        .padding(8)
        .width(Length::Fixed(270.0));
    let right = column![title("Arquivos recuperáveis"), crumbs(app), header(app), rows(app)].spacing(6).padding(8);
    column![ribbon(app), row![container(left).style(theme::surface(p, p.bg)), right].height(Length::Fill)].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crumb_tail_never_panics() {
        assert_eq!(crumb_tail(r"D:\Dados\a\b", r"D:\Dados"), r"\a\b");
        assert_eq!(crumb_tail(r"D:\Dados", r"D:\Dados"), "");
        // Root matched case-insensitively with a different byte length: no char boundary.
        assert_eq!(crumb_tail(r"D:\É\x", r"D:\e"), "");
        assert_eq!(crumb_tail("D:", r"D:\Dados"), "");
    }

    #[test]
    fn confirm_purge_caption_counts_selection() {
        assert_eq!(confirm_purge_caption(3), "Confirmar exclusão (3)");
    }
}
