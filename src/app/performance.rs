//! The "Debug performance" window: latency bars, totals over time, memory and stage averages.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Shape,
    Stroke, StrokeKind, Vec2,
};

use super::App;
use super::chrome::{drag_window, resize_edges, window_controls};
use crate::config::Settings;
use crate::i18n::tr;
use crate::perf::{self, Group, Path, Segment, Stage};
use crate::ui::theme::Palette;
use crate::ui::widgets;

/// Colour of a stage group in the debug view (fixed, so it reads the same in every scheme).
fn group_color(group: Group) -> Color32 {
    match group {
        Group::Camera => Color32::from_rgb(200, 200, 200),
        Group::Tracker => Color32::from_rgb(86, 156, 230),
        Group::Core => Color32::from_rgb(240, 160, 64),
        Group::Interface => Color32::from_rgb(168, 140, 255),
    }
}

/// A latency segment: solid in its group colour, or hatched while waiting (labelled
/// `queue` when there is room). Camera is drawn as an outline.
fn paint_segment(painter: &egui::Painter, rect: Rect, segment: Segment, pal: &Palette, queue: Option<&str>) {
    match segment {
        Segment::Work(Group::Camera) => {
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.text), StrokeKind::Inside);
        }
        Segment::Work(group) => {
            painter.rect_filled(rect, CornerRadius::ZERO, group_color(group));
        }
        Segment::Waiting => {
            let hatch = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
            let mut x = rect.left() - rect.height();
            while x < rect.right() {
                hatch.line_segment(
                    [Pos2::new(x, rect.bottom()), Pos2::new(x + rect.height(), rect.top())],
                    Stroke::new(1.0, pal.dim),
                );
                x += 5.0;
            }
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.dim), StrokeKind::Inside);
            if let Some(queue) = queue
                && rect.width() > 46.0
            {
                let label = Rect::from_center_size(rect.center(), Vec2::new(40.0, rect.height() - 2.0));
                painter.rect_filled(label, CornerRadius::ZERO, pal.bg);
                painter.text(rect.center(), Align2::CENTER_CENTER, queue, FontId::proportional(10.5), pal.text);
            }
        }
    }
}

/// One line of `multi_graph`: (age seconds, ms) points, colour, dashed, name.
type GraphLine<'a> = (Vec<(f32, f32)>, Color32, bool, &'a str);

