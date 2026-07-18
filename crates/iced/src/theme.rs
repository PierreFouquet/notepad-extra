//! Custom theme for the native shell (#70). Single source of truth for every
//! chrome colour: a palette of design tokens defining the app's light and dark
//! looks. The shell hand-styles its chrome from [`Tokens`];
//! the custom [`iced::Theme`] built alongside (see `main.rs`) only exists so the
//! widgets we *don't* hand-style (pick-list menus, the editor's internal
//! gutter / active-line / selection) still land in the right palette.
use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow};

/// One theme's worth of colour tokens. `Copy` (every field is a `Color`), so a
/// `Tokens` threads into each `.style()` closure by value with no allocation.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_fg: Color,
    pub app_bg: Color,
    pub toolbar_bg: Color,
    pub toolbar_border: Color,
    pub divider: Color,
    pub text: Color,
    pub text_muted: Color,
    pub btn_bg: Color,
    pub btn_border: Color,
    pub btn_hover: Color,
    pub btn_active: Color,
    pub tabs_bg: Color,
    pub tab_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_border: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub pill_bg: Color,
    pub popup_bg: Color,
    pub popup_border: Color,
    pub input_bg: Color,
    pub danger: Color,
    pub overlay: Color,
}

/// Build a `Color` from a packed `0xRRGGBB`. `const` (composed from components,
/// not the non-const `Color::from_rgb8`), so [`LIGHT`] / [`DARK`] are true consts.
const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

/// Light-theme tokens — the app's light palette.
pub const LIGHT: Tokens = Tokens {
    accent: rgb(0x2563eb),
    accent_hover: rgb(0x1d4ed8),
    accent_fg: rgb(0xffffff),
    app_bg: rgb(0xffffff),
    toolbar_bg: rgb(0xf6f7f9),
    toolbar_border: rgb(0xdfe3e8),
    divider: rgb(0xe2e6eb),
    text: rgb(0x1f2328),
    text_muted: rgb(0x626a73),
    btn_bg: rgb(0xffffff),
    btn_border: rgb(0xd4d9df),
    btn_hover: rgb(0xeef1f5),
    btn_active: rgb(0xe2e7ee),
    tabs_bg: rgb(0xeceff3),
    tab_fg: rgb(0x5c636b),
    tab_active_bg: rgb(0xffffff),
    tab_active_fg: rgb(0x1a1a1a),
    tab_border: rgb(0xdfe3e8),
    status_bg: rgb(0xf6f7f9),
    status_fg: rgb(0x4b5259),
    pill_bg: rgb(0xe7ecf2),
    popup_bg: rgb(0xffffff),
    popup_border: rgb(0xd4d9df),
    input_bg: rgb(0xffffff),
    danger: rgb(0xe5534b),
    overlay: Color::from_rgba(0.07, 0.09, 0.15, 0.38),
};

/// Dark-theme tokens — the app's dark palette. The Monokai-ish
/// `app_bg` `#272822` is deliberately **equal** to `tab_active_bg`, which is what
/// lets the active tab visually fold into the editor below it (#70).
pub const DARK: Tokens = Tokens {
    accent: rgb(0x4a90e2),
    accent_hover: rgb(0x5b9de8),
    accent_fg: rgb(0xffffff),
    app_bg: rgb(0x272822),
    toolbar_bg: rgb(0x2f312a),
    toolbar_border: rgb(0x1c1d18),
    divider: rgb(0x3d3f36),
    text: rgb(0xf2f2ea),
    text_muted: rgb(0xa6a89b),
    btn_bg: rgb(0x3a3c33),
    btn_border: rgb(0x4a4c40),
    btn_hover: rgb(0x45473c),
    btn_active: rgb(0x4f5144),
    tabs_bg: rgb(0x23241f),
    tab_fg: rgb(0xb0b2a4),
    tab_active_bg: rgb(0x272822),
    tab_active_fg: rgb(0xf8f8f2),
    tab_border: rgb(0x1c1d18),
    status_bg: rgb(0x2f312a),
    status_fg: rgb(0xc3c5b8),
    pill_bg: rgb(0x3d3f36),
    popup_bg: rgb(0x33352d),
    popup_border: rgb(0x14150f),
    input_bg: rgb(0x24251f),
    danger: rgb(0xe5534b),
    overlay: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
};

// ---- Buttons --------------------------------------------------------------

