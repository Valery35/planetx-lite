// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Выемка и насыпь»: разность двух съёмок рельефа, перенос
//! `ui/volumes.py` модуля. Растр из меню - новая съёмка, в окне выбираются
//! прежняя и участок. Расчёт - `data::volumes::Surveys`, строками за кадры.
//! Итог ложится в «Мои метки» картинкой и файлом GeoTIFF: выемка красная,
//! насыпь синяя. У модуля разность идёт слоем проекта QGIS, а участок -
//! слой многоугольников. У Lite участок - общий участок окон анализа.

use std::sync::Arc;

use data::geotiff::{LatLonGrid, MercatorGrid};
use data::grid::Grid;
use data::volumes::{self, Summary, Surveys};
use eframe::egui;

use crate::analysis::{self, Saved};
use crate::i18n::{Lang, tr, trf};

/// Строк расчёта за кадр.
const ROWS: usize = 64;

/// Открытый растр: имя и байты файла.
type Raster = (String, Arc<Vec<u8>>);

fn digits(value: f64) -> String {
    crate::earthwork::volume_text(value)
}

/// Сводка объёмов.
fn summary_text(lang: Lang, out: &Summary) -> String {
    trf(
        lang,
        "Cut {cut} m³ on {cut_area} m², down to {deepest} m. Fill {fill} m³ on {fill_area} m², up to {highest} m. Fill minus cut {net} m³. Compared area {area} m².",
        &[
            ("cut", &digits(out.cut)),
            ("cut_area", &digits(out.cut_area)),
            ("deepest", &format!("{:.1}", out.deepest)),
            ("fill", &digits(out.fill)),
            ("fill_area", &digits(out.fill_area)),
            ("highest", &format!("{:.1}", out.highest)),
            ("net", &digits(out.net)),
            ("area", &digits(out.area)),
        ],
    )
}

/// Файл разности: в системе новой съёмки, если она в метрах или градусах
/// и её код известен, иначе без файла.
fn tiff_of(new: &Grid, diff: &[f64]) -> Vec<u8> {
    let t = &new.transform;
    if t[2] != 0.0 || t[4] != 0.0 || (t[1] + t[5]).abs() > 1e-9 * t[1].abs() {
        return Vec::new();
    }
    let (cols, rows) = (new.width, new.height);
    let (x0, y0) = (t[0] + t[1] / 2.0, t[3] + t[5] / 2.0);
    match new.epsg {
        Some(4326) => data::geotiff::write_float(&LatLonGrid { north: y0, west: x0, dlat: -t[5], dlon: t[1], cols, rows }, diff),
        Some(code) => u16::try_from(code).ok().and_then(|code| data::geotiff::write_float_projected(&MercatorGrid { x0, y0, step: t[1], cols, rows }, code, diff)),
        None => None,
    }
    .unwrap_or_default()
}

/// Окно «Выемка и насыпь».
pub struct VolumeCalc {
    /// Имя новой съёмки.
    name: String,
    /// Имя прежней съёмки.
    old: String,
    use_site: bool,
    threshold: f64,
    status: String,
    job: Option<Surveys>,
    /// Расчёт для «Моих меток», забирает окно глобуса.
    pub saved: Option<Saved>,
}

