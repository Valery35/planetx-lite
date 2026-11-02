// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Земляные работы»: выемка и насыпь участка до ровной площадки,
//! перенос `ui/earthwork.py` модуля. Расчёт - `data::volumes`, высоты
//! участка - [`crate::gridheights`], с открытыми моделями и своим рельефом.
//!
//! Площадка - отметка окна, «Равные объёмы» ставит отметку, при которой
//! выемка равна насыпи. Смена отметки считается без загрузки высот. Слой
//! раскраски лежит рядом с другими слоями расчёта. «OK» кладёт в «Мои
//! метки» картинку и пишет разность файлом GeoTIFF: плюс - насыпь, минус -
//! выемка. У модуля разность идёт слоем проекта QGIS.

use std::sync::Arc;

use data::hydro::{self, Grid};
use data::volumes;
use eframe::egui;
use globe_core::lod::TileKey;

use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::gridheights::{GridHeights, Step};
use crate::i18n::{Lang, tr, trf};
use crate::reliefcalc::number;

/// Сторона участка по умолчанию и наибольшая, километры.
const SIDE: f64 = 1.0;
const MOST: f64 = 20.0;
/// Пауза смены отметки перед новой раскраской, секунды.
const PAUSE: f64 = 0.15;

/// Участок для расчёта: середина, полусторона рамки, вершины.
type SiteJob = (f64, f64, f64, Vec<(f64, f64)>);

/// Рамки своего рельефа, метры Web Mercator.
pub type Boxes = [(f64, f64, f64, f64)];

/// Объём с пробелами между разрядами.
pub fn volume_text(value: f64) -> String {
    let digits = format!("{:.0}", value.abs());
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    if value < 0.0 && digits.chars().any(|c| c != '0') { format!("-{out}") } else { out }
}

/// Сводка объёмов окна.
fn summary_text(lang: Lang, out: &volumes::Summary) -> String {
    trf(
        lang,
        "Cut {cut} m³ on {cut_area} ha, fill {fill} m³ on {fill_area} ha. Fill minus cut {net} m³.",
        &[("cut", &volume_text(out.cut)), ("cut_area", &number(out.cut_area / 1e4)), ("fill", &volume_text(out.fill)), ("fill_area", &number(out.fill_area / 1e4)), ("net", &volume_text(out.net))],
    )
}

/// Оговорка об открытых высотах, пустая строка на своём рельефе.
fn warning_text(lang: Lang, work: &volumes::Earthwork, share: f64) -> String {
    if share >= volumes::OWN_ENOUGH {
        return String::new();
    }
    let error = volume_text(volumes::open_error(work, share));
    let metres = number(volumes::OPEN_ERROR);
    if share <= 0.0 {
        return trf(lang, "The heights of the area are open models with an error of metres. An error of ±{metres} m changes the volume by ±{error} m³, the result is only an estimate. A calculation needs your own terrain, a GeoTIFF elevation raster opened in the program.", &[("metres", &metres), ("error", &error)]);
    }
    trf(lang, "Your own terrain covers {share} % of the area, the rest is open models. An error of ±{metres} m changes the volume by ±{error} m³, the result is only an estimate.", &[("share", &format!("{:.0}", 100.0 * share)), ("metres", &metres), ("error", &error)])
}

/// Сетка, высоты и нынешний итог.
struct State {
    grid: Grid,
    z: Vec<f64>,
    partial: bool,
    work: Arc<volumes::Earthwork>,
    out: volumes::Summary,
    share: f64,
}

/// Окно «Земляные работы».
#[derive(Default)]
pub struct Earthwork {
    pub open: bool,
    place: String,
    status: String,
    site: Option<SiteEdit>,
    level: f64,
    job: Option<GridHeights>,
    job_ring: Vec<(f64, f64)>,
    state: Option<State>,
    /// Момент смены отметки: раскраска ждёт паузу.
    moved: Option<f64>,
    radius: f64,
    /// Слой сменился: картинки тайлов рисуются заново.
    pub changed: bool,
    /// Расчёт для «Моих меток», забирает окно глобуса.
    pub saved: Option<Saved>,
}