/// A fully-specified `button::Style` from its three colours (the shared body of
/// every button helper). `iced_widget` 0.14's `Style` carries a `snap` field with
/// no `Default`, so it is written out in full here.
fn button_base(bg: Color, fg: Color, border: Color) -> button::Style {
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: border,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Solid file-action button (New / Open / Save).
pub fn btn(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| match status {
        button::Status::Hovered => button_base(t.btn_hover, t.text, t.btn_border),
        button::Status::Pressed => button_base(t.btn_active, t.text, t.btn_border),
        _ => button_base(t.btn_bg, t.text, t.btn_border),
    }
}

/// The ghost look for one button status: transparent and
/// muted until hovered. Factored out of the closure so [`ghost`] and [`toggle`]
/// share one body (and so the two are the *same* opaque type where it matters).
fn ghost_style(t: Tokens, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => button_base(t.btn_hover, t.text, Color::TRANSPARENT),
        button::Status::Pressed => button_base(t.btn_active, t.text, Color::TRANSPARENT),
        _ => button_base(Color::TRANSPARENT, t.text_muted, Color::TRANSPARENT),
    }
}

/// The accent "on" look for one button status. Shared by
/// [`toggle`]'s on-state (see [`ghost_style`] for why it's a free fn).
fn accent_style(t: Tokens, status: button::Status) -> button::Style {
    let bg = if matches!(status, button::Status::Hovered) {
        t.accent_hover
    } else {
        t.accent
    };
    button_base(bg, t.accent_fg, t.accent)
}

/// Ghost / utility button (Save As, Theme…): transparent
/// and muted until hovered. A real, fully-functional button; only the look
/// differs from [`btn`].
pub fn ghost(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| ghost_style(t, status)
}

/// A solid accent button — the primary call-to-action in the panels (a confirm
/// bar's Save, the About panel's Close). The same look [`toggle`] shows in its
/// "on" state.
pub fn accent(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| accent_style(t, status)
}

/// A toolbar toggle (Find / Wrap / gutter / About): the accent "on" look while
/// `on`, the ghost look otherwise. This is **one** function — hence one opaque
/// type — so it can be handed straight to `.style()` for a state that flips at
/// runtime. `if on { accent(t) } else { ghost(t) }` cannot: each `impl Fn`
/// helper is a *distinct* opaque type, so the two `if`-arms don't unify.
pub fn toggle(t: Tokens, on: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        if on {
            accent_style(t, status)
        } else {
            ghost_style(t, status)
        }
    }
}

/// The destructive-action button (Don't Save / Discard all).
pub fn danger(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = if matches!(status, button::Status::Hovered) {
            // Nudge toward white on hover for a pressed-brighter cue.
            Color {
                a: 0.85,
                ..t.danger
            }
        } else {
            t.danger
        };
        button_base(bg, Color::WHITE, t.danger)
    }
}

/// One cell of the segmented zoom control (`A− │ A │ A+`): square corners so the
/// three cells butt together inside the group's rounded, clipped border.
pub fn segment(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match status {
            button::Status::Hovered => t.btn_hover,
            button::Status::Pressed => t.btn_active,
            _ => t.btn_bg,
        };
        button::Style {
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..button_base(bg, t.text, Color::TRANSPARENT)
        }
    }
}

/// The rounded, clipped frame around the segmented zoom control (`A− │ A │ A+`):
/// a 1px `btn_border` outline with no fill, so the three [`segment`] cells butt
/// together edge-to-edge inside it and read as one control.
pub fn segment_group(t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        border: Border {
            color: t.btn_border,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

/// A tab's select-button: transparent fill so it folds into the tab container's
/// own background, carrying only the tab's foreground colour. Square corners so
/// it merges with the surrounding tab body.
pub fn tab_label(fg: Color) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, _status| button::Style {
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..button_base(Color::TRANSPARENT, fg, Color::TRANSPARENT)
    }
}

/// A tab's close (`×`) button: muted and transparent, reddening on hover.
pub fn tab_close(t: Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let (bg, fg) = match status {
            button::Status::Hovered => (t.danger, Color::WHITE),
            button::Status::Pressed => (t.danger, Color::WHITE),
            _ => (Color::TRANSPARENT, t.tab_fg),
        };
        button::Style {
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..button_base(bg, fg, Color::TRANSPARENT)
        }
    }
}

// ---- Containers -----------------------------------------------------------

/// A flat filled bar (toolbar, controls bar, status bar) painted `bg`.
pub fn bar(bg: Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        ..container::Style::default()
    }
}

/// The 2px Chrome-style accent strip sitting on top of a tab: the accent colour
/// for the active tab, transparent otherwise. iced's `Border` is uniform-width,
/// so a top-only accent has to be its own thin container rather than a border
/// side — this styles that container.
pub fn tab_top(active: bool, t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    let fill = if active { t.accent } else { Color::TRANSPARENT };
    move |_| container::Style {
        background: Some(fill.into()),
        ..container::Style::default()
    }
}