impl VolumeCalc {
    pub fn new(name: String) -> Self {
        Self { name, old: String::new(), use_site: false, threshold: volumes::THRESHOLD, status: String::new(), job: None, saved: None }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Окно и шаг расчёта. Ложь - окно закрыто.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, rasters: &[Raster], site: Option<&[(f64, f64)]>) -> bool {
        let mut open = true;
        let mut run = false;
        let others: Vec<&Raster> = rasters.iter().filter(|r| r.0 != self.name).collect();
        if !others.iter().any(|r| r.0 == self.old)
            && let Some(first) = others.first()
        {
            self.old = first.0.clone();
        }
        egui::Window::new(tr(lang, "Cut and fill")).id(egui::Id::unique("planetx-volumes")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            ui.add(egui::Label::new(trf(lang, "The raster «{name}» is a new terrain survey. The difference with an earlier survey gives cut and fill, their volumes and a difference raster.", &[("name", &self.name)])).wrap());
            ui.add_space(4.0);
            egui::Grid::new("planetx-volumes-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Earlier survey"));
                egui::ComboBox::from_id_salt("planetx-volumes-old").selected_text(self.old.as_str()).show_ui(ui, |ui| {
                    for r in &others {
                        ui.selectable_value(&mut self.old, r.0.clone(), r.0.as_str());
                    }
                }).response.on_hover_text(tr(lang, "A raster of an earlier survey of the same area. It is recalculated to the grid of the new survey."));
                ui.end_row();
                ui.label(tr(lang, "Area|site"));
                ui.add_enabled(site.is_some(), egui::Checkbox::new(&mut self.use_site, tr(lang, "Analysis area"))).on_hover_text(tr(lang, "Only the area shared by the analysis windows. Without the check - the whole extent of the new survey."));
                ui.end_row();
                ui.label(tr(lang, "Threshold"));
                ui.add(egui::DragValue::new(&mut self.threshold).range(0.0..=100.0).speed(0.01).fixed_decimals(2).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "A difference below the threshold is not included in the volumes, it is survey noise. A threshold above the survey error understates the volumes."));
                ui.end_row();
            });
            ui.add_space(4.0);
            let text = if others.is_empty() { tr(lang, "No other raster is open as an earlier survey.") } else { self.status.as_str() };
            ui.add(egui::Label::new(text).wrap());
            ui.add_space(4.0);
            run = ui.add_enabled(!others.is_empty() && self.job.is_none(), egui::Button::new(tr(lang, "Build"))).clicked();
        });
        if run {
            self.start(lang, rasters, site);
        }
        self.step(lang);
        if self.job.is_some() {
            ctx.request_repaint();
        }
        open
    }

    fn start(&mut self, lang: Lang, rasters: &[Raster], site: Option<&[(f64, f64)]>) {
        let read = |name: &str| -> Result<Grid, String> {
            let Some((_, bytes)) = rasters.iter().find(|r| r.0 == name) else {
                return Err(tr(lang, "The raster is no longer open.").to_string());
            };
            Grid::read(bytes.to_vec(), name)
        };
        match (read(&self.name), read(&self.old)) {
            (Ok(new), Ok(old)) => self.job = Some(Surveys::new(new, old, site.filter(|_| self.use_site).map(<[(f64, f64)]>::to_vec))),
            (Err(e), _) | (_, Err(e)) => self.status = e,
        }
    }

    /// Строки расчёта, в конце - итог.
    fn step(&mut self, lang: Lang) {
        let Some(job) = &mut self.job else {
            return;
        };
        if !job.step(ROWS) {
            self.status = trf(lang, "Computing: {done} %.", &[("done", &format!("{:.0}", 100.0 * job.done()))]);
            return;
        }
        let Some(job) = self.job.take() else {
            return;
        };
        let area = job.cell_area();
        let out = volumes::summary(&job.diff, area, self.threshold);
        if out.area <= 0.0 {
            self.status = tr(lang, "The surveys have no common cells.").to_string();
            return;
        }
        self.status = summary_text(lang, &out);
        self.saved = self.result(lang, &job, &out);
    }

    /// Разность картинкой и файлом.
    fn result(&self, lang: Lang, job: &Surveys, out: &Summary) -> Option<Saved> {
        let new = &job.new;
        let corners = new.corners()?;
        let south = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let north = corners.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max);
        let west = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
        let east = corners.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);
        let reach = out.deepest.abs().max(out.highest).max(1.0);
        let from = new.transformer_from(4326);
        let sample = |points: &[(f64, f64)]| -> Vec<[u8; 4]> {
            points
                .iter()
                .map(|&(lat, lon)| {
                    let at = match &from {
                        Some(t) => t.transform(lon, lat).ok(),
                        None => Some((lon, lat)),
                    };
                    let value = at.and_then(|(x, y)| new.pixel_of(x, y)).and_then(|(c, r)| {
                        (c >= 0.0 && r >= 0.0 && (c as usize) < new.width && (r as usize) < new.height).then(|| job.diff.get(r as usize * new.width + c as usize).copied().unwrap_or(f64::NAN))
                    });
                    volumes::color(value.unwrap_or(f64::NAN), reach)
                })
                .collect()
        };
        let cell = job.cell_area().sqrt();
        let mut layer = analysis::picture(tr(lang, "Cut and fill").to_string(), "volumes", (south, north, west, east), cell, &sample, tiff_of(new, &job.diff));
        layer.summary = summary_text(lang, out);
        Some(Saved { title: tr(lang, "Cut and fill").to_string(), ring: vec![(south, west), (north, west), (north, east), (south, east)], layers: vec![layer], extra: Vec::new(), session: None })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn raster(value: impl Fn(usize, usize) -> f64) -> Vec<u8> {
        let grid = MercatorGrid { x0: 6_000_000.0, y0: 7_000_000.0, step: 10.0, cols: 20, rows: 20 };
        let values: Vec<f64> = (0..20).flat_map(|r| (0..20).map(move |c| (r, c))).map(|(r, c)| value(r, c)).collect();
        data::geotiff::write_float_mercator(&grid, &values).unwrap()
    }

    #[test]
    fn pit_and_dump_give_cut_fill_and_a_file() {
        let old = raster(|_, _| 100.0);
        let new = raster(|r, c| if r < 5 && c < 5 { 90.0 } else if r > 14 && c > 14 { 104.0 } else { 100.0 });
        let rasters = vec![("new.tif".to_string(), Arc::new(new)), ("old.tif".to_string(), Arc::new(old))];
        let mut v = VolumeCalc::new("new.tif".into());
        v.old = "old.tif".into();
        v.start(Lang::EN, &rasters, None);
        while v.busy() {
            v.step(Lang::EN);
        }
        assert!(v.status.contains("Cut"), "{}", v.status);
        let saved = v.saved.take().expect("итог");
        assert!(!saved.layers[0].tiff.is_empty());
        assert!(saved.layers[0].rgba.iter().any(|p| p[3] > 0));
    }

    #[test]
    fn surveys_without_common_cells_say_so() {
        let empty = raster(|_, _| f64::NAN);
        let rasters = vec![("new.tif".to_string(), Arc::new(raster(|_, _| 1.0))), ("old.tif".to_string(), Arc::new(empty))];
        let mut v = VolumeCalc::new("new.tif".into());
        v.old = "old.tif".into();
        v.start(Lang::EN, &rasters, None);
        while v.busy() {
            v.step(Lang::EN);
        }
        assert!(v.saved.is_none());
        assert!(v.status.contains("no common"), "{}", v.status);
    }
}
