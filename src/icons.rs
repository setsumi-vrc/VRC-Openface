//! Line icons drawn with the painter on a 24 x 24 grid. They take the text color of the
//! scheme, stay sharp at any interface scale and need no font or image files.

use std::f32::consts::{PI, TAU};

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, epaint::PathShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Gear,
    Swatch,
    Pulse,
    Camera,
    Refresh,
    Mirror,
    Eye,
    Face,
    Ears,
    Undo,
    Target,
    Scan,
    Pause,
    Play,
    Search,
    ChevronDown,
    ChevronRight,
    Close,
    Info,
    Check,
    Symmetry,
    Smooth,
    Minimize,
    Maximize,
    Restore,
}

type Point = (f32, f32);

struct Canvas<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    stroke: Stroke,
    color: Color32,
}

impl Canvas<'_> {
    fn at(&self, p: Point) -> Pos2 {
        self.origin + Vec2::new(p.0, p.1) * self.scale
    }

    fn line(&self, points: &[Point]) {
        let points = points.iter().map(|&p| self.at(p)).collect();
        self.painter.add(Shape::line(points, self.stroke));
    }

    fn polygon(&self, points: &[Point], fill: bool) {
        let points = points.iter().map(|&p| self.at(p)).collect();
        let fill_color = if fill { self.color } else { Color32::TRANSPARENT };
        self.painter.add(Shape::Path(PathShape::convex_polygon(points, fill_color, self.stroke)));
    }

    fn circle(&self, center: Point, r: f32) {
        self.painter.circle_stroke(self.at(center), r * self.scale, self.stroke);
    }

    fn dot(&self, center: Point, r: f32) {
        self.painter.circle_filled(self.at(center), r * self.scale, self.color);
    }

    fn rect(&self, min: Point, max: Point, radius: f32) {
        let rect = Rect::from_min_max(self.at(min), self.at(max));
        let radius = CornerRadius::same((radius * self.scale).round() as u8);
        self.painter.rect_stroke(rect, radius, self.stroke, StrokeKind::Middle);
    }

    /// Points of an arc around `center`; angles in radians, 0 = right, growing clockwise on screen.
    fn arc(&self, center: Point, r: f32, from: f32, to: f32) -> Vec<Point> {
        let steps = 24;
        (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                (center.0 + r * a.cos(), center.1 + r * a.sin())
            })
            .collect()
    }

    /// Open arrowhead at `tip`, pointing along `dir`.
    fn arrow_head(&self, tip: Point, dir: Point, size: f32) {
        let angle = dir.1.atan2(dir.0);
        let wing = |turn: f32| {
            let a = angle + PI + turn;
            (tip.0 + size * a.cos(), tip.1 + size * a.sin())
        };
        self.line(&[wing(0.6), tip, wing(-0.6)]);
    }

    fn bezier(&self, from: Point, control: Point, to: Point) -> Vec<Point> {
        (0..=16)
            .map(|i| {
                let t = i as f32 / 16.0;
                let u = 1.0 - t;
                (
                    u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                    u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
                )
            })
            .collect()
    }
}

