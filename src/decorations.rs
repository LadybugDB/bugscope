//! Client-side window decorations.
//!
//! On Linux the window requests client-side decorations (see `main.rs`): with
//! GPUI's default server-side request, compositors without xdg-decoration
//! (GNOME/Mutter on Wayland) leave the window with no frame at all while GPUI
//! still reports `Decorations::Server`. When `window.window_decorations()` is
//! `Decorations::Client`, the root view is wrapped in an invisible resize
//! border, the header becomes the drag area and gains minimize / maximize /
//! close buttons. Where the platform draws the frame (macOS, Windows, X11
//! without a compositor) every helper here is a no-op.

use crate::theme::Theme;
use gpui::prelude::FluentBuilder;
use gpui::*;

/// Width of the invisible border that starts a resize drag.
pub const RESIZE_INSET: Pixels = px(8.0);

/// `Some(tiling)` when the app has to draw its own window frame.
pub fn client_tiling(window: &Window) -> Option<Tiling> {
    match window.window_decorations() {
        Decorations::Server => None,
        Decorations::Client { tiling } => Some(tiling),
    }
}

/// Which window edge or corner a point in window coordinates is on. Tiled
/// edges (maximized, snapped) have no border and never resize.
pub fn resize_edge(
    pos: Point<Pixels>,
    size: Size<Pixels>,
    inset: Pixels,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let top = !tiling.top && pos.y < inset;
    let bottom = !tiling.bottom && pos.y > size.height - inset;
    let left = !tiling.left && pos.x < inset;
    let right = !tiling.right && pos.x > size.width - inset;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    }
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Wrap the root view in a resizable, bordered frame when decorations are
/// client-side; returns `content` unchanged otherwise.
pub fn client_frame(content: impl IntoElement, theme: Theme, window: &mut Window) -> AnyElement {
    let Some(tiling) = client_tiling(window) else {
        return content.into_any_element();
    };
    window.set_client_inset(RESIZE_INSET);
    div()
        .id("client-frame")
        .size_full()
        .bg(transparent_black())
        .child(
            // Resize cursors over the border. Painted per frame from the
            // current mouse position; `on_mouse_move` below keeps it fresh.
            canvas(
                |_bounds, window, _cx| {
                    window.insert_hitbox(
                        Bounds::new(Point::default(), window.window_bounds().get_bounds().size),
                        HitboxBehavior::Normal,
                    )
                },
                move |_bounds, hitbox, window, _cx| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) =
                        resize_edge(window.mouse_position(), size, RESIZE_INSET, tiling)
                    {
                        window.set_cursor_style(resize_cursor(edge), &hitbox);
                    }
                },
            )
            .size_full()
            .absolute(),
        )
        .when(!tiling.top, |d| d.pt(RESIZE_INSET))
        .when(!tiling.bottom, |d| d.pb(RESIZE_INSET))
        .when(!tiling.left, |d| d.pl(RESIZE_INSET))
        .when(!tiling.right, |d| d.pr(RESIZE_INSET))
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |e, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(e.position, size, RESIZE_INSET, tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(
            div()
                .size_full()
                .overflow_hidden()
                .cursor(CursorStyle::Arrow)
                .border_color(theme.border)
                .when(!tiling.top, |d| d.border_t_1())
                .when(!tiling.bottom, |d| d.border_b_1())
                .when(!tiling.left, |d| d.border_l_1())
                .when(!tiling.right, |d| d.border_r_1())
                .when(!tiling.is_tiled(), |d| {
                    d.shadow(vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.4),
                        blur_radius: RESIZE_INSET / 2.,
                        spread_radius: px(0.),
                        offset: point(px(0.), px(0.)),
                    }])
                })
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
        .into_any_element()
}

