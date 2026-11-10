// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Поиск»: подсказки, места, команды.

use super::*;

impl LiteApp {
    /// Перелёт к найденному месту и метка на нём.
    pub(super) fn fly_to_found(&mut self, place: &data::geocode::Place, now: f64) {
        let camera = &self.nav.camera;
        let pose = place_pose(place, camera.fov_y, camera.aspect());
        crate::input::fly(&mut self.nav, pose, now);
        self.labels.set_search(Some((&place.name, place.lat, place.lon)));
        if let Some(check) = &mut self.check {
            check.note_search(place.lat, place.lon);
        }
    }

    /// Строка поиска места с кнопкой «Поиск», как у модуля, и найденные
    /// места под ней.
    pub(super) fn search_row(&mut self, ui: &mut egui::Ui, now: f64) {
        let button = tr(self.lang, "Search");
        let mut clicked = false;
        let mut make = false;
        let mut talk = false;
        let mut setup = false;
        let lang = self.lang;
        let edit = ui
            .horizontal(|ui| {
                // Запас - настоящая ширина кнопок на языке окна: постоянный
                // запас короче «Suchen» раздвигал панель кадр за кадром.
                let font = egui::TextStyle::Button.resolve(ui.style());
                let color = ui.visuals().text_color();
                let labels = [button, "✦"].map(|t| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), color)).size().x);
                let spacing = ui.spacing();
                let width = crate::theme::field_room(ui.available_width(), &labels, spacing.button_padding.x, spacing.item_spacing.x);
                // Поиск места - земной, на другом теле строка - только
                // для помощника. В небе строка ищет объекты неба, как
                // у модуля.
                let edit = ui.add_enabled(
                    self.planet.earth || self.sky.is_some() || self.settings.ai.search,
                    egui::TextEdit::singleline(&mut self.search_text)
                        .id(egui::Id::unique("planetx-search"))
                        .hint_text(tr(self.lang, "Voyages of Columbus"))
                        .desired_width(width.max(60.0)),
                )
                .on_hover_text(tr(self.lang, "Place, coordinates or action. A place name or coordinates, for example Perm or 58.0105, 56.2294, give a flight. A layer or a window, for example «ruler» or «grid», is found by its name and turned on from the hint. Ctrl+K from anywhere in the window puts the cursor into this line."));
                if ui.button(button).on_hover_text(tr(self.lang, "Find what is typed in the line. A place - a flight to it, several found places - a list below.")).clicked() {
                    clicked = true;
                }
                // Кнопка помощника, `Panel.make` модуля: нажатие создаёт
                // метки по описанию, правая кнопка - разговор и настройки.
                let ai = ui.button("✦").on_hover_text(tr(lang, "Create placemarks by the description in the line with the assistant, Ctrl+Enter. The right button - the conversation and the assistant settings."));
                make = ai.clicked();
                ai.context_menu(|ui| {
                    if ui.button(tr(lang, "Conversation with the assistant…")).clicked() {
                        talk = true;
                        ui.close();
                    }
                    if ui.button(tr(lang, "Assistant settings…")).clicked() {
                        setup = true;
                        ui.close();
                    }
                });
                edit
            })
            .inner;
        if talk {
            self.ai.open = true;
        }
        if setup {
            self.ai.settings_open = true;
        }
        make |= edit.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        if make {
            let text = self.search_text.clone();
            let context = self.ai_context();
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            if self.ai.generate(lang, &text, &self.settings.ai, &context, &request) {
                self.ai_answer_from = Some(self.ai.lines.len().saturating_sub(1));
            }
            return;
        }
        // Ctrl+K - в строку поиска, как у модуля.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            edit.request_focus();
        }
        // Подсказки при вводе: метки, действия, прежние запросы
        // (`scene::searchbar`, `ui/commands.py` модуля). Nominatim при
        // вводе не спрашивается, только по Enter.
        let items = scene::searchbar::drop_repeats(&scene::searchbar::suggestions(
            &self.search_text,
            &self.place_entries(),
            &self.settings.history,
            scene::searchbar::LIMIT,
            &crate::commands::actions(self.lang, &self.command_state()),
        ));
        let mut chosen: Option<scene::searchbar::Suggestion> = None;
        // Список подсказок встроен в панель под строкой, как у модуля:
        // виден, пока в нём есть строки и строка поиска в фокусе.
        if self.suggesting && !items.is_empty() {
            let lang = self.lang;
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT).id_salt("hints").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for item in &items {
                        if ui.selectable_label(false, hint_text(lang, item)).clicked() {
                            chosen = Some(item.clone());
                        }
                    }
                });
            });
        }
        self.suggesting = edit.has_focus();
        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        // Просьба словами и запрос вне Земли - помощнику, если он
        // подключён и флажок настроек стоит, `is_request` модуля.
        let ai_ready = self.settings.ai.search && self.settings.ai.ready();
        if (entered || clicked) && ai_ready && (assistant::is_request(&self.search_text) || (!self.planet.earth && self.sky.is_none())) {
            self.ask_from_search(&self.search_text.clone());
            return;
        }
        // Место не нашлось - запрос помощнику, `fly` модуля.
        if let crate::search::Status::NotFound(q) = &self.search.status
            && self.ai_fallback.as_deref() == Some(q.as_str())
        {
            self.ai_fallback = None;
            if ai_ready {
                self.ask_from_search(&self.search_text.clone());
            }
        }
        // Ответ помощника под строкой, `set_answer` модуля.
        if let Some(from) = self.ai_answer_from {
            let answer = self.ai.answer_since(from);
            let mut close = false;
            let mut write = false;
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT * 1.5).id_salt("ai-answer").show(ui, |ui| {
                    if answer.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.weak(tr(lang, "Waiting for the answer…"));
                        });
                    } else {
                        ui.label(&answer);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(text) = self.ai.proposal_text() {
                        ui.label(text);
                        write = ui.link(tr(lang, "Write")).clicked();
                    }
                    if ui.link(tr(lang, "Conversation…")).clicked() {
                        self.ai.open = true;
                    }
                    close = ui.link(tr(lang, "Close")).clicked();
                });
            });
            if write {
                self.ai.accept_proposal();
            }
            if close {
                self.ai_answer_from = None;
            }
        }
        // Поиск в небе: каталог неба, иначе служба имён Sesame.
        if (entered || clicked)
            && let Some(sky) = &mut self.sky
        {
            let loaders = &self.loaders;
            sky.search(&self.search_text, &|limit| (loaders.request)(limit));
            return;
        }
        // Enter с точным совпадением названия метки или действия -
        // подсказка, иначе запрос Nominatim, правило службы.
        // Ссылка на файл - открыть его: сцену, KML, набор 3D Tiles.
        let link = self.search_text.trim().to_string();
        if (entered || clicked) && chosen.is_none() && crate::tilesets::is_link(&link) {
            self.open_link(&link);
            return;
        }
        if entered && chosen.is_none() {
            chosen = scene::searchbar::exact(&self.search_text, &items).into_iter().find(|s| s.kind != scene::searchbar::Kind::History);
        }
        if let Some(item) = chosen {
            self.run_suggestion(ui.ctx(), &item, now);
            return;
        }
        let submitted = clicked || entered;
        if submitted {
            self.settings.history = scene::searchbar::remember(&self.settings.history, &self.search_text, scene::searchbar::HISTORY);
            self.settings_dirty = true;
        }
        // Сообщения поиска - в строку состояния, как у модуля.
        if let Some(message) = search_status(self.lang, &self.search.status)
            && self.open_message.as_deref() != Some(message.as_str())
            && self.message_seen.as_ref().is_none_or(|(m, _)| *m != message)
        {
            self.open_message = Some(message);
        }
        // Найденные места - список под строкой, когда их больше одного.
        let mut picked = None;
        if self.search.found.len() > 1 {
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT).id_salt("found").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (n, place) in self.search.found.iter().enumerate() {
                        if ui.selectable_label(false, data::geocode::place_text(place)).clicked() {
                            picked = Some(n);
                        }
                    }
                });
            });
        }
        if submitted {
            self.ai_fallback = Some(data::geocode::normalize(&self.search_text));
            if let Some(place) = self.search.submit(&self.search_text) {
                self.fly_to_found(&place, now);
            }
        }
        if let Some(place) = picked.and_then(|n| self.search.found.get(n).cloned()) {
            self.fly_to_found(&place, now);
        }
        // Строка маршрута под поиском, `set_answer` модуля.
        match self.route.ui(ui, self.lang) {
            Some(crate::route::Link::Mode(mode)) => {
                self.settings.route = mode.name().to_string();
                self.settings_dirty = true;
                self.route.set_mode(self.lang, mode, now);
            }
            Some(crate::route::Link::Fix) => ui.ctx().open_url(egui::OpenUrl::new_tab(data::routing::FIX_MAP)),
            Some(crate::route::Link::Clear) => self.route.clear(),
            None => {}
        }
    }

    /// Метки «Моих меток» для подсказок: ключ - путь через точку,
    /// название, первая точка.
    pub(super) fn place_entries(&self) -> Vec<(String, String, f64, f64)> {
        fn walk(folder: &scene::kml::Folder, path: &mut Vec<usize>, out: &mut Vec<(String, String, f64, f64)>) {
            for (i, child) in folder.children.iter().enumerate() {
                path.push(i);
                match child {
                    scene::kml::Item::Folder(f) => walk(f, path, out),
                    scene::kml::Item::Place(p) if !p.name.trim().is_empty() => {
                        let key = path.iter().map(usize::to_string).collect::<Vec<_>>().join(".");
                        let (lat, lon) = p.points.first().copied().unwrap_or((0.0, 0.0));
                        out.push((key, p.name.clone(), lat, lon));
                    }
                    _ => {}
                }
                path.pop();
            }
        }
        let mut out = Vec::new();
        if let Some(tree) = &self.marks.tree {
            walk(tree, &mut Vec::new(), &mut out);
        }
        out
    }

    /// Что включено - для приписок действий строки «Поиск».
    pub(super) fn command_state(&self) -> crate::commands::Now {
        crate::commands::Now {
            extras: self.extras_on.clone(),
            recording: self.recording.is_some(),
            time: self.timebar.extent.is_some().then_some(self.timebar.open),
            identify: self.identify,
            contours: self.planet.earth && self.vector.contours_on(),
        }
    }

    /// Выполнить подсказку строки «Поиск»: перелёт к метке, действие или
    /// прежний запрос.
    pub(super) fn run_suggestion(&mut self, ctx: &egui::Context, item: &scene::searchbar::Suggestion, now: f64) {
        match item.kind {
            scene::searchbar::Kind::Place => {
                let path: Vec<usize> = item.key.split('.').filter_map(|p| p.parse().ok()).collect();
                self.tree_state.selected = Some(path.clone());
                self.queued.push(crate::marks::Action::Fly(path));
                self.search_text = item.text.clone();
            }
            scene::searchbar::Kind::History => {
                self.search_text = item.text.clone();
                if let Some(place) = self.search.submit(&self.search_text) {
                    self.fly_to_found(&place, now);
                }
            }
            scene::searchbar::Kind::Action => {
                if let Some(command) = crate::commands::Command::from_key(&item.key) {
                    self.search_text.clear();
                    self.run_command(ctx, command, now);
                }
            }
        }
    }

    /// Действие программы из строки «Поиск».
    pub(super) fn run_command(&mut self, ctx: &egui::Context, command: crate::commands::Command, now: f64) {
        use crate::commands::Command;
        let before = self.settings.clone();
        match command {
            Command::Layer(e) => {
                let on = self.extras_on.contains(&e);
                self.set_extra(e, !on);
            }
            Command::Catalog => self.catalog.open = true,
            Command::Ruler => self.set_tool(crate::tool::Tool::ruler()),
            Command::Place => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Point)),
            Command::SaveView => self.name_dialog = Some((NameFor::View, scene::placetree::numbered_name(tr(self.lang, "View"), &self.marks.names()))),
            Command::Snapshot => self.snapshot = Some(Snapshot::Hide(SnapshotTo::File)),
            Command::CopyView => self.snapshot = Some(Snapshot::Hide(SnapshotTo::Clipboard)),
            Command::Record => self.toggle_record(now),
            Command::Time => self.timebar.open = !self.timebar.open && self.timebar.extent.is_some(),
            Command::Identify => self.identify = !self.identify,
            Command::Properties => self.settings_open = true,
            Command::Sources => self.credits.sources_open = true,
            Command::About => self.credits.about_request = true,
            Command::Events => self.events.open(&*self.loaders.request),
            Command::Tracks => self.tracks.open = true,
            #[cfg(feature = "addins")]
            Command::Addins => self.addins.open = true,
            #[cfg(not(feature = "addins"))]
            Command::Addins => {}
            Command::Assistant => self.ai.open = true,
            Command::MyLocation => self.gps.start(ctx),
            Command::ExportGltf => self.export_view(false),
            Command::ExportTiles => self.export_view(true),
            Command::ViewContours => self.globe_menu_choice(ctx, crate::globemenu::Choice::ViewContours, None, None, now),
            Command::SceneSave => {
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                self.save_marks(&folder, crate::export::Kind::Scene);
            }
            Command::SceneOpen => self.pick_file(ctx, now),
            Command::Share => self.share(ctx),
            Command::Fullscreen => {
                let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                self.set_fullscreen(ctx, !on);
            }
            Command::Demo(s, d) => {
                if let Some(demo) = crate::demos::MENU.get(s).and_then(|sec| sec.demos.get(d)) {
                    self.open_demo(demo, now);
                }
            }
        }
        if self.settings != before {
            self.settings_dirty = true;
            if self.settings.dark != before.dark {
                crate::theme::apply(ctx, self.settings.dark);
            }
        }
    }
}