impl Earthwork {
    /// Окно для точки: участок - квадрат у неё.
    pub fn open_here(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        match &mut self.site {
            Some(s) => s.move_to(lat, lon),
            None => self.site = Some(SiteEdit::new(lat, lon, SIDE, MOST)),
        }
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { self.site.as_mut() } else { None }
    }

    pub fn site(&self) -> Option<&SiteEdit> {
        if self.open { self.site.as_ref() } else { None }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.moved.is_some()
    }

    /// Слой выемки и насыпи для картинок тайлов.
    pub fn painter(&self) -> Option<crate::vector::Painter> {
        let work = self.state.as_ref()?.work.clone();
        let radius = self.radius;
        Some(Arc::new(move |key: TileKey| volumes::tile_rgba(&work, key, radius)))
    }

}

impl crate::analysis::SiteTool for Earthwork {
    fn id(&self) -> &'static str {
        "earthwork"
    }
    fn shown(&self) -> bool {
        self.state.is_some()
    }
    fn painter(&self) -> Option<crate::vector::Painter> {
        Earthwork::painter(self)
    }
    fn drop_result(&mut self) {
        self.moved = None;
        if self.state.take().is_some() {
            self.changed = true;
        }
    }
}

impl Earthwork {
    /// Шаг: окно, высоты, расчёт. boxes - рамки своего рельефа. Возвращает
    /// тайлы высот, которых ждёт расчёт.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32, boxes: &Boxes, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) -> Vec<TileKey> {
        self.window(ctx, lang, now, radius, max_level);
        if let Some(moved) = self.moved {
            if now - moved >= PAUSE {
                self.moved = None;
                self.apply(lang, boxes);
            }
            ctx.request_repaint();
        }
        let Some(job) = &mut self.job else {
            return Vec::new();
        };
        let wanted = job.wanted(has);
        match job.step(lang, now, has, heights) {
            Step::Going(text) => {
                self.status = text;
                ctx.request_repaint();
            }
            Step::Failed(text) => {
                self.job = None;
                self.status = text;
            }
            Step::Done(mut z) => {
                let Some(job) = self.job.take() else {
                    return wanted;
                };
                data::site::mask_grid(&mut z, &job.grid, &std::mem::take(&mut self.job_ring), radius);
                self.built(lang, job.grid, z, job.partial, boxes);
            }
        }
        wanted
    }

    /// Высоты готовы: первая отметка - равных объёмов.
    fn built(&mut self, lang: Lang, grid: Grid, z: Vec<f64>, partial: bool, boxes: &Boxes) {
        let Some(level) = volumes::balance_level(&z) else {
            self.status = tr(lang, "There are no heights in the area.").to_string();
            return;
        };
        self.level = (level * 100.0).round() / 100.0;
        let work = Arc::new(volumes::earthwork(&z, &grid, self.level));
        self.state = Some(State { grid, z, partial, work, out: volumes::Summary::default(), share: 0.0 });
        self.apply(lang, boxes);
    }

    /// Выемка и насыпь при нынешней отметке, без загрузки высот.
    fn apply(&mut self, lang: Lang, boxes: &Boxes) {
        let Some(state) = &mut self.state else {
            return;
        };
        let work = volumes::earthwork(&state.z, &state.grid, self.level);
        state.out = volumes::summary(&work.diff, state.grid.cell.powi(2), volumes::THRESHOLD);
        state.share = volumes::own_share(&work, boxes);
        let mut parts = vec![summary_text(lang, &state.out), trf(lang, "The grid step is {step} m.", &[("step", &number(state.grid.cell))]), warning_text(lang, &work, state.share)];
        if state.partial {
            parts.push(tr(lang, "Not all heights are loaded, the computation used less detailed ones.").to_string());
        }
        parts.push(analysis::built_text(lang, crate::pointcalc::clock_now()));
        parts.retain(|p| !p.is_empty());
        self.status = parts.join(" ");
        state.work = Arc::new(work);
        self.changed = true;
    }

    fn clear(&mut self) {
        self.job = None;
        self.moved = None;
        if self.state.take().is_some() {
            self.changed = true;
        }
        self.status.clear();
    }

    fn window(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.as_mut().and_then(|s| s.shared_note(lang)) {
            self.status = text.to_string();
        }
        let mut open = true;
        let mut pressed = None;
        let mut balance = false;
        let before = self.level;
        let ready = self.state.is_some() && self.job.is_none();
        egui::Window::new(tr(lang, "Earthwork")).id(egui::Id::unique("planetx-earthwork")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            egui::Grid::new("planetx-earthwork-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                if let Some(site) = &mut self.site {
                    ui.label(tr(lang, "Area|site"));
                    site.row(ui, lang);
                    ui.end_row();
                }
                ui.label(tr(lang, "Pad elevation"));
                ui.add(egui::DragValue::new(&mut self.level).range(-500.0..=9000.0).speed(0.1).fixed_decimals(2).suffix(tr(lang, " m")))
                    .on_hover_text(tr(lang, "Elevation of the level pad above sea level. Everything above it is cut, everything below is fill. A new elevation is calculated at once, without loading heights."));
                ui.end_row();
            });
            ui.add_space(4.0);
            let hint = self.site.as_ref().and_then(|s| s.hint(lang));
            ui.add(egui::Label::new(hint.unwrap_or(&self.status)).wrap());
            ui.add_space(4.0);
            balance = ui.add_enabled(ready, egui::Button::new(tr(lang, "Equal volumes"))).on_hover_text(tr(lang, "The elevation at which the excavated soil is enough for the fill.")).clicked();
            pressed = analysis::buttons(ui, lang, ready, tr(lang, "Heights of the area, cut and fill to the pad elevation - colours on the globe and volumes. The first calculation sets the elevation of equal volumes."));
        });
        if balance && let Some(level) = self.state.as_ref().and_then(|s| volumes::balance_level(&s.z)) {
            self.level = (level * 100.0).round() / 100.0;
        }
        if self.state.is_some() && (self.level - before).abs() > 1e-9 {
            self.moved = Some(now);
        }
        match pressed {
            Some(Button::Build) => match Self::site_job(self.site.as_ref(), lang) {
                Ok((lat, lon, half, ring)) => {
                    self.radius = radius;
                    self.job_ring = ring;
                    self.job = Some(GridHeights::new(lat, lon, half, radius, max_level, now));
                }
                Err(text) => self.status = text,
            },
            Some(Button::Ok) => {
                self.save(lang);
                self.clear();
                self.open = false;
            }
            Some(Button::Cancel) => {
                self.clear();
                self.open = false;
            }
            None => {}
        }
        // Крестик - как «Отмена».
        if !open {
            self.clear();
            self.open = false;
        }
        if let Some(site) = &mut self.site
            && std::mem::take(&mut site.changed)
            && self.state.is_some()
            && self.job.is_none()
        {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
    }

    /// Участок окна для расчёта: середина, полусторона рамки и вершины.
    fn site_job(site: Option<&SiteEdit>, lang: Lang) -> Result<SiteJob, String> {
        let Some(s) = site else {
            return Err(tr(lang, "Close the area by a click on its first vertex.").to_string());
        };
        if let Some(problem) = s.problem(lang) {
            return Err(problem);
        }
        let (lat, lon) = s.site.centre();
        Ok((lat, lon, s.site.size() / 2.0, s.site.ring.clone()))
    }

    /// «OK»: разность файлом GeoTIFF и раскраска картинкой, `export`
    /// модуля. Плюс - насыпь, минус - выемка.
    fn save(&mut self, lang: Lang) {
        let radius = self.radius;
        let Some(state) = &self.state else {
            return;
        };
        let work = &state.work;
        let g = &state.grid;
        let tiff = data::geotiff::write_float_mercator(&data::site::mercator_of(g), &work.diff).unwrap_or_default();
        let (north, west) = hydro::latlon(g, -0.5, -0.5, radius);
        let (south, east) = hydro::latlon(g, g.rows as f64 - 0.5, g.cols as f64 - 0.5, radius);
        let sample = |points: &[(f64, f64)]| volumes::points_rgba(work, points, radius);
        let name = trf(lang, "Cut and fill to the elevation {level} m", &[("level", &format!("{:.2}", work.level))]);
        let mut layer = analysis::picture(name, "earthwork", (south, north, west, east), g.cell, &sample, tiff);
        let warning = warning_text(lang, work, state.share);
        layer.summary = [summary_text(lang, &state.out), warning].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        let ring = self.site.as_ref().map(|s| s.site.ring.clone()).unwrap_or_default();
        self.saved = Some(Saved { title: tr(lang, "Earthwork").to_string(), ring, layers: vec![layer], extra: Vec::new(), session: None });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    const R: f64 = 6378137.0;
    const LAT: f64 = 58.0;
    const LON: f64 = 56.25;

    /// Склон: 1 м на 100 м к востоку от отметки 100 м.
    fn slope(pts: &[(f64, f64)]) -> Vec<f64> {
        pts.iter().map(|(_, lon)| 100.0 + (lon - LON).to_radians() * R * LAT.to_radians().cos() * 0.01).collect()
    }

    fn run(e: &mut Earthwork) {
        let ctx = egui::Context::default();
        e.radius = R;
        e.job = Some(GridHeights::new(LAT, LON, 500.0, R, 14, 0.0));
        for _ in 0..1000 {
            if e.job.is_none() {
                break;
            }
            let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
                e.step(ctx, Lang::EN, 0.0, R, 14, &[], &|_| true, &slope);
            });
            out.textures_delta.clear();
        }
        assert!(e.job.is_none());
    }

    #[test]
    fn volume_groups_digits() {
        assert_eq!(volume_text(0.2), "0");
        assert_eq!(volume_text(999.0), "999");
        assert_eq!(volume_text(1234567.0), "1 234 567");
        assert_eq!(volume_text(-1000.0), "-1 000");
    }

    #[test]
    fn first_build_balances_and_level_recomputes_without_heights() {
        let mut e = Earthwork::default();
        e.open_here(LAT, LON, "точка".into());
        run(&mut e);
        // Первая отметка - равные объёмы склона.
        assert!((e.level - 100.0).abs() < 1.0, "{}", e.level);
        let first = e.state.as_ref().unwrap().out;
        assert!(first.cut > 0.0 && (first.net / first.cut).abs() < 0.02, "{first:?}");
        assert!(e.status.contains("only an estimate"), "{}", e.status);
        assert!(e.painter().is_some());
        // Новая отметка считается по тем же высотам.
        e.level += 10.0;
        e.changed = false;
        e.apply(Lang::EN, &[]);
        assert!(e.changed);
        assert!(e.state.as_ref().unwrap().out.fill > first.fill);
    }

    #[test]
    fn own_terrain_drops_the_warning() {
        let mut e = Earthwork::default();
        e.open_here(LAT, LON, "точка".into());
        run(&mut e);
        let everywhere = [(-3e7, -3e7, 3e7, 3e7)];
        e.apply(Lang::EN, &everywhere);
        assert!(!e.status.contains("only an estimate"), "{}", e.status);
    }

    #[test]
    fn ok_writes_a_raster_and_a_picture() {
        let mut e = Earthwork::default();
        e.open_here(LAT, LON, "точка".into());
        run(&mut e);
        e.save(Lang::EN);
        let saved = e.saved.take().expect("расчёт");
        assert_eq!(saved.title, "Earthwork");
        assert_eq!(saved.layers.len(), 1);
        assert!(!saved.layers[0].tiff.is_empty() && saved.layers[0].png().is_some());
        assert!(saved.layers[0].rgba.iter().any(|p| p[3] > 0));
        assert!(saved.layers[0].summary.contains("Cut"));
    }
}
