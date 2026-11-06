// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Помощник ИИ и сервер MCP.

use super::*;

impl LiteApp {
    /// Ретранслятор MCP: запуск и остановка по настройке, сообщения
    /// агентов - в этом кадре, по одному. Снимок вида ждёт кадра без
    /// окон, пока он не снят, другие сообщения ждут.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn step_mcp(&mut self, ctx: &egui::Context, now: f64) {
        if !self.settings.ai.mcp && !self.mcp_cli {
            self.mcp = None;
            self.mcp_failed = false;
            return;
        }
        if self.mcp.is_none() && !self.mcp_failed {
            let port = crate::mcprelay::port();
            let wake_ctx = ctx.clone();
            match crate::mcprelay::Relay::start(port, crate::mcprelay::random_token(), crate::mcprelay::token_path(port), Arc::new(move || wake_ctx.request_repaint())) {
                Ok(relay) => self.mcp = Some(relay),
                Err(e) => {
                    self.mcp_failed = true;
                    self.open_message = Some(trf(self.lang, "MCP agents are not let in: {error}", &[("error", &e)]));
                }
            }
        }
        // Снимок - когда перелёт кончился и место загрузилось, как
        // `--shot`, но не позже [`MCP_SHOT_WAIT`].
        if let Some((_, _, since, false)) = &self.mcp_shot {
            if self.idle_frames >= SHOT_IDLE_FRAMES || now - since > MCP_SHOT_WAIT {
                if self.snapshot.is_none() {
                    self.snapshot = Some(Snapshot::Hide(SnapshotTo::Mcp));
                    if let Some(shot) = &mut self.mcp_shot {
                        shot.3 = true;
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        while self.mcp_shot.is_none() {
            let Some(job) = self.mcp.as_ref().and_then(crate::mcprelay::Relay::take) else {
                break;
            };
            if assistant::mcp::is_snapshot(&job.message) {
                self.mcp_shot = Some((job.message, job.reply, now, false));
                self.idle_frames = 0;
                ctx.request_repaint();
                break;
            }
            let server = assistant::mcp::Server::new(&crate::basemap::READY.map(|r| r.key));
            let reply = server.handle(&mut McpHost { app: self, ctx, now }, &job.message);
            let _ = job.reply.send(reply);
        }
    }

    /// Пускать агентов MCP в этом запуске, `planetx-lite --mcp`, без
    /// записи в настройки.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    pub fn with_mcp(mut self) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.mcp_cli = true;
        }
        self
    }

    /// Контекст вида для модели, `assistant_context` модуля: точка
    /// взгляда, тело, включённые строки, сектор разреза, количество
    /// выделенных меток, время. Названия меток не уходят.
    pub(super) fn ai_context(&self) -> serde_json::Value {
        let p = &self.nav.pose;
        let round = |v: f64, k: f64| (v * k).round() / k;
        let mut on: Vec<String> = Vec::new();
        if self.settings.relief {
            on.push("relief".into());
        }
        on.extend(self.extras_on.iter().map(|e| e.key().to_string()));
        let mut keys = vec!["relief".to_string()];
        keys.extend(crate::layers::Extra::all().into_iter().filter(|e| !e.desktop_only() || !cfg!(target_arch = "wasm32")).map(|e| e.key().to_string()));
        let mut bodies = vec!["sky".to_string()];
        bodies.extend(scene::planets::MENU.iter().flat_map(|(_, keys)| keys.iter().map(|k| k.to_string())));
        let wedge = self.cutaway.as_ref().map(|c| [c.wedge.west, c.wedge.east, c.wedge.south, c.wedge.north].map(|v| round(v, 100.0)));
        serde_json::json!({
            "view": {"lat": round(p.lat, 1e4), "lon": round(p.lon, 1e4), "distance_km": round(p.distance / 1000.0, 10.0), "heading": round(p.heading, 10.0), "tilt": round(p.tilt, 10.0)},
            "body": if self.sky.is_some() { "sky".to_string() } else { self.planet.key().to_string() },
            "body_keys": bodies,
            "layers_on": on,
            "layer_keys": keys,
            "cutaway": wedge,
            "selected_places": usize::from(self.tree_state.selected.is_some()),
            "utc": scene::when::text(web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())),
        })
    }

