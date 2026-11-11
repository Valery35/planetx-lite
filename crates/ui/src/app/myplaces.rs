// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Мои метки»: дерево, туры, наложения и модели, сцена и её сохранение.

use super::*;

impl LiteApp {
    /// Окно названия: «Сохранить вид», «Сохранить тур», «Сохранить
    /// измерение» модуля. Название по умолчанию с номером, объект ложится
    /// в выделенную папку или в папку выделенной метки.
    pub(super) fn name_window(&mut self, ctx: &egui::Context) {
        let Some((what, name)) = &mut self.name_dialog else {
            return;
        };
        let lang = self.lang;
        let title = match what {
            NameFor::View => tr(lang, "Save view"),
            NameFor::Tour(_) => tr(lang, "Save tour"),
            NameFor::Measure(_) => tr(lang, "Save measurement"),
        };
        let mut open = true;
        let mut done: Option<bool> = None;
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            ui.label(tr(lang, "Name"));
            let edit = ui.text_edit_singleline(name);
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        if !open {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let Some((what, name)) = self.name_dialog.take() else {
                    return;
                };
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                let name = name.trim().to_string();
                match what {
                    NameFor::View => {
                        let name = if name.is_empty() { scene::placetree::numbered_name(tr(lang, "View"), &self.marks.names()) } else { name };
                        self.add_view_place(&folder, &name);
                    }
                    NameFor::Tour(mut place) | NameFor::Measure(mut place) => {
                        if !name.is_empty() {
                            place.name = name;
                        }
                        let at = self.marks.insert(&folder, scene::kml::Item::Place(*place), marks::root_name(lang));
                        self.tree_state.selected = Some(at);
                        self.marks_changed();
                    }
                }
            }
            Some(false) => self.name_dialog = None,
            None => {}
        }
    }

    /// Метка вида камеры в папке по пути folder.
    pub(super) fn add_view_place(&mut self, folder: &[usize], name: &str) {
        let pose = &self.nav.pose;
        let mut place = scene::kml::Place::new(name, scene::features::Kind::Point, vec![(pose.lat, pose.lon)]);
        place.view = scene::lookat::make(pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt);
        let at = self.marks.insert(folder, scene::kml::Item::Place(place), marks::root_name(self.lang));
        self.tree_state.selected = Some(at);
        self.marks_changed();
    }

    /// Строка текстом KML в буфер обмена, как `copy_places` модуля: её
    /// вставляют в «Мои метки» или в Google Earth. Копия остаётся и для
    /// пункта меню «Вставить».
    pub(super) fn copy_marks(&mut self, ctx: &egui::Context, path: &[usize]) {
        let tree = self.marks.export_tree(path);
        self.copy_tree(ctx, tree);
    }

    /// Папка текстом KML в буфер обмена.
    pub(super) fn copy_tree(&mut self, ctx: &egui::Context, tree: Option<scene::kml::Folder>) {
        let text = tree.and_then(|tree| crate::export::tree_bytes(&tree, false).ok());
        if let Some(text) = text.and_then(|b| String::from_utf8(b).ok()) {
            ctx.copy_text(text.clone());
            self.clip_kml = Some(text);
            self.open_message = Some(tr(self.lang, "Copied as KML.").to_string());
        }
    }

    /// Вопрос при закрытии окна, если во «Временных метках» что-то есть,
    /// `ask_temporary` модуля. «Сохранить» переносит в «Мои метки», «Не
    /// сохранять» удаляет, «Отмена» оставляет окно. Окно страницы браузера
    /// этого сигнала не даёт, содержимое остаётся до следующего раза.
    pub(super) fn ask_temporary(&mut self, ctx: &egui::Context) {
        let ask = self.check.is_none() && self.shot.is_none() && !self.closing && self.marks.temporary_count() > 0;
        if ask && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.temporary_asking = true;
        }
        if !self.temporary_asking {
            return;
        }
        let lang = self.lang;
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("planetx-temporary-places")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.strong(tr(lang, "Temporary Places"));
            ui.label(tr(lang, "Temporary Places contain unsaved items. Save them in My Places?"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Save")).clicked() {
                    answer = Some(true);
                }
                if ui.button(tr(lang, "Don't save")).clicked() {
                    answer = Some(false);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    self.temporary_asking = false;
                }
            });
        });
        if let Some(save) = answer {
            if save {
                self.marks.save_temporary();
            } else {
                self.marks.clear_temporary();
            }
            self.reset_selection();
            self.marks_changed();
            self.temporary_asking = false;
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Удалить всё из «Моих меток» после вопроса.
    pub(super) fn clear_marks(&mut self) {
        self.reset_selection();
        self.marks.clear();
        self.shared.clear();
        self.player = None;
        self.open_message = None;
        self.marks_changed();
    }

    /// «Мои метки» изменились: картинки и модели удалённых меток
    /// забываются, шкала времени и надписи - по новому дереву.
    pub(super) fn marks_changed(&mut self) {
        // Метки, по которым режется подземная модель, могли измениться.
        if let Some(sub) = &mut self.subsurface {
            let marks = &self.marks;
            sub.places_changed(&|uid| marks.points_of(uid));
        }
        self.pictures_changed();
        if self.marks.all_overlays().is_empty() {
            self.models.clear();
        }
        self.time_extent();
        self.marks_to_labels();
        self.places_dirty = true;
    }

    /// Шаг тура: камера в позе тура, на новой остановке с датой шкала
    /// встаёт на её время, в конце тур снимается.
    pub(super) fn step_tour(&mut self, now: f64) {
        let Some(player) = &self.player else {
            return;
        };
        if player.done(now) {
            // Тур по кругу начинается с первой остановки.
            if player.looped {
                let pose = self.nav.pose.clone();
                if let Some(p) = &mut self.player {
                    p.play_from(0, &pose, now);
                }
                self.tour_index = None;
                return;
            }
            self.nav.show(player.pose(now));
            self.player = None;
            return;
        }
        let index = player.index(now);
        let time = index.and_then(|i| player.tour.stops.get(i)).and_then(|s| s.time.clone());
        self.nav.show(player.pose(now));
        if index != self.tour_index {
            self.tour_index = index;
            if let Some(time) = time {
                self.timebar.show_time(&time);
            }
        }
    }

    /// Шкала времени и управление туром сверху вида, подпись остановки
    /// снизу.
    pub(super) fn overlays(&mut self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        // Кнопка ⇆ шторки темы GIBS - на шкале, нажатия - после неё.
        self.theme_compare_bar();
        self.theme_compare_input();
        // На узком экране открытая панель занимает почти всё окно: шкала
        // ложилась поверх её строк. Показ при этом идёт дальше.
        let covered = self.panel_open && ctx.content_rect().width() < crate::timeline::ONE_ROW;
        let show_bar = self.timebar.open && self.timebar.extent.is_some() && !covered;
        if !show_bar && self.player.is_none() {
            return;
        }
        let lang = self.lang;
        let mut act: Option<TourAct> = None;
        // Шкала времени под панелью значков от её левого края, панель тура
        // под шкалой, как у модуля (`layers.py`, `tour.py`).
        let left = self.toolbar_rect.map_or(rect.left() + 8.0, |r| r.left());
        let top = self.toolbar_rect.map_or(rect.top() + 8.0, |r| r.bottom() + 8.0);
        egui::Area::new(egui::Id::unique("planetx-top")).order(egui::Order::Foreground).pivot(egui::Align2::LEFT_TOP).fixed_pos(egui::pos2(left, top)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if show_bar {
                crate::theme::plate().show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(crate::theme::pal().plate_text);
                    self.timebar.ui(ui, lang, now);
                });
            }
            if let Some(player) = &self.player {
                crate::theme::plate().show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(crate::theme::pal().plate_text);
                    ui.horizontal(|ui| {
                        use crate::timeline::{Icon, icon_button, icon_toggle};
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let duration = player.tour.duration.max(1e-9);
                        let mut share = (player.time(now) / duration) as f32;
                        ui.spacing_mut().slider_width = 220.0;
                        if ui
                            .add(egui::Slider::new(&mut share, 0.0..=1.0).show_value(false))
                            .on_hover_text(tr(lang, "How much of the tour has passed. The slider rewinds the tour. The camera stands at this point at once, the tour goes on from it."))
                            .changed()
                        {
                            act = Some(TourAct::Seek(f64::from(share)));
                        }
                        ui.label(format!("{} / {}", scene::tour::clock(player.time(now)), scene::tour::clock(player.tour.duration)));
                        if icon_button(ui, Icon::ToStart).on_hover_text(tr(lang, "Previous stop")).clicked() {
                            act = Some(TourAct::Prev);
                        }
                        let (icon, tip) = if player.paused() { (Icon::Play, tr(lang, "Continue")) } else { (Icon::Pause, tr(lang, "Pause")) };
                        if icon_button(ui, icon).on_hover_text(tip).clicked() {
                            act = Some(TourAct::Toggle);
                        }
                        if icon_button(ui, Icon::ToEnd).on_hover_text(tr(lang, "Next stop")).clicked() {
                            act = Some(TourAct::Next);
                        }
                        if icon_toggle(ui, Icon::Loop, player.looped).on_hover_text(tr(lang, "Tour in a loop. After the last stop the tour starts from the first. Pause or the cross stops it.")).clicked() {
                            act = Some(TourAct::Loop);
                        }
                        if icon_toggle(ui, Icon::Captions, player.captions).on_hover_text(tr(lang, "Stop captions on the frame. At a stop the name and the description of the placemark stand large at the bottom of the frame, in flight there is no caption.")).clicked() {
                            act = Some(TourAct::Captions);
                        }
                        if let Some((n, count)) = player.stop_number(now) {
                            let name = player.stops.get(n).map(|s| s.name.as_str()).filter(|s| !s.is_empty()).unwrap_or(tr(lang, "Untitled"));
                            ui.label(trf(lang, "Stop {n} of {count}: {name}", &[("n", &(n + 1).to_string()), ("count", &count.to_string()), ("name", name)]));
                        }
                        ui.add_space(12.0);
                        ui.label(tr(lang, "Pause"));
                        let mut pause = player.pause;
                        if ui
                            .add(egui::DragValue::new(&mut pause).range(0.0..=60.0).fixed_decimals(1).speed(0.1).suffix(tr(lang, " s")))
                            .on_hover_text(tr(lang, "How many seconds the camera stands at each stop. A new pause acts from a jump by the buttons or from a new start of the tour."))
                            .changed()
                        {
                            act = Some(TourAct::SetPause(pause));
                        }
                        // На странице папки нет, запись только в программе.
                        if cfg!(not(target_arch = "wasm32")) && self.film.is_none() && icon_button(ui, Icon::Record).on_hover_text(crate::film::hint(lang)).clicked() {
                            act = Some(TourAct::Film);
                        }
                        if let Some(film) = &self.film {
                            ui.label(trf(lang, "Recording: frame {n} of {count}", &[("n", &(film.n + 1).to_string()), ("count", &film.line.count.to_string())]));
                        }
                        if icon_button(ui, Icon::Stop).on_hover_text(tr(lang, "End tour")).clicked() {
                            act = Some(TourAct::Stop);
                        }
                    });
                    // Описание остановки под кнопками, когда подписей на
                    // кадре нет.
                    if !player.captions
                        && let Some((n, _)) = player.stop_number(now)
                        && let Some(stop) = player.stops.get(n)
                        && !stop.description.trim().is_empty()
                    {
                        ui.set_max_width(480.0f32.max(ui.min_rect().width()));
                        ui.add(egui::Label::new(caption_text(&stop.description)).wrap());
                    }
                });
            }
        });
        // Клавиши тура, как у модуля: вправо и влево - остановки, пробел -
        // пауза, Esc - конец. С Ctrl, Shift и Alt клавиши не для тура.
        if self.player.is_some() && !ctx.memory(|m| m.focused().is_some()) {
            ctx.input(|i| {
                if i.modifiers.any() {
                    return;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    act = Some(TourAct::Next);
                } else if i.key_pressed(egui::Key::ArrowLeft) {
                    act = Some(TourAct::Prev);
                } else if i.key_pressed(egui::Key::Space) {
                    act = Some(TourAct::Toggle);
                } else if i.key_pressed(egui::Key::Escape) {
                    act = Some(TourAct::Stop);
                }
            });
        }
        // Во время записи камеру ведёт запись, действует только конец тура.
        if self.film.is_some() && act != Some(TourAct::Stop) {
            act = None;
        }
        if act == Some(TourAct::Film) {
            act = None;
            self.start_film();
        }
        if let (Some(a), Some(p)) = (act, &mut self.player) {
            match a {
                TourAct::Film => {}
                TourAct::Toggle => p.toggle(now),
                TourAct::Stop => self.player = None,
                TourAct::Prev | TourAct::Next => {
                    if let Some((n, _)) = p.stop_number(now) {
                        let k = if a == TourAct::Prev { n.saturating_sub(1) } else { n + 1 };
                        let pose = self.nav.pose.clone();
                        p.play_from(k, &pose, now);
                        self.tour_index = None;
                    }
                }
                TourAct::Seek(share) => p.seek(share, now),
                TourAct::Loop => p.looped = !p.looped,
                TourAct::Captions => p.captions = !p.captions,
                TourAct::SetPause(v) => p.pause = v,
            }
        }
        self.stop_caption(ctx, rect, now);
    }

    /// Подпись остановки внизу кадра, как `StopCaption` модуля: плашка
    /// с названием крупно и описанием под ним, по середине, 44 пикселя
    /// над нижним краем.
    pub(super) fn stop_caption(&self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        let Some(player) = self.player.as_ref().filter(|p| p.captions) else {
            return;
        };
        let Some((i, alpha)) = player.tour.caption_at(player.time(now)) else {
            return;
        };
        let Some(stop) = player.tour.stops.get(i) else {
            return;
        };
        let a = alpha as f32;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("planetx-caption")));
        let width = (rect.width() * 0.6).max(360.0).min(rect.width() - 16.0);
        let pad = 14.0;
        let title = painter.layout(
            stop.name.clone(),
            egui::FontId::new(28.0, egui::FontFamily::Name(crate::fonts::SEMIBOLD.into())),
            egui::Color32::WHITE.gamma_multiply(a),
            width - 2.0 * pad,
        );
        let text = caption_text(&stop.description);
        let mut job = egui::text::LayoutJob::simple(text, egui::FontId::proportional(17.0), egui::Color32::from_rgb(225, 232, 240).gamma_multiply(a), width - 2.0 * pad);
        job.halign = egui::Align::Center;
        let desc = painter.layout_job(job);
        let gap = if desc.size().y > 0.0 { 6.0 } else { 0.0 };
        let height = title.size().y + gap + desc.size().y + 2.0 * pad;
        let box_rect = egui::Rect::from_min_size(egui::pos2(rect.center().x - width / 2.0, rect.bottom() - 44.0 - height), egui::vec2(width, height));
        painter.rect_filled(box_rect, 8.0, egui::Color32::from_rgba_unmultiplied(15, 25, 40, (175.0 * a) as u8));
        painter.galley(egui::pos2(box_rect.center().x - title.size().x / 2.0, box_rect.top() + pad), title.clone(), egui::Color32::WHITE);
        // У текста с выравниванием по середине опорная точка - середина строк.
        painter.galley(egui::pos2(box_rect.center().x, box_rect.top() + pad + title.size().y + gap), desc, egui::Color32::WHITE);
    }

    /// Точки меток - в надписи.
    pub(super) fn marks_to_labels(&mut self) {
        let points: Vec<(String, f64, f64, [u8; 4])> =
            self.marks.points().into_iter().map(|p| (p.name, p.lat, p.lon, p.icon)).collect();
        self.labels.set_marks(&points);
    }

    /// Левая панель меток: название сцены, дерево с флажками. Двойной
    /// щелчок - перелёт к метке или папке.
    pub(super) fn marks_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        // Действия, которые просили карточка и окна прошлого кадра.
        let mut actions = std::mem::take(&mut self.queued);
        let lang = self.lang;
        let title = tr(lang, "Places");
        let hint = tr(
            lang,
            "Open or drop a .planetx scene, KML, KMZ, a GeoTIFF elevation raster or a LAS, LAZ, COPC point cloud into the window. GeoPackage, Shapefile, GeoJSON, GPX, MapInfo, FlatGeobuf and DXF layers go to the My layers section.",
        );
        let layers_title = tr(lang, "Layers");
        // Во весь экран панель прячется, как у модуля.
        let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        if !self.panel_open || full {
            self.tree_actions(ui, actions, now);
            return;
        }
        // Сообщение видно MESSAGE_TIME секунд.
        let now_message = self.open_message.clone();
        match (&now_message, &self.message_seen) {
            (Some(text), Some((seen, at))) if text == seen => {
                if now - at > MESSAGE_TIME {
                    self.open_message = None;
                    self.message_seen = None;
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.5));
                }
            }
            (Some(text), _) => self.message_seen = Some((text.clone(), now)),
            (None, _) => self.message_seen = None,
        }
        let status = self.status_text();
        egui::Panel::left("marks").resizable(true).default_size(300.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.search_row(ui, now);
            ui.add_space(6.0);
            // Строка состояния внизу панели на две строки, как у модуля:
            // камера и точка под курсором.
            let status_height = 40.0;
            // Под раздел «Слои» - его высота в прошлом кадре.
            let layers_height = self.layers_height;
            if crate::theme::section(ui, "marks", title) {
                // Дереву - заданная границей высота, иначе место над
                // «Слоями». Разделам остаётся не меньше SECTION_MIN.
                let free = ui.available_height() - status_height - 34.0;
                let chosen = self.settings.places_height;
                let room = if chosen > 0.0 { chosen } else { free - layers_height };
                let room = room.min(free - SECTION_MIN).max(SECTION_MIN);
                let temporary = self.marks.temporary;
                let empty = self.marks.own_count() == 0;
                let root = self.marks.root(marks::root_name(lang));
                let shown = egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, chosen <= 0.0])
                    .show(ui, |ui| {
                        crate::marks::root_ui(ui, root, temporary, lang, &mut self.tree_state, &mut actions);
                        // Заданная высота - всё пустое место под списком.
                        let rest = if chosen > 0.0 { room - ui.min_rect().height() } else { 0.0 };
                        crate::marks::empty_ui(ui, rest.max(16.0), lang, &mut actions);
                    })
                    .inner_rect
                    .height();
                if empty {
                    ui.label(egui::RichText::new(hint).color(crate::theme::pal().muted));
                }
                self.tour_row(ui, &mut actions);
                self.splitter(ui, shown);
            }
            let top = ui.cursor().top();
            // Раздел «Проект RustGIS» - между «Метками» и «Слоями», как раздел
            // «Проект QGIS» модуля, решение автора от 5 ноября 2026 года.
            #[cfg(not(target_arch = "wasm32"))]
            self.rustgis_section(ui, lang);
            // У RustPlanetX «Моих слоёв» нет: слои ведёт проект RustGIS.
            if self.edition() == crate::edition::Edition::Lite {
                self.my_layers_section(ui, lang, now);
            }
            if crate::theme::section(ui, "layers", layers_title) {
                let mut out = Vec::new();
                let mut menu = self.layers_menu.take();
                let state = self.layers_state();
                let room = (ui.available_height() - status_height - 8.0).max(SECTION_MIN);
                egui::ScrollArea::vertical().id_salt("layers").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                    crate::layers::panel(ui, lang, &state, &mut menu, &mut out);
                });
                self.layers_menu = menu;
                self.apply_layers(out, now);
            }
            self.layers_height = ui.cursor().top() - top;
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(status).color(crate::theme::pal().muted));
            });
        });
        self.tree_actions(ui, actions, now);
    }

    /// Граница между «Моими метками» и «Слоями»: тянется мышью, как
    /// разделитель панели модуля. Двойной щелчок возвращает высоту
    /// по умолчанию. room - высота дерева в этом кадре.
    pub(super) fn splitter(&mut self, ui: &mut egui::Ui, room: f32) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let active = response.hovered() || response.dragged();
        let color = if active { crate::theme::pal().accent } else { crate::theme::pal().muted.gamma_multiply(0.4) };
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(if active { 2.0 } else { 1.0 }, color));
        if response.dragged() {
            self.settings.places_height = (room + response.drag_delta().y).clamp(SECTION_MIN, crate::settings::PLACES_MAX);
        }
        if response.drag_stopped() {
            self.settings_dirty = true;
        }
        if response.double_clicked() {
            self.settings.places_height = 0.0;
            self.settings_dirty = true;
        }
    }

    /// Строка под деревом «Моих меток», как у модуля: ползунок
    /// непрозрачности выделенного и кнопка ▶ тура.
    pub(super) fn tour_row(&mut self, ui: &mut egui::Ui, actions: &mut Vec<crate::marks::Action>) {
        let lang = self.lang;
        let selected = self.tree_state.selected.clone();
        // Выделение сменилось: цвета выделенного и содержимого папок
        // снимаются, ползунок встаёт на их непрозрачность.
        if self.opacity.as_ref().map(|o| &o.0) != selected.as_ref() {
            self.opacity = selected.as_ref().map(|path| {
                let items = self.marks.colors_of(path);
                let base = scene::opacity::selection_opacity(&items.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>());
                (path.clone(), items, base, base)
            });
        }
        let tour = self.tour_key(selected.as_deref());
        ui.horizontal(|ui| {
            let usable = self.opacity.as_ref().is_some_and(|o| !o.1.is_empty());
            let mut value = self.opacity.as_ref().filter(|_| usable).map_or(100.0, |o| (o.3 * 100.0).round());
            ui.spacing_mut().slider_width = (ui.available_width() - 40.0).max(40.0);
            let slider = ui
                .add_enabled(usable, egui::Slider::new(&mut value, 0.0..=100.0).show_value(false))
                .on_hover_text(tr(lang, "Opacity of the selected placemarks, paths, polygons, pictures and folder contents. To the left - more transparent. The value is kept in My Places and in KML."));
            if slider.changed()
                && let Some((_, items, base, current)) = &mut self.opacity
            {
                *current = value / 100.0;
                let colors: Vec<(Vec<usize>, Vec<Option<scene::Rgba>>)> =
                    items.iter().map(|(path, c)| (path.clone(), scene::opacity::scaled(c, *base, *current))).collect();
                self.marks.set_colors(&colors);
                self.marks_changed();
            }
            let tip = match &tour {
                Some(p) if p.is_empty() => tr(lang, "Tour over the checked places"),
                Some(p) if matches!(self.marks.tree.as_ref().and_then(|t| marks::item_at(t, p)), Some(scene::kml::Item::Folder(_))) => {
                    tr(lang, "Tour over the checked places of the folder")
                }
                Some(_) => tr(lang, "Tour along the selected path"),
                None => tr(lang, "Tour: select the My Places folder or a path in it"),
            };
            if ui.add_enabled(tour.is_some(), egui::Button::new("▶")).on_hover_text(tip).on_disabled_hover_text(tip).clicked()
                && let Some(path) = tour
            {
                actions.push(crate::marks::Action::Tour(path));
            }
        });
    }

    /// Что облетит кнопка тура, `_tour_key` модуля: [] - все «Мои метки»,
    /// путь папки - её метки, путь линии - вдоль неё. У метки тур идёт по
    /// её папке. None - выделения нет.
    pub(super) fn tour_key(&self, selected: Option<&[usize]>) -> Option<Vec<usize>> {
        let path = selected?;
        if path.is_empty() {
            return Some(Vec::new());
        }
        let tree = self.marks.tree.as_ref()?;
        match marks::item_at(tree, path)? {
            scene::kml::Item::Folder(_) => Some(path.to_vec()),
            scene::kml::Item::Place(p) if p.kind == scene::features::Kind::Line => Some(path.to_vec()),
            _ => self.tour_key(Some(&path[..path.len() - 1])),
        }
    }

    /// Клавиши и действия дерева «Моих меток».
    pub(super) fn tree_actions(&mut self, ui: &mut egui::Ui, mut actions: Vec<crate::marks::Action>, now: f64) {
        // Delete удаляет выделенную строку, F2 переименовывает, как
        // в списке модуля. Пока пишется текст, клавиши - для текста.
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing && let Some(path) = self.tree_state.selected.clone() {
            let picked = self.tree_state.picked();
            let many = picked.len() > 1;
            let fixed = self.marks.is_temporary(&path);
            if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                if many {
                    actions.push(crate::marks::Action::RemoveMany(picked));
                } else if !fixed {
                    actions.push(crate::marks::Action::Remove(path));
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                if !fixed {
                    actions.push(crate::marks::Action::StartRename(path));
                }
            } else if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                actions.push(if many { crate::marks::Action::CopyMany(picked) } else { crate::marks::Action::Copy(path) });
            }
        }
        // Ctrl+V: текст KML из буфера обмена - в выделенную папку, как
        // `paste_places` модуля.
        if !typing {
            let pasted = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            });
            if let Some(text) = pasted {
                self.paste_kml(&text);
            }
        }
        for action in actions {
            match action {
                crate::marks::Action::Toggled(path) => {
                    self.marks.toggled(&path);
                    let window = self.timebar.range;
                    let open = self.timebar.open;
                    self.time_extent();
                    if self.timebar.extent.is_some() {
                        self.timebar.open = open;
                        self.timebar.range = window;
                    }
                    self.marks_to_labels();
                }
                crate::marks::Action::Fly(path) => {
                    self.player = None;
                    let camera = &self.nav.camera;
                    if let Some(pose) = self.marks.pose_of(&path, camera.fov_y, camera.aspect()) {
                        crate::input::fly(&mut self.nav, pose, now);
                    }
                    if let Some(time) = self.marks.time_of(&path) {
                        self.timebar.show_time(&time);
                    }
                }
                crate::marks::Action::Tour(path) => {
                    let camera = &self.nav.camera;
                    if let Some((name, stops)) = self.marks.tour_of(&path, camera.fov_y, camera.aspect()) {
                        self.player = Player::new(&name, &self.nav.pose, stops, camera.fov_y, now);
                        self.tour_index = None;
                        self.nav.stop();
                    }
                }
                // Удаление - после вопроса, как `_confirm_remove` модуля.
                crate::marks::Action::Remove(path) => {
                    let item = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path));
                    self.confirm = match item {
                        Some(scene::kml::Item::Folder(f)) => Some((
                            tr(self.lang, "Delete folder").to_string(),
                            trf(self.lang, "Delete folder “{name}” with all its contents?", &[("name", &f.name)]),
                            Confirmed::Remove(path),
                        )),
                        Some(scene::kml::Item::Place(scene::kml::Place { name, .. }) | scene::kml::Item::Overlay(scene::kml::Overlay { name, .. })) => Some((
                            tr(self.lang, "Delete placemark").to_string(),
                            trf(self.lang, "Delete \"{name}\" from My Places?", &[("name", name)]),
                            Confirmed::Remove(path),
                        )),
                        None => None,
                    };
                }
                crate::marks::Action::Cut(path) => {
                    self.copy_marks(ui.ctx(), &path);
                    if self.marks.remove(&path) {
                        self.reset_selection();
                        self.player = None;
                        self.marks_changed();
                    }
                }
                crate::marks::Action::Paste(folder) => {
                    if let Some(text) = self.clip_kml.clone() {
                        self.paste_kml_into(&folder, &text);
                    }
                }
                crate::marks::Action::Sort(path) => {
                    if self.marks.sort(&path) {
                        self.reset_selection();
                        self.marks_changed();
                    }
                }
                crate::marks::Action::ViewSnapshot(path) => {
                    let p = &self.nav.pose;
                    if self.marks.set_view(&path, scene::lookat::make(p.lat, p.lon, p.distance, p.heading, p.tilt)) {
                        self.marks_changed();
                    }
                }
                crate::marks::Action::Add(folder, what) => {
                    use crate::marks::AddWhat;
                    self.tree_state.selected = Some(folder.clone());
                    match what {
                        AddWhat::Folder => self.queued.push(crate::marks::Action::NewFolder(folder)),
                        AddWhat::Point => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Point)),
                        AddWhat::Line => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Path)),
                        AddWhat::Polygon => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Polygon)),
                        AddWhat::RecordedTour => {
                            if self.recording.is_none() {
                                self.toggle_record(now);
                            }
                        }
                        AddWhat::Ground => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Ground, folder)),
                        AddWhat::Photo => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Photo, folder)),
                        AddWhat::Screen => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Screen, folder)),
                        AddWhat::Model => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Model, folder)),
                        AddWhat::Link => {
                            self.link_dialog = Some(crate::netlinks::LinkDialog { path: None, parent: folder, name: tr(self.lang, "Network link").to_string(), link: String::new(), refresh: 0.0 });
                        }
                    }
                }
                crate::marks::Action::OverlayProperties(path) => self.open_overlay_props(path, now),
                crate::marks::Action::LinkProperties(path) => {
                    if let Some(f) = self.marks.folder(&path) {
                        self.link_dialog = Some(crate::netlinks::LinkDialog { path: Some(path.clone()), parent: Vec::new(), name: f.name.clone(), link: f.link.clone(), refresh: f.refresh });
                    }
                }
                crate::marks::Action::ReloadLink(path) => self.netlinks.reload(&path),
                crate::marks::Action::NewFolderAfter(path) => {
                    let parent = path[..path.len().saturating_sub(1)].to_vec();
                    let index = path.last().map_or(0, |i| i + 1);
                    let name = scene::placetree::numbered_name(tr(self.lang, "New folder"), &self.marks.names());
                    let at = self.marks.insert_at(&parent, index, scene::kml::Item::Folder(scene::kml::Folder::new(&name)), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.tree_state.renaming = Some((at, name));
                    self.marks_changed();
                }
                crate::marks::Action::FolderProperties(path) => {
                    self.close_side();
                    if let Some(scene::kml::Item::Folder(f)) = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path)) {
                        self.folder_props = Some(crate::folderprops::FolderProps::new(path, f));
                    }
                }
                crate::marks::Action::TempSave => {
                    if self.marks.save_temporary() > 0 {
                        self.reset_selection();
                        self.marks_changed();
                    }
                }
                crate::marks::Action::TempClear => {
                    let count = self.marks.temporary_count();
                    if count > 0 {
                        self.confirm = Some((
                            tr(self.lang, "Clear Temporary Places…").trim_end_matches('…').to_string(),
                            trf(self.lang, "Delete the contents of Temporary Places, placemarks and folders {count}? Deletion cannot be undone.", &[("count", &count.to_string())]),
                            Confirmed::ClearTemporary,
                        ));
                    }
                }
                crate::marks::Action::RemoveMany(paths) => {
                    self.confirm = Some((
                        tr(self.lang, "Delete selected").to_string(),
                        trf(self.lang, "Delete the selected rows, {count}? Deletion cannot be undone.", &[("count", &paths.len().to_string())]),
                        Confirmed::RemoveMany(paths),
                    ));
                }
                crate::marks::Action::CopyMany(paths) => {
                    let tree = self.marks.export_many(&paths);
                    self.copy_tree(ui.ctx(), tree);
                }
                crate::marks::Action::ShowMany(paths, on) => {
                    self.marks.set_visible_many(&paths, on);
                    self.marks_changed();
                }
                crate::marks::Action::ModelSection(path) => {
                    if self.subsurface.is_none() {
                        self.open_message = Some(tr(self.lang, "No subsurface data: open a GeoPackage file with drill holes, bed roofs or tunnels.").to_string());
                    } else if let Some(place) = self.marks.place(&path) {
                        let (title, points) = (place.name.clone(), place.points.clone());
                        match &mut self.model_section {
                            Some(w) => w.set_source(title, points),
                            None => self.model_section = Some(crate::modelsection::ModelSectionWindow::new(title, points, &self.settings.section_colors)),
                        }
                    }
                }
                // Метка режет модель или ставит стенку разреза, повторный
                // выбор убирает, как у модуля.
                crate::marks::Action::ModelPart(path, part) => {
                    use crate::subsurface::PlacePart;
                    let lang = self.lang;
                    self.open_message = match (self.marks.place(&path), &mut self.subsurface) {
                        (_, None) => Some(tr(lang, "No subsurface data: open a GeoPackage file with drill holes, bed roofs or tunnels.").to_string()),
                        (None, _) => None,
                        (Some(place), Some(sub)) => {
                            let name = if place.name.is_empty() { tr(lang, "Untitled").to_string() } else { place.name.clone() };
                            let marks = &self.marks;
                            let on = sub.toggle_place(place.uid.0, part, &|uid| marks.points_of(uid));
                            let text = match (part, on) {
                                (PlacePart::Cut, true) => "Model cut along «{name}».",
                                (PlacePart::Cut, false) => "Model cut along «{name}» is removed.",
                                (PlacePart::Section, true) => "Section wall along «{name}».",
                                (PlacePart::Section, false) => "Section wall along «{name}» is removed.",
                            };
                            Some(trf(lang, text, &[("name", &name)]))
                        }
                    };
                }
                // Участок окна - рамка метки, `sentinel_place` модуля.
                crate::marks::Action::Sentinel(path) => {
                    let width = self.view_width();
                    if self.planet.earth
                        && let Some(((west, south, east, north), size)) = self.marks.place(&path).and_then(|p| data::site::ring_box(&p.points))
                    {
                        let (lat, lon) = ((south + north) / 2.0, (west + east) / 2.0);
                        let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                        self.sentinel.get_or_insert_with(|| crate::sentinel::Sentinel::new(lat, lon, width, unix)).show_place(lat, lon, size);
                    }
                }
                crate::marks::Action::Clear => {
                    let count = self.marks.own_count();
                    self.confirm = Some((
                        tr(self.lang, "Clear My Places…").trim_end_matches('…').to_string(),
                        trf(self.lang, "Delete all placemarks and folders of My Places, placemarks in all {count}? Deletion cannot be undone.", &[("count", &count.to_string())]),
                        Confirmed::Clear,
                    ));
                }
                crate::marks::Action::Open => self.pick_file(ui.ctx(), now),
                crate::marks::Action::Extrude(path) => {
                    if let Some(folder) = self.marks.folder(&path) {
                        self.extrude = Some(crate::foldercalc::ExtrudeWindow::new(path, folder));
                    }
                }
                crate::marks::Action::Sounding(path) => {
                    let level = self.marks.folder_points(&path).first().map_or(0.0, |p| self.heights.read().height_at(p.0, p.1));
                    if let Some(folder) = self.marks.folder(&path) {
                        self.sounding = Some(crate::foldercalc::SoundingWindow::new(folder, level, self.lang));
                    }
                }
                crate::marks::Action::Properties(path) => {
                    self.close_side();
                    if let Some(place) = self.marks.place(&path) {
                        self.tool = Some(crate::tool::Tool::edit(path, place));
                    }
                }
                crate::marks::Action::Profile(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        self.profile = Some(crate::profile::ProfileWindow { title: place.name.clone(), points: place.points.clone(), hovered: None, ruler: false });
                    }
                }
                crate::marks::Action::Section(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let (title, points) = (place.name.clone(), place.points.clone());
                        match &mut self.section {
                            Some(s) => s.set_source(title, points),
                            None => self.section = Some(crate::section::SectionWindow::new(title, points, self.crust_cache().as_deref())),
                        }
                    }
                }
                crate::marks::Action::Catchment(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let polygon = place.kind == scene::features::Kind::Polygon;
                        let name = if place.name.is_empty() { tr(self.lang, "Untitled").to_string() } else { place.name.clone() };
                        let points = place.points.clone();
                        let radius = self.nav.camera.ellipsoid.a;
                        let max_level = terrain_source(&self.planet).map_or(terrain::MAX_LEVEL, |s| s.max_level);
                        self.hydro.catchment_of(points, polygon, name, radius, max_level, now);
                    }
                }
                crate::marks::Action::FlowLines(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let polygon = place.kind == scene::features::Kind::Polygon;
                        let name = if place.name.is_empty() { tr(self.lang, "Untitled").to_string() } else { place.name.clone() };
                        let points = place.points.clone();
                        let radius = self.nav.camera.ellipsoid.a;
                        self.hydro.flow_from(points, polygon, name, radius);
                    }
                }
                crate::marks::Action::Copy(path) => {
                    self.copy_marks(ui.ctx(), &path);
                }
                crate::marks::Action::SaveView(path) => {
                    let name = scene::placetree::numbered_name(tr(self.lang, "View"), &self.marks.names());
                    self.add_view_place(&path, &name);
                }
                crate::marks::Action::NewFolder(path) => {
                    // «Новая папка 2», сразу на переименование, как у модуля.
                    let name = scene::placetree::numbered_name(tr(self.lang, "New folder"), &self.marks.names());
                    let at = self.marks.insert(&path, scene::kml::Item::Folder(scene::kml::Folder::new(&name)), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.tree_state.renaming = Some((at, name));
                    self.marks_changed();
                }
                crate::marks::Action::StartRename(path) => {
                    let name = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path)).map(|item| match item {
                        scene::kml::Item::Folder(f) => f.name.clone(),
                        scene::kml::Item::Place(p) => p.name.clone(),
                        scene::kml::Item::Overlay(o) => o.name.clone(),
                    });
                    self.tree_state.renaming = name.map(|n| (path, n));
                }
                crate::marks::Action::Rename(path, name) => {
                    self.tree_state.renaming = None;
                    if self.marks.rename(&path, &name) {
                        self.marks_changed();
                    }
                }
                crate::marks::Action::CancelRename => self.tree_state.renaming = None,
                crate::marks::Action::SaveKml(path) => self.save_marks(&path, crate::export::Kind::Kml),
                crate::marks::Action::SaveScene(path) => self.save_marks(&path, crate::export::Kind::Scene),
                crate::marks::Action::ToLayers(path) => self.places_to_layers(&path),
                crate::marks::Action::OverlayToLayers(path) => self.overlay_to_layers(&path),
                crate::marks::Action::Move(from, to, into) => {
                    if self.marks.move_many(&from, &to, into) {
                        self.reset_selection();
                        self.player = None;
                        self.marks_changed();
                    }
                }
            }
        }
    }

    /// Вид и время сцены, `apply` модуля: основа, рельеф и его
    /// преувеличение, промежуток шкалы времени. Чего у Lite нет, то
    /// пропускается, контракт `planetx-scene`.
    pub(super) fn apply_scene_view(&mut self, s: &scene::scene::Scene) {
        // Основа - сначала по ключу view.basemap_key, потом по названию
        // view.basemap: решение 7 `planetx-scene/DECISIONS.md`.
        let text = |key: &str| s.view.get(key).and_then(serde_json::Value::as_str);
        if self.planet.earth
            && let Some(basemap) = Basemap::from_scene(text("basemap_key"), text("basemap"))
        {
            self.set_basemap(basemap);
        }
        let relief = s.view.get("relief").and_then(serde_json::Value::as_bool);
        let scale = s.view.get("scale").and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());
        if relief.is_some() || scale.is_some() {
            if let Some(on) = relief {
                self.settings.relief = on;
            }
            if let Some(v) = scale {
                self.settings.scale = v.clamp(crate::settings::SCALE_RANGE.0, crate::settings::SCALE_RANGE.1);
            }
            self.settings_dirty = true;
            // Геоид и палеогеография держат свою высоту.
            if self.geoid.is_none() && self.paleo.is_none() {
                self.heights.write().set_scale(if self.settings.relief { self.settings.scale } else { 0.0 });
            }
        }
        // Шкала - после меток: их время входит в охват шкалы.
        if let Some((lo, hi)) = s.span() {
            self.timebar.point = lo == hi;
            self.timebar.range = (lo, hi);
            self.timebar.open = true;
        }
    }

    /// Сцена с глобуса для записи, `capture` модуля: камера, тело, вид,
    /// время и слои хозяина сцены с корнем меток root.
    pub(super) fn scene_now(&self, root: &str) -> scene::scene::Scene {
        use serde_json::{Map, Value};
        let globe_pose = self.sky.as_ref().map_or(&self.nav.pose, |sky| &sky.globe_pose);
        let mut s = crate::export::scene_at(globe_pose);
        s.body = if self.sky.is_some() { "sky".to_string() } else { self.planet.key().to_string() };
        let mut view = Map::new();
        if self.planet.earth {
            view.insert("basemap".into(), Value::String(self.basemap.name(crate::i18n::Lang::EN).to_string()));
            view.insert("basemap_key".into(), Value::String(self.basemap.key().to_string()));
        }
        view.insert("relief".into(), Value::Bool(self.settings.relief));
        view.insert("scale".into(), serde_json::json!(self.settings.scale));
        let extras: Map<String, Value> =
            self.extras.iter().chain(self.extras_on.iter()).map(|e| (e.key().to_string(), Value::Bool(self.extras_on.contains(e)))).collect();
        view.insert("extras".into(), Value::Object(extras));
        if let Some(sky) = &self.sky {
            view.insert("sky".into(), serde_json::json!([sky.view.ra.to_degrees(), sky.view.dec.to_degrees(), sky.view.fov]));
        }
        s.view = view;
        if self.timebar.open {
            s.set_span(Some(self.timebar.range));
        }
        s.layers = self.scene_layers.get(root).cloned().unwrap_or_default();
        s
    }

    /// Сохранить строку «Моих меток» по пути: KML или KMZ, или сцену
    /// с текущей камерой. Итог - в строке сообщения.
    /// Метки папки по пути - тремя слоями «Моих слоёв» в папку с её
    /// именем, «Добавить слои меток в проект» модуля. Слои - копии: правка
    /// меток их не меняет.
    pub(super) fn places_to_layers(&mut self, path: &[usize]) {
        let lang = self.lang;
        let Some(tree) = self.marks.export_tree(path) else {
            return;
        };
        let names = [tr(lang, "Placemarks - points"), tr(lang, "Placemarks - lines"), tr(lang, "Placemarks - polygons")];
        let layers = crate::datalayers::places_layers(&tree, names);
        let count = layers.len();
        // RustPlanetX: «Моих слоёв» нет, слои меток - в проект RustGIS.
        if count > 0
            && self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(&tree.name, layers.clone(), Vec::new())
        {
            self.open_message = Some(text);
            return;
        }
        for layer in layers.into_iter().rev() {
            let title = layer.name.clone();
            self.data_layers.add_to(vec![layer], &title, &tree.name);
        }
        self.open_message = Some(if count == 0 { tr(lang, "The folder has no placemarks with points.").to_string() } else { trf(lang, "Placemark layers added to «My layers», folder {folder}: {count}.", &[("folder", &tree.name), ("count", &count.to_string())]) });
    }

    /// Картинка на поверхности по пути - копией слоем-картинкой «Моих
    /// слоёв», папка «Картинки на поверхности», как «Картинки на
    /// поверхности в проект QGIS» модуля. Картинка - из архива KMZ или
    /// файлом с диска, по адресу в сети не берётся.
    pub(super) fn overlay_to_layers(&mut self, path: &[usize]) {
        let lang = self.lang;
        let Some(mut overlay) = self.marks.overlay(path).cloned() else {
            return;
        };
        if overlay.kind() != scene::overlays::Kind::Ground {
            self.open_message = Some(tr(lang, "Only a picture on the surface goes to «My layers».").to_string());
            return;
        }
        if overlay.image.as_ref().is_none_or(Vec::is_empty) {
            let local = !overlay.href.contains("://");
            overlay.image = if local { std::fs::read(&overlay.href).ok() } else { None };
        }
        if overlay.image.as_ref().is_none_or(Vec::is_empty) {
            self.open_message = Some(tr(lang, "The picture of the layer is not loaded").to_string());
            return;
        }
        // Слой несёт картинку сам, ссылка на файл ему не нужна.
        overlay.href.clear();
        let name = overlay.name.clone();
        if self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(tr(lang, "Pictures on the surface"), Vec::new(), vec![overlay.clone()])
        {
            self.open_message = Some(text);
            return;
        }
        self.data_layers.add_picture(overlay, tr(lang, "Pictures on the surface"));
        self.open_message = Some(trf(lang, "Copied to «My layers»: {name}", &[("name", &name)]));
    }

    pub(super) fn save_marks(&mut self, path: &[usize], kind: crate::export::Kind) {
        let Some(tree) = self.marks.export_tree(path) else {
            return;
        };
        let base = self.scene_now(&tree.name);
        self.open_message = match crate::export::save(kind, &tree, &base) {
            Ok(Some(to)) => Some(trf(self.lang, "Saved: {to}", &[("to", &to)])),
            Ok(None) => None,
            Err(e) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
        };
    }

    /// Окно свойств наложения по пути. У модели по ссылке пакет читается
    /// для рамки на глобусе.
    pub(super) fn open_overlay_props(&mut self, path: Vec<usize>, now: f64) {
        self.close_side();
        let Some(overlay) = self.marks.overlay(&path) else {
            return;
        };
        let mut props = crate::overlayprops::OverlayProps::new(path, overlay);
        if props.bytes.is_none()
            && let Some(package) = self.linked_package(&props.edit.href.clone(), now)
        {
            props.set_model_bytes(package.to_vec());
        }
        self.overlay_props = Some(props);
    }

    /// Наложения кадра для моделей: у модели по ссылке на файл пакет
    /// читается с диска, через промежуток обновления файл проверяется
    /// и читается заново, если изменился, как у модуля.
    pub(super) fn model_overlays(&mut self, now: f64) -> Vec<scene::kml::Overlay> {
        let mut list: Vec<scene::kml::Overlay> = self.marks.overlays().to_vec();
        list.extend(self.tracks.overlays());
        for o in list.iter_mut().filter(|o| o.kind() == scene::overlays::Kind::Model && o.image.as_ref().is_none_or(Vec::is_empty)) {
            if let Some(p) = self.linked_package(&o.href, now) {
                o.image = Some(p.to_vec());
            }
        }
        list
    }

    /// Модели по ссылке с обновлением: через промежуток файл
    /// проверяется, изменённый читается заново и модели собираются
    /// заново.
    pub(super) fn check_linked_models(&mut self, now: f64) {
        let mut stale = Vec::new();
        for o in self.marks.overlays().iter().filter(|o| o.kind() == scene::overlays::Kind::Model && o.overlay.refresh > 0.0) {
            let step = crate::netlinks::interval(o.overlay.refresh);
            if let Some(entry) = self.linked_models.get_mut(&o.href)
                && now - entry.1 >= step
            {
                entry.1 = now;
                if crate::netlinks::local_path(&o.href).and_then(|p| file_stamp_of(&p)) != entry.0 {
                    stale.push(o.href.clone());
                }
            }
        }
        if !stale.is_empty() {
            for href in stale {
                self.linked_models.remove(&href);
            }
            self.models.clear();
            self.marks.refresh();
        }
    }

    /// Окна наложения и сетевой ссылки, как `_open_overlay_properties`
    /// и `add_link` модуля.
    pub(super) fn overlay_windows(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        if let Some(props) = &mut self.overlay_props {
            let outcome = self.side.overlay.take().unwrap_or(crate::overlayprops::Outcome::Open);
            // Правки видны на глобусе сразу.
            if std::mem::take(&mut props.changed) {
                let (path, edit) = (props.path.clone(), props.edit.clone());
                self.marks.set_overlay(&path, edit);
                self.marks_changed();
            }
            match outcome {
                crate::overlayprops::Outcome::Open => {}
                crate::overlayprops::Outcome::Browse => self.pick_for(ctx, Pick::Browse),
                crate::overlayprops::Outcome::Ok => {
                    if let Some(props) = self.overlay_props.take() {
                        self.marks.set_overlay(&props.path, props.edit);
                        self.marks_changed();
                    }
                }
                crate::overlayprops::Outcome::Cancel => {
                    if let Some(props) = self.overlay_props.take() {
                        self.marks.set_overlay(&props.path, props.original);
                        self.marks_changed();
                    }
                }
            }
        }
        if let Some(dialog) = &mut self.link_dialog {
            match dialog.window(ctx, lang) {
                None => {}
                Some(false) => self.link_dialog = None,
                Some(true) => {
                    let Some(d) = self.link_dialog.take() else {
                        return;
                    };
                    let (name, link) = (d.name.trim().to_string(), d.link.trim().to_string());
                    match d.path {
                        Some(path) => {
                            let refresh = d.refresh;
                            if self.marks.edit_folder(&path, |f| {
                                if !name.is_empty() {
                                    f.name = name;
                                }
                                f.link = link;
                                f.refresh = refresh;
                            }) {
                                self.netlinks.reload(&path);
                                self.marks_changed();
                            }
                        }
                        None if !link.is_empty() => {
                            let mut folder = scene::kml::Folder::new(if name.is_empty() { &link } else { &name });
                            folder.link = link;
                            folder.refresh = d.refresh;
                            let at = self.marks.insert(&d.parent, scene::kml::Item::Folder(folder), marks::root_name(lang));
                            self.tree_state.selected = Some(at);
                            self.marks_changed();
                        }
                        None => {}
                    }
                }
            }
        }
    }
}