/// A tab's body surface, square-cornered so adjacent tabs butt together. The
/// active tab is filled with `tab_active_bg` (which equals `app_bg`) and drops
/// its border entirely, so it folds cleanly into the editor rendered directly
/// below — a bottom border would otherwise draw a line between them, and iced's
/// `Border` is uniform-width so it can't be dropped on just that one side.
/// Inactive tabs are `tabs_bg` with a hairline `tab_border` to separate them.
pub fn tab_body(active: bool, t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    let bg = if active { t.tab_active_bg } else { t.tabs_bg };
    let (border_color, border_width) = if active {
        (Color::TRANSPARENT, 0.0)
    } else {
        (t.tab_border, 1.0)
    };
    move |_| container::Style {
        background: Some(bg.into()),
        border: Border {
            color: border_color,
            width: border_width,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

/// An elevated card (find bar, About panel, confirm / quit bars, context menu,
/// tab-overflow menu): the popup background with a crisp 1px popup border and
/// rounded corners — the shared surface for every floating panel.
///
/// Deliberately **no** soft drop shadow. On the tiny-skia software renderer a
/// panel that contains hover buttons repaints just the hovered button, without
/// recomposing an overlapping *blurred* shadow, so stale shadow pixels smear over
/// the panel as the pointer moves across its controls (find / About / the menus).
/// Hover changes emit no `Message`, so the [`repaint_nudge`](crate) full-redraw
/// workaround never fires for them. The border carries the elevation instead —
/// crisp edges repaint cleanly. See the `card`-shadow note in `main.rs`'s
/// `repaint_nudge` docs for the underlying limitation.
pub fn card(t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(t.text),
        background: Some(t.popup_bg.into()),
        border: Border {
            color: t.popup_border,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

/// A status-bar / mode pill (rounded rectangle in `pill_bg`).
pub fn pill(t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(t.text),
        background: Some(t.pill_bg.into()),
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The bottom status bar — filled `status_bg` with the muted `status_fg` as its
/// default text colour (so the readouts read quieter than the toolbar).
/// Distinct from [`bar`] only in that it also sets the text
/// colour.
pub fn status_bar(t: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(t.status_fg),
        background: Some(t.status_bg.into()),
        ..container::Style::default()
    }
}

// ---- Inputs & pickers -----------------------------------------------------

/// A text input with the accent focus ring. `text_input::Style` has no `Default`,
/// so every field is explicit.
pub fn input(t: Tokens) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: Background::Color(t.input_bg),
            border: Border {
                color: if focused { t.accent } else { t.popup_border },
                width: 1.0,
                radius: 6.0.into(),
            },
            icon: t.text_muted,
            placeholder: t.text_muted,
            value: t.text,
            selection: t.accent,
        }
    }
}

/// A pick-list (font / language / encoding dropdowns) styled as a ghost-ish
/// button: button background, popup border, accent on hover / open.
pub fn picker(
    t: Tokens,
) -> impl Fn(&iced::Theme, iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list::Status;
    move |_, status| {
        let border = match status {
            Status::Hovered | Status::Opened { .. } => t.accent,
            Status::Active => t.btn_border,
        };
        iced::widget::pick_list::Style {
            text_color: t.text,
            placeholder_color: t.text_muted,
            handle_color: t.text_muted,
            background: Background::Color(t.btn_bg),
            border: Border {
                color: border,
                width: 1.0,
                radius: 6.0.into(),
            },
        }
    }
}

/// The dropdown menu a pick-list opens (fonts, language, encoding, "Reopen as…"):
/// popup background / border, accent-tinted selection — so the menu matches the
/// [`card`] surfaces rather than iced's default palette.
///
/// Like [`card`], it casts **no** soft drop shadow. On the tiny-skia software
/// renderer a blurred shadow smears: hovering a row repaints just that row without
/// recomposing the overlapping blur, so stale shadow pixels accumulate and grow as
/// the pointer moves across the menu. The 1px border carries the elevation instead
/// — crisp edges repaint cleanly.
pub fn picker_menu(t: Tokens) -> impl Fn(&iced::Theme) -> iced::widget::overlay::menu::Style {
    move |_| iced::widget::overlay::menu::Style {
        background: Background::Color(t.popup_bg),
        border: Border {
            color: t.popup_border,
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: t.text,
        selected_text_color: t.accent_fg,
        selected_background: Background::Color(t.accent),
        shadow: Shadow::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blurred drop shadow smears on the tiny-skia software renderer: hovering a
    /// control repaints it without recomposing the overlapping blur, so stale
    /// shadow pixels accumulate and grow as the pointer moves (the trap [`card`]
    /// documents). Every floating surface must stay shadow-free — the elevated
    /// cards (find / About / confirm bars / context + overflow menus) and the
    /// pick-list dropdown menus (both font pickers, language, encoding, "Reopen
    /// as…"). This fails if any regains a soft (blurred) shadow.
    #[test]
    fn floating_surfaces_cast_no_soft_shadow() {
        for t in [LIGHT, DARK] {
            assert_eq!(
                card(t)(&iced::Theme::Light).shadow.blur_radius,
                0.0,
                "card surfaces must have no soft shadow"
            );
            assert_eq!(
                picker_menu(t)(&iced::Theme::Light).shadow.blur_radius,
                0.0,
                "pick-list dropdown menus must have no soft shadow"
            );
        }
    }
}
