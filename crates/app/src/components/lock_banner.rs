//! The banner an entity shows while somebody else is editing it: who, what that means, and "Take over".

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, button::Button, h_flex};
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};

use crate::{
    app_model::ModelHandle,
    components::dialogs::confirm_danger,
    i18n::{t, t_with},
    model::claims::Claim,
};

/// The banner for `claim`, held by another session. "Take over" asks first, then claims it by force.
pub fn lock_banner<V: 'static>(claim: Claim, cx: &mut Context<V>) -> AnyElement {
    let theme = cx.theme().clone();
    let text = t_with("claims.banner", &[("by", &claim.by)]);
    let take_over = cx.listener({
        let claim = claim.clone();
        move |_, _: &ClickEvent, window, cx| {
            let Some(model) = cx.try_global::<ModelHandle>().and_then(|handle| handle.0.upgrade()) else { return };
            let claim = claim.clone();
            let title = t_with("claims.confirm.title", &[("name", &claim.name), ("by", &claim.by)]);
            let body = t_with("claims.confirm.body", &[("by", &claim.by)]);
            confirm_danger(cx.weak_entity(), window, cx, title, body, t("claims.confirm.ok"), move |_, _, cx| {
                model.update(cx, |model, cx| model.take_over(&claim, cx))
            });
        }
    });
    h_flex()
        .id("lock-banner")
        .test_support()
        .flex_none()
        .px_3()
        .py_1p5()
        .gap_2()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.warning.opacity(0.12))
        .child(Icon::new(IconName::Lock).small().text_color(theme.warning))
        .child(div().flex_1().min_w_0().text_sm().child(text))
        .child(Button::new("lock-take-over").outline().small().label(t("claims.take_over")).on_click(take_over))
        .into_any_element()
}