    /// Окно помощника: ответы модели, вызовы инструментов, запись
    /// предложенных и созданных меток.
    pub(super) fn step_ai(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let context = self.ai_context();
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            self.ai.step(lang, now, &self.settings.ai, &context, &request);
            if self.shows_ui() && self.ai.show(ctx, lang, &mut self.settings.ai, &context, &request) {
                self.settings_dirty = true;
            }
        }
        // Вызовы без разрешения исполняются подряд, `_step` модуля.
        for _ in 0..assistant::MAX_ROUNDS * 4 {
            let Some(call) = self.ai.run.take() else {
                break;
            };
            let text = self.ai_tool(ctx, &call, now);
            let context = self.ai_context();
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            self.ai.result(lang, &text, &self.settings.ai, &context, &request);
        }
        if let Some((kml, made)) = self.ai.write.take() {
            let note = self.write_ai_kml(&kml, made);
            self.ai.say(crate::ai::Who::Note, &note);
        }
        if self.ai.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Документ помощника - новой папкой в «Мои метки», перелёт к ней.
    /// У созданного одним запросом шкала времени открывается на даты
    /// меток, `_show_made_time` модуля.
    pub(super) fn write_ai_kml(&mut self, kml: &str, made: bool) -> String {
        let lang = self.lang;
        let opened = match marks::open(kml.as_bytes(), "assistant.kml") {
            Ok(o) => o,
            Err(e) => return trf(lang, "Not pasted: {e}", &[("e", &e)]),
        };
        let count = count_places(&opened.tree);
        let mut tree = opened.tree;
        if tree.name.trim().is_empty() || tree.name == "assistant.kml" {
            tree.name = tr(lang, "Assistant").to_string();
        }
        let folder = self.tree_state.selected.clone().unwrap_or_default();
        let at = self.marks.insert(&folder, scene::kml::Item::Folder(tree), marks::root_name(lang));
        self.marks_changed();
        self.tree_state.selected = Some(at.clone());
        self.queued.push(crate::marks::Action::Fly(at));
        if made && self.timebar.extent.is_some() {
            self.timebar.open = true;
        }
        trf(lang, "Placemarks written to My Places: {count}.", &[("count", &count.to_string())])
    }

