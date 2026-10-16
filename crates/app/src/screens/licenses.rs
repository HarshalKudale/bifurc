//! The Licenses page of Settings: Bifurc's own license, the assets it bundles, and the notice of every
//! open-source library it ships.
//!
//! Most of those licenses (MIT, BSD, Apache) require the notice to travel with the program, so the page is
//! where a user can read every one. The notices are data, not code: `licenses/third-party.json` is produced
//! by `cargo xtask licenses` from the dependency graph and embedded here. Regenerate it whenever
//! dependencies change.

use std::{collections::HashSet, sync::OnceLock};

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, WindowExt as _,
    button::Button,
    dialog::{DialogButtonProps, DialogFooter},
    h_flex,
    link::Link,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Axis, Context, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use serde::Deserialize;

use crate::i18n::{t, t_with};

const REPOSITORY_URL: &str = "https://github.com/HarshalKudale/bifurc";
const LICENSE_URL: &str = "https://github.com/HarshalKudale/bifurc/blob/master/LICENSE";
const NONCOMMERCIAL_URL: &str = "https://polyformproject.org/licenses/noncommercial/1.0.0";

#[derive(Debug, Deserialize)]
struct Notices {
    libraries: usize,
    assets: Vec<Asset>,
    groups: Vec<Group>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    license: String,
    detail: String,
}

#[derive(Debug, Deserialize)]
struct Group {
    name: String,
    #[serde(default)]
    note: String,
    libraries: usize,
    notices: Vec<Notice>,
}

/// One license text and the libraries it covers.
#[derive(Debug, Deserialize)]
struct Notice {
    crates: Vec<String>,
    text: String,
}

fn notices() -> &'static Notices {
    static NOTICES: OnceLock<Notices> = OnceLock::new();
    NOTICES.get_or_init(|| {
        serde_json::from_str(include_str!("../../licenses/third-party.json")).expect("embedded license notices parse")
    })
}

/// The page's groups; `index` of a group is what its "View notices" button opens.
pub fn page() -> SettingPage {
    let data = notices();
    let own = SettingGroup::new()
        .title("Bifurc")
        .item(
            SettingItem::new(
                t("licenses.own.license.title"),
                SettingField::render(|_, _, _| {
                    Link::new("own-license-link").href(NONCOMMERCIAL_URL).child("PolyForm Noncommercial 1.0.0")
                }),
            )
            .description(t("licenses.own.license.description"))
            .layout(Axis::Horizontal),
        )
        .item(
            SettingItem::new(
                t("licenses.own.full_text.title"),
                SettingField::render(|_, _, _| Link::new("own-license-text").href(LICENSE_URL).child("LICENSE")),
            )
            .description(t_with("licenses.own.full_text.description", &[("url", &REPOSITORY_URL)]))
            .layout(Axis::Horizontal),
        );

    let assets = data.assets.iter().fold(SettingGroup::new().title(t("licenses.assets.title")), |group, asset| {
        group.item(
            SettingItem::new(
                asset.name.clone(),
                SettingField::render({
                    let license = asset.license.clone();
                    move |_, _, cx| div().text_color(cx.theme().muted_foreground).child(license.clone())
                }),
            )
            .description(asset.detail.clone())
            .keywords(t("licenses.assets.keywords").split(',')),
        )
    });

    let libraries = data
        .groups
        .iter()
        .enumerate()
        .fold(
            SettingGroup::new().title(t_with("licenses.libraries.title", &[("count", &data.libraries)])),
            |group, (index, license)| {
                group.item(
                    SettingItem::new(
                        license.name.clone(),
                        SettingField::render(move |options, _, _| {
                            Button::new(ElementId::Name(format!("view-notices-{index}").into()))
                                .with_size(options.size())
                                .label(t("licenses.libraries.view_notices"))
                                .on_click(move |_, window, cx| open_notices(index, window, cx))
                        }),
                    )
                    .description(t_with(
                        if license.libraries == 1 {
                            "licenses.libraries.description_one"
                        } else {
                            "licenses.libraries.description_other"
                        },
                        &[("count", &license.libraries), ("note", &license.note)],
                    ))
                    .keywords(t("licenses.libraries.keywords").split(',')),
                )
            },
        )
        .footer(|_, _| t("licenses.libraries.footer"));

    SettingPage::new(t("licenses.page.title")).icon(IconName::Info).groups(vec![own, assets, libraries])
}

/// A scrollable list of one license's notices; each opens to its full text.
struct NoticeList {
    group: usize,
    open: HashSet<usize>,
}

impl Render for NoticeList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let group = &notices().groups[self.group];
        let rows = group.notices.iter().enumerate().map(|(ix, notice)| {
            let expanded = self.open.contains(&ix);
            let title: SharedString = match notice.crates.as_slice() {
                [only] => only.clone().into(),
                [first, rest @ ..] => format!("{first} and {} more", rest.len()).into(),
                [] => SharedString::default(),
            };
            let body = expanded.then(|| {
                v_flex()
                    .gap_2()
                    .px_3()
                    .pb_3()
                    .child(div().text_xs().text_color(theme.muted_foreground).child(notice.crates.join(", ")))
                    .child(div().text_xs().font_family(theme.mono_font_family.clone()).child(notice.text.clone()))
            });
            v_flex()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .id(ElementId::Name(format!("notice-{ix}").into()))
                        .cursor_pointer()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .hover(|row| row.bg(theme.accent))
                        .child(div().w_4().text_color(theme.muted_foreground).child(if expanded {
                            "▾"
                        } else {
                            "▸"
                        }))
                        .child(div().flex_1().min_w_0().truncate().child(title))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.open.remove(&ix) {
                                this.open.insert(ix);
                            }
                            cx.notify();
                        })),
                )
                .children(body)
        });
        v_flex().id("notice-list").max_h(px(460.)).overflow_y_scroll().children(rows)
    }
}

fn open_notices(index: usize, window: &mut Window, cx: &mut App) {
    let list: Entity<NoticeList> = cx.new(|_| NoticeList { group: index, open: HashSet::new() });
    let title = notices().groups[index].name.clone();
    // Deferred so the dialog opens after the click that asked for it has been handled.
    window.defer(cx, move |window, cx| {
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(760.))
                .child(list.clone())
                .button_props(DialogButtonProps::default())
                .footer(
                    DialogFooter::new().child(
                        Button::new("notices-close")
                            .outline()
                            .label(t("licenses.notices.close"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_notices_cover_every_library_with_a_license_text() {
        let data = notices();
        assert!(data.libraries > 0);
        let covered: usize = data.groups.iter().map(|group| group.libraries).sum();
        assert!(covered >= data.libraries, "{covered} covered of {}", data.libraries);
        for group in &data.groups {
            for notice in &group.notices {
                assert!(!notice.crates.is_empty());
                assert!(notice.text.trim().len() > 100, "{} has no real license text", notice.crates[0]);
            }
        }
    }

    #[test]
    fn the_permissive_licenses_that_need_a_notice_are_present() {
        let names: Vec<_> = notices().groups.iter().map(|group| group.name.as_str()).collect();
        for expected in ["MIT License", "Apache License 2.0"] {
            assert!(names.contains(&expected), "{expected} missing from {names:?}");
        }
    }
}