/// Make `area` move the window on drag, maximize/restore on double-click and
/// open the compositor's window menu on right-click.
pub fn drag_area(area: Div, window: &Window) -> Div {
    if client_tiling(window).is_none() {
        return area;
    }
    area.on_mouse_down(MouseButton::Left, |e, window, _| {
        if e.click_count == 2 {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |e, window, _| {
        window.show_window_menu(e.position)
    })
}

#[derive(Clone, Copy)]
enum Icon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Line icons drawn as paths (font glyphs for these differ in size and weight
/// from font to font), sized like GNOME's symbolic window-control icons.
fn paint_icon(icon: Icon, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let c = bounds.center();
    let p = |dx: f32, dy: f32| point(c.x + px(dx), c.y + px(dy));
    let mut path = PathBuilder::stroke(px(1.5));
    match icon {
        Icon::Minimize => {
            path.move_to(p(-4., 3.5));
            path.line_to(p(4., 3.5));
        }
        Icon::Maximize => {
            path.move_to(p(-4., -4.));
            path.line_to(p(4., -4.));
            path.line_to(p(4., 4.));
            path.line_to(p(-4., 4.));
            path.close();
        }
        Icon::Restore => {
            path.move_to(p(-4., -1.5));
            path.line_to(p(1.5, -1.5));
            path.line_to(p(1.5, 4.));
            path.line_to(p(-4., 4.));
            path.close();
            path.move_to(p(-1.5, -1.5));
            path.line_to(p(-1.5, -4.));
            path.line_to(p(4., -4.));
            path.line_to(p(4., 1.5));
            path.line_to(p(1.5, 1.5));
        }
        Icon::Close => {
            path.move_to(p(-4., -4.));
            path.line_to(p(4., 4.));
            path.move_to(p(4., -4.));
            path.line_to(p(-4., 4.));
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// Minimize / maximize / close buttons in the GNOME header-bar style: small
/// round buttons with a subtle fill and line icons.
pub fn window_buttons(theme: Theme, window: &Window) -> Option<Div> {
    client_tiling(window)?;
    let controls = window.window_controls();
    let button = |icon: Icon| {
        div()
            .size(px(24.))
            .rounded_full()
            .bg(theme.selection)
            .hover(|s| s.bg(theme.border))
            .cursor_pointer()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_icon(icon, bounds, theme.foreground, window),
                )
                .size_full(),
            )
    };
    let maximize = if window.is_maximized() {
        Icon::Restore
    } else {
        Icon::Maximize
    };
    Some(
        div()
            .ml_2()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .when(controls.minimize, |d| {
                d.child(
                    button(Icon::Minimize)
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.minimize_window()),
                )
            })
            .when(controls.maximize, |d| {
                d.child(
                    button(maximize)
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.zoom_window()),
                )
            })
            .child(
                button(Icon::Close)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.remove_window()),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::{resize_edge, RESIZE_INSET};
    use gpui::{point, px, Pixels, ResizeEdge, Size, Tiling};

    const SIZE: Size<Pixels> = Size {
        width: px(800.),
        height: px(600.),
    };
    const FLOATING: Tiling = Tiling {
        top: false,
        left: false,
        right: false,
        bottom: false,
    };

    fn edge_at(x: f32, y: f32, tiling: Tiling) -> Option<ResizeEdge> {
        resize_edge(point(px(x), px(y)), SIZE, RESIZE_INSET, tiling)
    }

    #[test]
    fn interior_is_not_an_edge() {
        assert_eq!(edge_at(400., 300., FLOATING), None);
        assert_eq!(edge_at(8., 8., FLOATING), None);
    }

    #[test]
    fn sides_and_corners() {
        assert_eq!(edge_at(400., 2., FLOATING), Some(ResizeEdge::Top));
        assert_eq!(edge_at(400., 598., FLOATING), Some(ResizeEdge::Bottom));
        assert_eq!(edge_at(2., 300., FLOATING), Some(ResizeEdge::Left));
        assert_eq!(edge_at(798., 300., FLOATING), Some(ResizeEdge::Right));
        assert_eq!(edge_at(2., 2., FLOATING), Some(ResizeEdge::TopLeft));
        assert_eq!(edge_at(798., 2., FLOATING), Some(ResizeEdge::TopRight));
        assert_eq!(edge_at(2., 598., FLOATING), Some(ResizeEdge::BottomLeft));
        assert_eq!(edge_at(798., 598., FLOATING), Some(ResizeEdge::BottomRight));
    }

    #[test]
    fn tiled_edges_do_not_resize() {
        assert_eq!(edge_at(2., 2., Tiling::tiled()), None);
        assert_eq!(edge_at(798., 598., Tiling::tiled()), None);
        let left_snapped = Tiling {
            top: true,
            left: true,
            bottom: true,
            right: false,
        };
        assert_eq!(edge_at(2., 300., left_snapped), None);
        assert_eq!(edge_at(798., 2., left_snapped), Some(ResizeEdge::Right));
    }
}
