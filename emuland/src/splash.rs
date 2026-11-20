//! L'écran de chargement : le nom qui apparaît, une fine barre qui se remplit, puis un fondu vers le menu.

use crate::theme::*;
use eframe::egui::{self, Color32, Id, LayerId, Order, Pos2, Rect, Vec2};

const BAR_START: f32 = 0.3;
const BAR_TIME: f32 = 1.5;
const WORD_TIME: f32 = 0.6;
const LEAVE_START: f32 = 2.0;
const LEAVE_TIME: f32 = 0.5;

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// Dessine l'écran de chargement par-dessus tout. Renvoie `true` quand il est terminé.
pub fn show(ctx: &egui::Context, t: f32) -> bool {
    if t >= LEAVE_START + LEAVE_TIME {
        return true;
    }
    ctx.request_repaint();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("splash")));

    let alpha = 1.0 - ((t - LEAVE_START) / LEAVE_TIME).clamp(0.0, 1.0);
    painter.rect_filled(screen, 0.0, fade(BG, alpha));

    let center = screen.center();
    let word_a = ease(t / WORD_TIME) * alpha;
    let galley = painter.layout_no_wrap("emuland".into(), egui::FontId::new(34.0, bold()), fade(INK, word_a));
    let size = galley.size();
    painter.galley(Pos2::new(center.x - size.x / 2.0, center.y - size.y / 2.0 - 8.0), galley, INK);

    let bar = Rect::from_center_size(Pos2::new(center.x, center.y + 34.0), Vec2::new(140.0, 1.0));
    painter.rect_filled(bar, 0.0, fade(LINE, alpha));
    let fill = ease((t - BAR_START) / BAR_TIME) * bar.width();
    if fill > 0.0 {
        painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(fill, 1.0)), 0.0, fade(INK, alpha));
    }
    false
}