/// Several lines on one chart from 0 ms, newest on the right.
fn multi_graph(ui: &mut egui::Ui, pal: &Palette, lines: &[GraphLine<'_>], span: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    let hi = lines.iter().flat_map(|l| l.0.iter().map(|p| p.1)).fold(1.0f32, f32::max);
    for (points, color, dashed, _) in lines {
        if points.len() < 2 {
            continue;
        }
        let line: Vec<Pos2> = points
            .iter()
            .map(|(age, v)| {
                Pos2::new(
                    rect.right() - age / span * rect.width(),
                    rect.bottom() - 4.0 - v / hi * (rect.height() - 8.0),
                )
            })
            .collect();
        let stroke = Stroke::new(1.2, *color);
        if *dashed {
            painter.extend(Shape::dashed_line(&line, stroke, 4.0, 3.0));
        } else {
            painter.add(Shape::line(line, stroke));
        }
    }
}

/// Line chart of (age seconds, value) points, newest on the right, scaled to their range.
fn graph(ui: &mut egui::Ui, pal: &Palette, points: &[(f32, f32)], span: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    if points.len() < 2 {
        return;
    }
    let lo = points.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let hi = points.iter().map(|p| p.1).fold(f32::MIN, f32::max).max(lo + 1e-3);
    let line: Vec<Pos2> = points
        .iter()
        .map(|(age, v)| {
            Pos2::new(
                rect.right() - age / span * rect.width(),
                rect.bottom() - 4.0 - (v - lo) / (hi - lo) * (rect.height() - 8.0),
            )
        })
        .collect();
    painter.add(Shape::line(line, Stroke::new(1.2, pal.accent)));
}

impl App {
    pub(super) fn performance_window(&self, ctx: &egui::Context, s: &mut Settings) {
        let pal = self.pal;
        let perf = self.shared.perf.lock().clone();
        let mut open = s.debug;
        let language = s.language;
        let t = |key: &str| tr(language, key);
        let (one, twelve) = (Duration::from_secs(1), Duration::from_secs(12));
        let builder = egui::ViewportBuilder::default()
            .with_title(t("debug_performance"))
            .with_decorations(false)
            .with_inner_size([620.0, 780.0])
            .with_min_inner_size([480.0, 420.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("debug_performance"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                open = false;
            }
            resize_edges(ui.ctx());
            let bar = Frame::new().fill(pal.bg).inner_margin(Margin::symmetric(14, 8));
            egui::Panel::top("debug-bar").frame(bar).show(ui, |ui| {
                let maximized = drag_window(ui, "debug_drag");
                ui.horizontal(|ui| {
                    if let Some(mark) = &self.mark {
                        ui.add(egui::Image::new((mark.id(), Vec2::splat(20.0))));
                    }
                    ui.label(RichText::new(t("debug_performance")).font(FontId::new(14.0, widgets::semibold())));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if window_controls(ui, &pal, maximized) {
                            open = false;
                        }
                    });
                });
            });
            egui::CentralPanel::default().frame(Frame::new().fill(pal.bg).inner_margin(Margin::same(14))).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        // Latency bars: where each frame spends its time on the way to each result.
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("latency")).strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for (segment, key) in [
                                    (Segment::Waiting, "perf_waiting"),
                                    (Segment::Work(Group::Interface), "perf_interface"),
                                    (Segment::Work(Group::Core), "perf_core"),
                                    (Segment::Work(Group::Tracker), "perf_tracker"),
                                ] {
                                    ui.label(RichText::new(t(key)).small());
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                    paint_segment(ui.painter(), rect, segment, &pal, None);
                                }
                            });
                        });
                        let rows: Vec<(Path, Option<perf::Breakdown>)> = Path::ALL
                            .iter()
                            .map(|p| (*p, perf.paths.get(p).and_then(|series| series.average(one))))
                            .collect();
                        let longest = rows
                            .iter()
                            .filter_map(|(_, b)| b.as_ref().map(|b| b.iter().map(|(_, v)| v).sum::<f32>()))
                            .fold(0.0f32, f32::max);
                        let scale = ((longest / 10.0).ceil() * 10.0).max(30.0);
                        let (label_w, value_w) = (130.0, 64.0);
                        let (axis, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
                        let bar_x =
                            |rect: Rect, v: f32| rect.left() + label_w + v / scale * (rect.width() - label_w - value_w);
                        let ticks: Vec<f32> = (0..=(scale / 10.0) as usize).map(|i| i as f32 * 10.0).collect();
                        for &tick in &ticks {
                            ui.painter().text(
                                Pos2::new(bar_x(axis, tick), axis.center().y),
                                Align2::CENTER_CENTER,
                                format!("{tick:.0}"),
                                FontId::proportional(10.5),
                                pal.dim,
                            );
                        }
                        ui.painter().text(
                            axis.right_center(),
                            Align2::RIGHT_CENTER,
                            "ms",
                            FontId::proportional(10.5),
                            pal.dim,
                        );
                        for (path, breakdown) in &rows {
                            let (row, _) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
                            let painter = ui.painter_at(row);
                            let name = t(path.text_key());
                            painter.text(
                                row.left_center(),
                                Align2::LEFT_CENTER,
                                name,
                                FontId::proportional(11.5),
                                pal.text,
                            );
                            for &tick in &ticks {
                                painter.vline(
                                    bar_x(row, tick),
                                    row.y_range(),
                                    Stroke::new(1.0, pal.line.gamma_multiply(0.5)),
                                );
                            }
                            let Some(breakdown) = breakdown else {
                                painter.text(
                                    row.right_center(),
                                    Align2::RIGHT_CENTER,
                                    "—",
                                    FontId::proportional(11.5),
                                    pal.dim,
                                );
                                continue;
                            };
                            let mut x = bar_x(row, 0.0);
                            let mut total = 0.0;
                            for (segment, v) in breakdown {
                                total += v;
                                let end = bar_x(row, total);
                                let rect = Rect::from_x_y_ranges(x..=end, row.top() + 3.0..=row.bottom() - 3.0);
                                if rect.width() >= 0.5 {
                                    paint_segment(&painter, rect, *segment, &pal, Some(t("perf_queue")));
                                }
                                x = end;
                            }
                            let text = format!("{total:.1} ms");
                            painter.text(
                                row.right_center(),
                                Align2::RIGHT_CENTER,
                                text,
                                FontId::proportional(11.5),
                                pal.text,
                            );
                        }
                        ui.separator();

                        // Totals over time.
                        let history = |path: Path| -> Vec<(f32, f32)> {
                            perf.paths.get(&path).map(|series| series.total.history().collect()).unwrap_or_default()
                        };
                        let osc = perf.paths.get(&Path::MediaPipeOsc).and_then(|series| series.total.average(twelve));
                        ui.horizontal(|ui| {
                            ui.label(t("total_latency"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(osc.map(|v| format!("{v:.0} ms")).unwrap_or_else(|| "—".into()));
                            });
                        });
                        let lines = [
                            (history(Path::MediaPipeOsc), group_color(Group::Core), false, "OSC"),
                            (history(Path::Tables), group_color(Group::Interface), false, t("path_tables")),
                            (history(Path::Preview), pal.text, true, t("path_preview")),
                        ];
                        multi_graph(ui, &pal, &lines, 12.0);
                        ui.horizontal(|ui| {
                            for (_, color, dashed, name) in &lines {
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 10.0), Sense::hover());
                                let points = [rect.left_center(), rect.right_center()];
                                if *dashed {
                                    ui.painter().extend(Shape::dashed_line(
                                        &points,
                                        Stroke::new(1.5, *color),
                                        3.0,
                                        2.0,
                                    ));
                                } else {
                                    ui.painter().line_segment(points, Stroke::new(1.5, *color));
                                }
                                ui.label(RichText::new(*name).small());
                                ui.add_space(6.0);
                            }
                        });
                        ui.separator();

                        // Memory over the last 10 minutes.
                        let now = Instant::now();
                        let to_mib = |b: u64| b as f32 / (1024.0 * 1024.0);
                        let series = |pick: fn(&perf::MemorySample) -> u64| -> Vec<(f32, f32)> {
                            perf.memory
                                .iter()
                                .map(|(at, m)| (now.duration_since(*at).as_secs_f32(), to_mib(pick(m))))
                                .collect()
                        };
                        let private = series(|m| m.private);
                        let working = series(|m| m.working);
                        let (lo, hi) =
                            private.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
                        ui.horizontal(|ui| {
                            ui.label(t("memory"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if hi >= lo {
                                    ui.label(format!("±{:.0} MiB", (hi - lo) / 2.0));
                                }
                            });
                        });
                        graph(ui, &pal, &private, 600.0);
                        egui::Grid::new("memory-table").num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
                            ui.label("");
                            for key in ["mem_now", "mem_change", "mem_trend"] {
                                ui.label(RichText::new(t(key)).color(pal.dim).small());
                            }
                            ui.end_row();
                            for (key, points) in [("mem_private", &private), ("mem_working", &working)] {
                                ui.label(t(key));
                                let (first, last) = (points.first().map(|p| p.1), points.last().map(|p| p.1));
                                ui.label(last.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()));
                                ui.label(
                                    first
                                        .zip(last)
                                        .map(|(a, b)| format!("{:+.1}", b - a))
                                        .unwrap_or_else(|| "—".into()),
                                );
                                // Points carry their age; the trend needs time running forward, in minutes.
                                let forward: Vec<(f32, f32)> =
                                    points.iter().map(|(age, v)| (-age / 60.0, *v)).collect();
                                // The first minute is mostly start-up loading, not a trend.
                                let span = points.first().map_or(0.0, |p| p.0);
                                let trend = perf::trend_per_minute(&forward).filter(|_| span >= 60.0);
                                ui.label(trend.map(|v| format!("{v:+.2}")).unwrap_or_else(|| "—".into()));
                                ui.end_row();
                            }
                        });
                        ui.separator();

                        // Stage averages, by where they run.
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(t("averages")).color(pal.dim).small());
                            });
                        });
                        let value = |stage: Stage, over: Duration| {
                            perf.average(stage, over).map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into())
                        };
                        egui::Grid::new("stage-groups").num_columns(2).spacing([28.0, 12.0]).show(ui, |ui| {
                            for pair in Group::ALL.chunks(2) {
                                for &group in pair {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                            paint_segment(ui.painter(), rect, Segment::Work(group), &pal, None);
                                            ui.label(RichText::new(t(group.text_key())).strong());
                                        });
                                        egui::Grid::new(("stages", group)).num_columns(3).spacing([14.0, 2.0]).show(
                                            ui,
                                            |ui| {
                                                for stage in Stage::ALL.into_iter().filter(|st| st.group() == group) {
                                                    ui.label(t(stage.text_key()));
                                                    ui.label(value(stage, one));
                                                    ui.label(value(stage, twelve));
                                                    ui.end_row();
                                                }
                                            },
                                        );
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    });
                },
            );
        });
        s.debug = open;
    }
}