    /// Исполнить вызов инструмента модели, ответ - текст для модели,
    /// `assistant_tool` модуля. Ошибка в аргументах - тоже ответ, модель
    /// может исправиться. Тексты - для модели, по-русски, как описания
    /// инструментов.
    pub(super) fn ai_tool(&mut self, ctx: &egui::Context, call: &assistant::Call, now: f64) -> String {
        let args = &call.args;
        let num = |k: &str| args.get(k).and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());
        let text = |k: &str| args.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string();
        match call.name.as_str() {
            "fly_to" => {
                let (Some(lat), Some(lon)) = (num("lat"), num("lon")) else {
                    return "Ошибка в аргументах fly_to: нужны lat и lon.".into();
                };
                let distance = num("distance_km").unwrap_or(500.0).clamp(0.05, 50_000.0) * 1000.0;
                let (heading, tilt) = (self.nav.pose.heading, self.nav.pose.tilt);
                crate::input::fly(&mut self.nav, Pose::new(lat.clamp(-90.0, 90.0), lon, distance, heading, tilt), now);
                format!("Перелёт к {lat:.3}, {lon:.3}.")
            }
            "search_place" => {
                let query = text("query");
                if query.is_empty() {
                    return "Ошибка в аргументах search_place: нужен query.".into();
                }
                self.search_text = query.clone();
                if let Some(place) = self.search.submit(&query) {
                    self.fly_to_found(&place, now);
                }
                format!("Поиск запущен: {query}.")
            }
            "set_body" => {
                let body = text("body");
                if body == "sky" {
                    self.enter_sky();
                } else if scene::planets::MENU.iter().any(|(_, keys)| keys.contains(&body.as_str())) {
                    self.leave_sky();
                    self.set_body(&body);
                } else {
                    return format!("Неизвестное тело: {body}.");
                }
                format!("Тело: {body}.")
            }
            "set_layer" => {
                let key = text("key");
                let on = args.get("on").and_then(serde_json::Value::as_bool).unwrap_or(true);
                if key == "relief" {
                    self.settings.relief = on;
                    self.settings_dirty = true;
                } else if let Some(extra) = crate::layers::Extra::from_key(&key).filter(|e| !e.desktop_only() || !cfg!(target_arch = "wasm32")) {
                    self.set_extra(extra, on);
                } else {
                    return format!("Неизвестная строка: {key}.");
                }
                format!("Строка {key}: {}.", if on { "включена" } else { "выключена" })
            }
            "list_actions" => assistant::actions_text(&assistant::assistant_actions(&self.ai_actions())),
            "run_action" => {
                let key = text("key");
                let Some(action) = assistant::assistant_actions(&self.ai_actions()).into_iter().find(|a| a.key == key) else {
                    return format!("Нет такого действия: {key}. Ключ берётся из list_actions.");
                };
                let Some(command) = crate::commands::Command::from_key(&key) else {
                    return format!("Нет такого действия: {key}. Ключ берётся из list_actions.");
                };
                self.run_command(ctx, command, now);
                format!("Выполнено: {} - {}.", action.text, action.note)
            }
            "set_opacity" => {
                let key = text("key");
                let Some(value) = num("value") else {
                    return "Ошибка в аргументах set_opacity: нужен value.".into();
                };
                let value = value.clamp(0.0, 1.0) as f32;
                let entry = crate::layers::Extra::from_key(&key).filter(|e| self.extras_on.contains(e)).and_then(|e| match e {
                    crate::layers::Extra::Map(n) => crate::maps::entry(n),
                    _ => None,
                });
                let Some(entry) = entry else {
                    return format!("Слоя {key} нет среди включённых карт.");
                };
                self.map_opacity.insert(entry.key.to_string(), value);
                self.settings.opacity = self.map_opacity.iter().map(|(k, v)| (k.clone(), *v)).collect();
                self.settings_dirty = true;
                if self.vector.set_opacity(entry.key, value)
                    && let Some(globe) = &mut self.globe
                {
                    globe.clear_overlays();
                }
                format!("Непрозрачность {key}: {value:.2}.")
            }
            "set_time" => {
                let (start, end) = (text("start"), text("end"));
                if start.is_empty() && end.is_empty() {
                    self.timebar.open = false;
                    return "Шкала времени закрыта.".into();
                }
                let Some(extent) = self.timebar.extent else {
                    return "Шкалы времени нет: у видимых меток нет времени.".into();
                };
                let lo = if start.is_empty() { Some(extent.0) } else { scene::when::parse(&start) };
                let hi = if end.is_empty() { Some(extent.1) } else { scene::when::parse(&end) };
                let (Some(lo), Some(hi)) = (lo, hi) else {
                    return format!("Даты не разобраны: {start} - {end}.");
                };
                self.timebar.open = true;
                self.timebar.range = (lo.min(hi), lo.max(hi));
                format!("Шкала времени: {start} - {end}.")
            }
            "earth_cutaway" => {
                if args.get("off").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    self.set_extra(crate::layers::Extra::Cutaway, false);
                    return "Разрез Земли убран.".into();
                }
                if !self.planet.earth {
                    return "Разрез Земли есть только у Земли.".into();
                }
                if self.cutaway.is_none() {
                    self.set_extra(crate::layers::Extra::Cutaway, true);
                }
                if let (Some(c), Some(west), Some(east)) = (&mut self.cutaway, num("west"), num("east")) {
                    c.wedge = data::cutaway::Wedge { west, east, south: num("south").unwrap_or(0.0).clamp(-90.0, 90.0), north: num("north").unwrap_or(90.0).clamp(-90.0, 90.0) };
                }
                match &self.cutaway {
                    Some(c) => format!("Сектор: {:.1}, {:.1}, {:.1}, {:.1}.", c.wedge.west, c.wedge.east, c.wedge.south, c.wedge.north),
                    None => "Разрез не построен.".into(),
                }
            }
            "section_down" => {
                let points: Vec<(f64, f64)> = args
                    .get("points")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).filter(|(a, b)| a.is_finite() && b.is_finite()).collect())
                    .unwrap_or_default();
                if points.len() < 2 {
                    return "Для разреза нужны хотя бы две точки.".into();
                }
                let title = tr(self.lang, "Assistant section").to_string();
                match &mut self.section {
                    Some(s) => s.set_source(title, points.clone()),
                    None => self.section = Some(crate::section::SectionWindow::new(title, points.clone(), self.crust_cache().as_deref())),
                }
                format!("Разрез открыт: точек {}, очаги и строение коры в его окне.", points.len())
            }
            "point_info" => {
                let (Some(lat), Some(lon)) = (num("lat"), num("lon")) else {
                    return "Ошибка в аргументах point_info: нужны lat и lon.".into();
                };
                let objects = self.make_objects(lat, lon, None, None);
                let mut lines = vec![objects.point];
                for (group, features) in &objects.groups {
                    for (name, values) in features {
                        let fields: Vec<String> = values.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                        lines.push(format!("{group}, {name}: {}", fields.join("; ")));
                    }
                }
                lines.join("\n")
            }
            "quakes_summary" => {
                let (Some(south), Some(north), Some(west), Some(east)) = (num("south"), num("north"), num("west"), num("east")) else {
                    return "Ошибка в аргументах quakes_summary: нужны south, north, west, east.".into();
                };
                let Some(quakes) = self.dots.quakes(crate::dots::Layer::Quakes) else {
                    self.set_extra(crate::layers::Extra::Dots(crate::dots::Layer::Quakes), true);
                    return "Сводка землетрясений загружается, повтори запрос через несколько секунд.".into();
                };
                let list: Vec<assistant::Quake> = quakes.iter().map(|q| assistant::Quake { lat: q.lat, lon: q.lon, depth: q.depth, mag: q.mag, place: q.place.clone() }).collect();
                assistant::quakes_text(&list, (south, north, west, east), 3)
            }
            "add_basemap" => {
                let ready = text("ready");
                if ready.is_empty() {
                    return "Свою подложку по адресу помощник у PlanetX Lite не ставит. Готовые источники - ready.".into();
                }
                let Some(n) = crate::basemap::READY.iter().position(|r| r.key == ready) else {
                    return format!("Нет готового источника {ready}.");
                };
                self.set_basemap(Basemap::Ready(n));
                format!("Подложка поставлена: {}.", crate::basemap::READY[n].name)
            }
            "add_kml" => {
                let raw = text("kml");
                let kml = assistant::extract_kml(&raw).unwrap_or(raw);
                let tree = match scene::kml::read_kml(kml.as_bytes(), "assistant.kml") {
                    Ok(t) => t,
                    Err(e) => return format!("KML не разобран: {e:?}. Исправь документ."),
                };
                let count = count_places(&tree);
                if count == 0 {
                    return "В документе KML нет меток.".into();
                }
                let name = if tree.name.trim().is_empty() { tr(self.lang, "Assistant").to_string() } else { tree.name.clone() };
                self.ai.propose(trf(self.lang, "The assistant offers the folder «{name}», placemarks {count}.", &[("name", &name), ("count", &count.to_string())]), kml);
                format!("Документ показан пользователю, меток {count}. Запись - после его подтверждения.")
            }
            "get_kml" => {
                let p = &self.nav.pose;
                let mut out = vec![assistant::look_at_kml(p.lat, p.lon, p.distance, p.heading, p.tilt)];
                let selected = self.tree_state.selected.clone().unwrap_or_default();
                let folder = match (self.marks.folder(&selected), self.marks.place(&selected)) {
                    (Some(f), _) if !selected.is_empty() => Some(f.clone()),
                    (_, Some(place)) => {
                        let mut f = scene::kml::Folder::new(&place.name);
                        f.children.push(scene::kml::Item::Place(place.clone()));
                        Some(f)
                    }
                    _ => None,
                };
                match folder.map(|f| crate::export::tree_bytes(&f, false)) {
                    Some(Ok(bytes)) => out.push(String::from_utf8_lossy(&bytes).into_owned()),
                    _ => out.push("Выделенных меток нет.".into()),
                }
                out.join("\n")
            }
            other => format!("Нет такого инструмента: {other}."),
        }
    }

    /// Вопрос помощнику из строки «Поиск»: ответ встаёт под строкой.
    pub(super) fn ask_from_search(&mut self, text: &str) {
        let from = self.ai.lines.len();
        let context = self.ai_context();
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        if self.ai.ask(self.lang, text, &self.settings.ai, &context, &request) {
            self.ai_answer_from = Some(from);
        }
    }

    /// Действия строки «Поиск» для помощника.
    pub(super) fn ai_actions(&self) -> Vec<assistant::Action> {
        crate::commands::actions(self.lang, &self.command_state()).into_iter().map(|s| assistant::Action { key: s.key, text: s.text, note: s.note }).collect()
    }
}