/// Draws `glyph` centred in `rect`, scaled to its smaller side.
pub fn paint(painter: &Painter, glyph: Glyph, rect: Rect, color: Color32) {
    let size = rect.width().min(rect.height());
    let scale = size / 24.0;
    let c = Canvas {
        painter,
        origin: rect.center() - Vec2::splat(size / 2.0),
        scale,
        stroke: Stroke::new((1.6 * scale).max(1.2), color),
        color,
    };
    match glyph {
        Glyph::Gear => {
            let mut points = Vec::new();
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                for (r, turn) in [(7.4, -0.2), (9.6, -0.13), (9.6, 0.13), (7.4, 0.2)] {
                    points.push((12.0 + r * (a + turn).cos(), 12.0 + r * (a + turn).sin()));
                }
            }
            points.push(points[0]);
            c.line(&points);
            c.circle((12.0, 12.0), 3.2);
        }
        Glyph::Swatch => {
            c.circle((12.0, 12.0), 9.0);
            for p in [(8.0, 10.0), (12.5, 7.5), (16.0, 11.0), (9.5, 15.5)] {
                c.dot(p, 1.4);
            }
        }
        Glyph::Pulse => c.line(&[(2.0, 12.0), (7.0, 12.0), (10.0, 5.0), (14.0, 19.0), (17.0, 12.0), (22.0, 12.0)]),
        Glyph::Camera => {
            c.rect((3.0, 7.0), (21.0, 19.0), 2.5);
            c.line(&[(8.0, 7.0), (9.5, 4.5), (14.5, 4.5), (16.0, 7.0)]);
            c.circle((12.0, 13.0), 3.4);
        }
        Glyph::Refresh => {
            let end_angle = 1.35 * PI;
            let arc = c.arc((12.0, 12.0), 8.0, -0.2 * PI, end_angle);
            c.line(&arc);
            c.arrow_head(arc[arc.len() - 1], (-end_angle.sin(), end_angle.cos()), 4.5);
        }
        Glyph::Mirror => {
            c.line(&[(3.0, 8.0), (21.0, 8.0)]);
            c.arrow_head((21.0, 8.0), (1.0, 0.0), 4.5);
            c.line(&[(21.0, 16.0), (3.0, 16.0)]);
            c.arrow_head((3.0, 16.0), (-1.0, 0.0), 4.5);
        }
        Glyph::Eye => {
            let mut outline = c.bezier((2.0, 12.0), (12.0, 2.0), (22.0, 12.0));
            outline.extend(c.bezier((22.0, 12.0), (12.0, 22.0), (2.0, 12.0)));
            c.line(&outline);
            c.circle((12.0, 12.0), 3.2);
        }
        Glyph::Face => {
            c.circle((12.0, 12.0), 9.0);
            c.dot((8.8, 10.0), 1.2);
            c.dot((15.2, 10.0), 1.2);
            c.line(&c.arc((12.0, 11.0), 5.0, 0.2 * PI, 0.8 * PI));
        }
        Glyph::Ears => c.polygon(
            &[(4.0, 19.0), (4.0, 4.0), (9.5, 8.5), (14.5, 8.5), (20.0, 4.0), (20.0, 19.0), (12.0, 21.0)],
            false,
        ),
        Glyph::Undo => {
            let start_angle = 1.1 * PI;
            let arc = c.arc((12.0, 13.0), 7.0, start_angle, 2.1 * PI);
            c.line(&arc);
            c.arrow_head(arc[0], (start_angle.sin(), -start_angle.cos()), 4.5);
        }
        Glyph::Target => {
            c.circle((12.0, 12.0), 6.5);
            c.dot((12.0, 12.0), 1.6);
            for (a, b) in [
                ((12.0, 2.0), (12.0, 5.0)),
                ((12.0, 19.0), (12.0, 22.0)),
                ((2.0, 12.0), (5.0, 12.0)),
                ((19.0, 12.0), (22.0, 12.0)),
            ] {
                c.line(&[a, b]);
            }
        }
        Glyph::Scan => {
            for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let (x, y) = (12.0 - sx * 9.0, 12.0 - sy * 9.0);
                c.line(&[(x + sx * 5.0, y), (x, y), (x, y + sy * 5.0)]);
            }
            c.dot((9.2, 10.5), 1.2);
            c.dot((14.8, 10.5), 1.2);
            c.line(&[(9.5, 14.5), (14.5, 14.5)]);
        }
        Glyph::Pause => {
            c.polygon(&[(7.0, 5.0), (10.0, 5.0), (10.0, 19.0), (7.0, 19.0)], true);
            c.polygon(&[(14.0, 5.0), (17.0, 5.0), (17.0, 19.0), (14.0, 19.0)], true);
        }
        Glyph::Play => c.polygon(&[(8.0, 5.0), (19.0, 12.0), (8.0, 19.0)], true),
        Glyph::Search => {
            c.circle((10.5, 10.5), 6.0);
            c.line(&[(15.0, 15.0), (20.5, 20.5)]);
        }
        Glyph::ChevronDown => c.line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Glyph::ChevronRight => c.line(&[(9.0, 6.0), (15.0, 12.0), (9.0, 18.0)]),
        Glyph::Close => {
            c.line(&[(6.0, 6.0), (18.0, 18.0)]);
            c.line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Glyph::Info => {
            c.circle((12.0, 12.0), 9.0);
            c.dot((12.0, 7.8), 1.2);
            c.line(&[(12.0, 11.0), (12.0, 17.0)]);
        }
        Glyph::Check => c.line(&[(5.0, 12.5), (10.0, 17.0), (19.0, 7.0)]),
        Glyph::Symmetry => {
            c.circle((12.0, 12.0), 8.5);
            c.line(&[(12.0, 2.0), (12.0, 22.0)]);
            c.dot((8.4, 10.0), 1.1);
            c.dot((15.6, 10.0), 1.1);
            c.line(&[(8.5, 15.0), (12.0, 16.5), (15.5, 15.0)]);
        }
        Glyph::Smooth => {
            let mut wave = c.bezier((2.0, 15.0), (7.0, 3.0), (12.0, 12.0));
            wave.extend(c.bezier((12.0, 12.0), (17.0, 21.0), (22.0, 9.0)));
            c.line(&wave);
        }
        Glyph::Minimize => c.line(&[(5.0, 12.0), (19.0, 12.0)]),
        Glyph::Maximize => c.rect((5.0, 5.0), (19.0, 19.0), 1.5),
        Glyph::Restore => {
            c.rect((4.5, 8.5), (15.5, 19.5), 1.5);
            c.line(&[(8.5, 8.5), (8.5, 5.5), (19.5, 5.5), (19.5, 16.5), (15.5, 16.5)]);
        }
    }
}
