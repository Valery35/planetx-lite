// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настройки, как окно «Свойства вида» модуля (`ui/properties.py`):
//! язык интерфейса, оформление, вертикальный масштаб рельефа, язык
//! подписей на глобусе. Просьба автора от 23 октября 2026 года.
//! Настройки хранятся в хранилище eframe строками «ключ=значение»,
//! чужие и испорченные строки пропускаются.

use eframe::egui;

use crate::basemap::Basemap;
use crate::i18n::{LANGUAGES, Lang, tr};

/// Ключ хранилища.
/// Свой цвет разреза модели: ключ, заливка, линия.
pub type SectionColor = (String, Option<[u8; 3]>, Option<[u8; 3]>);

pub const KEY: &str = "planetx_settings";
/// Вертикальный масштаб рельефа: пределы и шаг, как у модуля.
pub const SCALE_RANGE: (f64, f64) = (0.5, 10.0);
pub const SCALE_STEP: f64 = 0.5;
/// Предел высоты дерева меток при чтении настроек, точки экрана.
pub const PLACES_MAX: f32 = 10000.0;

/// Языки подписей сверх языков интерфейса: коды OpenMapTiles модуля.
const LABEL_EXTRA: [(&str, &str); 4] = [("pl", "Polski"), ("uk", "Українська"), ("kk", "Қазақша"), ("ar", "العربية")];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Свои цвета разреза модели, `section_colors` проекта модуля.
    pub section_colors: Vec<SectionColor>,
    /// Код языка интерфейса, None - язык системы.
    pub language: Option<String>,
    /// Язык подписей: "" - как интерфейс, "local" - местные, иначе код.
    pub labels: String,
    pub dark: bool,
    /// Вертикальный масштаб рельефа.
    pub scale: f64,
    /// Свет солнца вместо отмывки, строка «Солнце» раздела «Слои».
    pub sun: bool,
    /// Сила отмывки рельефа 0..1, окно «Свойства вида».
    pub shading: f64,
    /// Азимут света отмывки, градусы по часовой от севера.
    pub light_azimuth: f64,
    /// Строка «Координатная сетка» раздела «Слои».
    pub grid: bool,
    /// Строка «Атмосфера», у модуля включена по умолчанию.
    pub air: bool,
    /// Строка «Звёзды», у модуля включена по умолчанию.
    pub stars: bool,
    /// Флажок «Глубины морей и океанов» окна «Свойства вида»: дно моря
    /// на своих глубинах, над ним вода. У модуля включён по умолчанию.
    pub sea: bool,
    /// Флажок строки «Рельеф» раздела «Слои».
    pub relief: bool,
    /// Строки векторной основы, `VectorBase::to_text`.
    pub vector: String,
    /// Формат координат из [`COORD_FORMATS`].
    pub coords: String,
    /// Прежние запросы строки «Поиск», от новых к старым, как у модуля.
    pub history: Vec<String>,
    /// Высота дерева «Моих меток» над разделом «Слои», точки экрана.
    /// 0 - по умолчанию. Граница разделов тянется мышью, как разделитель
    /// панели модуля.
    pub places_height: f32,
    /// Включённые карты картинками, ключи [`crate::maps::ALL`].
    pub maps: Vec<String>,
    /// Ключ основы, `Basemap::key`.
    pub basemap: String,
    /// Включённые слои точек, ключи `crate::dots::Layer`.
    pub dots: Vec<String>,
    /// Способ маршрута, `data::routing::Mode::name`, `route_mode` модуля.
    pub route: String,
    /// Непрозрачность карт картинками: ключ карты и доля.
    pub opacity: Vec<(String, f32)>,
    /// «Растягивать кору» разреза Земли, у модуля включено.
    pub stretch: bool,
    /// Свои источники тайлов основы.
    pub own: Vec<crate::basemap::Own>,
    /// Свои службы WMS - карты поверх подложки.
    pub wms: Vec<crate::maps::WmsSaved>,
    /// Масштаб режима «Геоид», `geoid.SCALE` модуля.
    pub geoid_scale: f64,
    /// Свои пункты «Открыть в браузере»: название и адрес-шаблон.
    pub links: Vec<(String, String)>,
    /// Выбранные группы спутников CelesTrak.
    pub sat_groups: Vec<String>,
    /// Флажки меню «Тело» для неба: линии и названия созвездий, объекты
    /// глубокого неба, снимки неба при приближении. У модуля включены.
    pub constellations: bool,
    pub deepsky: bool,
    pub sky_imagery: bool,
    /// Помощник ИИ: подключение, адреса, модели, ключи.
    pub ai: crate::ai::AiSettings,
    /// Адреса подложек, условия которых для расчётов окон анализа приняты,
    /// `ACCEPTED_KEY` модуля с ключами «analysis:».
    pub analysis_ok: Vec<String>,
    /// Ключи подоснов с предупреждением, с условиями которых пользователь
    /// согласился, `ACCEPTED_KEY` модуля.
    pub terms_ok: Vec<String>,
    /// Ключ Earthdata пользователя для файлов NASA, `EARTHDATA_KEY`
    /// модуля. Его вводит сам пользователь в окне «Минералы EMIT».
    pub earthdata: String,
    /// Связь с открытым RustGIS ([`crate::rustgislink`]), только программа.
    pub rustgis: bool,
    /// Направление синхронизации с картой RustGIS, ключ
    /// [`crate::rustgislink::Direction`], `PlanetX/sync` модуля.
    pub sync: String,
    /// Режим «Слои как на карте RustGIS», флажок `follow_legend` окна
    /// свойств модуля. По умолчанию включён, решение автора от 8 ноября
    /// 2026 года: флажок ушёл из раздела «Проект RustGIS» в свойства.
    pub rustgis_map: bool,
    /// Слои проектов RustGIS, отмеченные на глобусе без режима карты:
    /// проект и названия слоёв, `read_shown` модуля.
    pub rustgis_shown: Vec<(String, Vec<String>)>,
}

/// Наибольшее количество проектов RustGIS в настройках глобуса: старые
/// уходят первыми.
pub const RUSTGIS_PROJECTS: usize = 50;

/// Строка настроек без знаков табуляции и перевода строки.
fn plain(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

impl Settings {
    /// Названия слоёв проекта project, отмеченных на глобусе.
    pub fn rustgis_shown_of(&self, project: &str) -> &[String] {
        self.rustgis_shown.iter().find(|(p, _)| p == project).map_or(&[], |(_, n)| n.as_slice())
    }

    pub fn set_rustgis_shown(&mut self, project: &str, names: Vec<String>) {
        self.rustgis_shown.retain(|(p, _)| p != project);
        if !names.is_empty() {
            self.rustgis_shown.push((plain(project), names.iter().map(|n| plain(n)).collect()));
        }
        let extra = self.rustgis_shown.len().saturating_sub(RUSTGIS_PROJECTS);
        self.rustgis_shown.drain(..extra);
    }
}

/// Цвет шестью знаками или пусто.
fn color_text(c: Option<[u8; 3]>) -> String {
    c.map_or(String::new(), |c| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
}

/// Цвет из шести знаков, другое - нет цвета.
fn read_color(text: &str) -> Option<[u8; 3]> {
    let part = |a: usize| text.get(a..a + 2).and_then(|t| u8::from_str_radix(t, 16).ok());
    if text.len() == 6 { Some([part(0)?, part(2)?, part(4)?]) } else { None }
}

/// Строка настроек своего цвета разреза. Ключ - код пласта из файла.
fn section_color_line(c: &SectionColor) -> String {
    let key: String = c.0.chars().filter(|c| !c.is_control()).collect();
    format!("section_color={key}\t{}\t{}\n", color_text(c.1), color_text(c.2))
}

fn read_section_color(line: &str) -> Option<SectionColor> {
    let mut parts = line.split('\t');
    let (key, fill, stroke) = (parts.next()?, parts.next()?, parts.next()?);
    Some((key.to_string(), read_color(fill), read_color(stroke)))
}

impl Default for Settings {
    fn default() -> Self {
        Self { section_colors: Vec::new(), language: None, labels: String::new(), dark: true, scale: 1.0, sun: false, shading: f64::from(render::RELIEF_STRENGTH), light_azimuth: f64::from(render::RELIEF_AZIMUTH), grid: false, air: true, stars: true, sea: true, relief: true, vector: crate::vector::VectorBase::default().to_text(), coords: "decimal".into(), history: Vec::new(), places_height: 0.0, maps: Vec::new(), basemap: "esri".into(), dots: Vec::new(), route: "car".into(), opacity: Vec::new(), stretch: true, own: Vec::new(), wms: Vec::new(), geoid_scale: data::geoid::SCALE, links: Vec::new(), sat_groups: data::satellites::DEFAULT_GROUPS.iter().map(|g| g.to_string()).collect(), constellations: true, deepsky: true, sky_imagery: true, ai: crate::ai::AiSettings::default(), analysis_ok: Vec::new(), terms_ok: Vec::new(), earthdata: String::new(), rustgis: false, sync: "both".into(), rustgis_map: true, rustgis_shown: Vec::new() }
    }
}

impl Settings {
    /// Язык интерфейса: `PLANETX_LANGUAGE`, выбранный, язык системы.
    pub fn lang(&self) -> Lang {
        Lang::from_env().or_else(|| self.language.as_deref().and_then(Lang::from_code)).unwrap_or_else(Lang::from_system)
    }

    pub fn to_text(&self) -> String {
        format!(
            "language={}\nlabels={}\ntheme={}\nscale={}\nsun={}\ngrid={}\nair={}\nstars={}\nsea={}\nrelief={}\nvector={}\ncoords={}\nplaces_height={}\nmaps={}\nbasemap={}\ndots={}\nroute={}\nopacity={}\nstretch={}\ngeoid_scale={}\n",
            self.language.as_deref().unwrap_or(""),
            self.labels,
            if self.dark { "dark" } else { "light" },
            self.scale,
            self.sun,
            self.grid,
            self.air,
            self.stars,
            self.sea,
            self.relief,
            self.vector,
            self.coords,
            self.places_height,
            self.maps.join(","),
            self.basemap,
            self.dots.join(","),
            self.route,
            self.opacity.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(","),
            self.stretch,
            self.geoid_scale
        ) + &self.history.iter().map(|h| format!("history={h}\n")).collect::<String>()
            + &self.own.iter().map(|o| format!("own={}\t{}\t{}\t{}\n", o.name, o.url, o.max_level, o.credit)).collect::<String>()
            + &self
                .wms
                .iter()
                .map(|w| format!("wms={}\t{}\t{}\t{}{}\n", w.name, w.template, w.bbox.map(|b| b.map(|v| v.to_string()).join(",")).unwrap_or_default(), w.credit, w.max_level.map_or(String::new(), |l| format!("\t{l}"))))
                .collect::<String>()
            + &format!("sat_groups={}\n", self.sat_groups.join(","))
            + &format!("constellations={}\ndeepsky={}\nsky_imagery={}\n", self.constellations, self.deepsky, self.sky_imagery)
            + &if self.links.is_empty() { String::new() } else { format!("links={}\n", data::menulinks::dump(&self.links)) }
            + &self.section_colors.iter().map(section_color_line).collect::<String>()
            + &self.ai.to_text()
            + &self.analysis_ok.iter().map(|u| format!("analysis_ok={u}\n")).collect::<String>()
            + &self.terms_ok.iter().map(|k| format!("terms_ok={k}\n")).collect::<String>()
            + &if self.earthdata.is_empty() { String::new() } else { format!("earthdata={}\n", self.earthdata) }
            + if self.rustgis { "rustgis=true\n" } else { "" }
            + &if self.sync == "both" { String::new() } else { format!("sync={}\n", self.sync) }
            + if self.rustgis_map { "" } else { "rustgis_map=false\n" }
            + &self.rustgis_shown.iter().map(|(p, names)| format!("rustgis_shown={p}\t{}\n", names.join("\t"))).collect::<String>()
            + &format!("shading={}\nlight_azimuth={}\n", self.shading, self.light_azimuth)
    }

    /// Настройки из строки хранилища. Строка считается враждебной:
    /// незнакомые ключи и значения вне пределов пропускаются.
    pub fn from_text(text: &str) -> Self {
        let mut s = Settings { section_colors: text.lines().filter_map(|l| l.strip_prefix("section_color=")).filter_map(read_section_color).collect(), ..Default::default() };
        // Свои источники - раньше основы: основа может быть своей.
        for value in text.lines().filter_map(|l| l.strip_prefix("own=")) {
            let parts: Vec<&str> = value.split('\t').collect();
            if let [name, url, level, credit] = parts.as_slice() {
                let own = crate::basemap::Own { name: name.to_string(), url: url.to_string(), max_level: level.trim().parse().unwrap_or(19), credit: credit.to_string() };
                if crate::basemap::register(&own).is_some() && !s.own.contains(&own) {
                    s.own.push(own);
                }
            }
        }
        // Свои службы WMS - раньше карт: карта может быть из них.
        for value in text.lines().filter_map(|l| l.strip_prefix("wms=")) {
            let parts = value.split('\t').collect::<Vec<&str>>();
            if let [name, template, bbox, credit, ..] = parts.as_slice() {
                let numbers: Vec<f64> = bbox.split(',').filter_map(|v| v.trim().parse().ok()).filter(|v: &f64| v.is_finite()).collect();
                let bbox = <[f64; 4]>::try_from(numbers).ok();
                let max_level = parts.get(4).and_then(|l| l.trim().parse().ok());
                let saved = crate::maps::WmsSaved { name: name.to_string(), template: template.to_string(), bbox, credit: credit.to_string(), max_level };
                if crate::maps::register_wms(&saved).is_some() && !s.wms.contains(&saved) {
                    s.wms.push(saved);
                }
            }
        }
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            let value = value.trim();
            if s.ai.read_line(key.trim(), value) {
                continue;
            }
            match key.trim() {
                "language" => s.language = Lang::from_code(value).map(|l| l.code().to_string()),
                "labels" => {
                    if value.is_empty() || value == data::places::LOCAL || data::places::LABEL_LANGUAGES.contains(&value) {
                        s.labels = value.to_string();
                    }
                }
                "theme" => s.dark = value != "light",
                "sun" => s.sun = value == "true",
                "grid" => s.grid = value == "true",
                "air" => s.air = value != "false",
                "stars" => s.stars = value != "false",
                "sea" => s.sea = value != "false",
                "stretch" => s.stretch = value != "false",
                "geoid_scale" => {
                    if let Ok(v) = value.parse::<f64>()
                        && data::geoid::SCALES.contains(&v)
                    {
                        s.geoid_scale = v;
                    }
                }
                "relief" => s.relief = value != "false",
                "constellations" => s.constellations = value != "false",
                "deepsky" => s.deepsky = value != "false",
                "rustgis" => s.rustgis = value == "true",
                "sync" if matches!(value, "both" | "map" | "globe") => s.sync = value.to_string(),
                "rustgis_map" => s.rustgis_map = value != "false",
                "rustgis_shown" => {
                    let mut parts = value.split('\t');
                    if let Some(project) = parts.next().filter(|p| !p.is_empty()) {
                        s.set_rustgis_shown(project, parts.filter(|n| !n.is_empty()).map(str::to_string).collect());
                    }
                }
                "sky_imagery" => s.sky_imagery = value != "false",
                // Адрес подложки: строка без пробелов, как шаблон своего
                // источника.
                // Ключ - строка без пробелов.
                "earthdata" if !value.chars().any(char::is_whitespace) => s.earthdata = value.to_string(),
                "terms_ok" if crate::basemap::READY.iter().any(|r| r.warned && r.key == value) && !s.terms_ok.iter().any(|k| k == value) => s.terms_ok.push(value.to_string()),
                "analysis_ok" if !value.is_empty() && !value.chars().any(char::is_whitespace) && !s.analysis_ok.iter().any(|u| u == value) => s.analysis_ok.push(value.to_string()),
                "links" => s.links = data::menulinks::load(value),
                "sat_groups" => s.sat_groups = value.split(',').filter(|g| data::satellites::GROUPS.iter().any(|(k, _)| k == g)).map(str::to_string).collect(),
                "coords" if COORD_FORMATS.contains(&value) => s.coords = value.to_string(),
                "vector" => s.vector = crate::vector::VectorBase::from_text(value).to_text(),
                "history" if !value.is_empty() && s.history.len() < scene::searchbar::HISTORY => s.history.push(value.to_string()),
                "basemap" if Basemap::from_key(value).is_some() => s.basemap = value.to_string(),
                "opacity" => {
                    s.opacity = value
                        .split(',')
                        .filter_map(|p| p.split_once(':'))
                        .filter(|(k, _)| crate::maps::index(k).is_some())
                        .filter_map(|(k, v)| v.parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| (k.to_string(), v.clamp(0.0, 1.0))))
                        .collect();
                }
                "route" if data::routing::Mode::from_name(value).is_some() => s.route = value.to_string(),
                "dots" => {
                    s.dots = value.split(',').filter(|k| crate::dots::Layer::from_key(k).is_some()).map(str::to_string).collect();
                }
                "maps" => {
                    // Слои всего каталога GIBS ждут его ответа.
                    s.maps = value.split(',').filter(|k| crate::maps::index(k).is_some() || (k.starts_with(data::gibs::CATALOG_PREFIX) && k.len() < 200)).map(str::to_string).collect();
                }
                "places_height" => {
                    if let Ok(v) = value.parse::<f32>()
                        && v.is_finite()
                    {
                        s.places_height = v.clamp(0.0, PLACES_MAX);
                    }
                }
                "shading" => {
                    if let Ok(v) = value.parse::<f64>()
                        && v.is_finite()
                    {
                        s.shading = v.clamp(0.0, 1.0);
                    }
                }
                "light_azimuth" => {
                    if let Ok(v) = value.parse::<f64>()
                        && v.is_finite()
                    {
                        s.light_azimuth = v.rem_euclid(360.0);
                    }
                }
                "scale" => {
                    if let Ok(v) = value.parse::<f64>()
                        && v.is_finite()
                    {
                        s.scale = v.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
                    }
                }
                _ => {}
            }
        }
        s
    }
}

/// Название языка подписей по коду.
fn label_name(lang: Lang, code: &str) -> String {
    match code {
        "" => tr(lang, "As the interface").to_string(),
        c if c == data::places::LOCAL => tr(lang, "Local names").to_string(),
        c => LANGUAGES.iter().chain(LABEL_EXTRA.iter()).find(|(k, _)| *k == c).map_or(c.to_string(), |(_, n)| n.to_string()),
    }
}

/// Окно «Свойства вида», как `ui/properties.py` модуля: группы «Основа»,
/// «Рельеф», «Подписи», «Координаты», «Оформление», «Недра», «Поиск»
/// с очисткой истории и кнопка «Закрыть». Групп QGIS («Карта QGIS»,
/// «Обновление») и помощника у Lite нет.
/// Язык интерфейса модуль берёт у QGIS, у Lite он выбирается в группе
/// «Оформление». Меняет settings и basemap на месте. Возвращает,
/// изменилось ли что-то, нажата ли кнопка «Источники данных…» и кнопка
/// «Подземный режим…».
pub fn window(ctx: &egui::Context, lang: Lang, open: &mut bool, settings: &mut Settings, basemap: &mut Basemap) -> (bool, bool, bool) {
    let before = (settings.clone(), *basemap);
    let mut sources = false;
    let mut under = false;
    let mut close = false;
    egui::Window::new(tr(lang, "View properties"))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            group(ui, tr(lang, "Base map"), |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("properties-basemap").selected_text(crate::layers::basemap_title(lang, *basemap)).show_ui(ui, |ui| {
                        for b in Basemap::all() {
                            ui.selectable_value(basemap, b, crate::layers::basemap_title(lang, b));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "Source of the globe imagery. The terms of use of each source are in the Data sources window."));
                    if ui.button(tr(lang, "Data sources…")).on_hover_text(tr(lang, "All globe sources with their terms of use.")).clicked() {
                        sources = true;
                    }
                });
            });
            group(ui, tr(lang, "Terrain"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Vertical exaggeration"));
                    ui.add_enabled(
                        settings.relief,
                        egui::DragValue::new(&mut settings.scale).range(SCALE_RANGE.0..=SCALE_RANGE.1).speed(SCALE_STEP).fixed_decimals(1),
                    )
                    .on_hover_text(tr(
                        lang,
                        "Multiplier of terrain heights. Above 1 mountains and valleys stand out more, and flat terrain becomes visible. The camera stays above the raised surface. After a change the globe rebuilds the surface within a few seconds.",
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Hillshade"));
                    let mut percent = (settings.shading * 100.0).round();
                    let changed = ui
                        .add_enabled(settings.relief, egui::Slider::new(&mut percent, 0.0..=100.0).step_by(5.0).suffix(" %"))
                        .on_hover_text(tr(
                            lang,
                            "Strength of the shading of slopes and hollows of the terrain in every pixel. 0 % - flat imagery, 50 % - usual, 100 % - twice as sharp. With the Sun on, slopes take the light of the Sun.",
                        ))
                        .changed();
                    if changed {
                        settings.shading = (percent / 100.0).clamp(0.0, 1.0);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Light azimuth"));
                    ui.add_enabled(
                        settings.relief && settings.shading > 0.0,
                        egui::DragValue::new(&mut settings.light_azimuth).range(0.0..=360.0).speed(5.0).suffix("°").fixed_decimals(0),
                    )
                    .on_hover_text(tr(lang, "Direction the hillshade light comes from, degrees clockwise from north. 315° - from the north-west, as on maps."));
                });
                ui.checkbox(&mut settings.sea, tr(lang, "Sea and ocean depths")).on_hover_text(tr(
                    lang,
                    "With the box checked the sea and ocean floor lies at its depths under semi-transparent water, the status bar shows the depth under the cursor, and the elevation profile shows depths and sea level. Without it heights below sea level count as zero and the sea is flat. Exists only on the Earth.",
                ));
            });
            group(ui, tr(lang, "Labels"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Language")).on_hover_text(tr(
                        lang,
                        "Language of place, water, peak and other labels on the globe. \"As the interface\" takes the interface language. If a name in the chosen language is missing, the Latin or local name is shown. Labels change at once.",
                    ));
                    egui::ComboBox::from_id_salt("settings-labels").selected_text(label_name(lang, &settings.labels)).show_ui(ui, |ui| {
                        for code in std::iter::once("").chain(std::iter::once(data::places::LOCAL)).chain(data::places::LABEL_LANGUAGES) {
                            ui.selectable_value(&mut settings.labels, code.to_string(), label_name(lang, code));
                        }
                    });
                });
            });
            group(ui, tr(lang, "Coordinates"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Format"));
                    let name = |key: &str| match key {
                        "dms" => tr(lang, "Degrees, minutes, seconds"),
                        "utm" => "UTM",
                        "mgrs" => "MGRS",
                        _ => tr(lang, "Decimal degrees"),
                    };
                    egui::ComboBox::from_id_salt("properties-coords").selected_text(name(&settings.coords)).show_ui(ui, |ui| {
                        for key in COORD_FORMATS {
                            ui.selectable_value(&mut settings.coords, key.to_string(), name(key));
                        }
                    })
                    .response
                    .on_hover_text(tr(
                        lang,
                        "How coordinates are written in the status line and the Features window. The Search field understands all four formats whatever the choice. North of 84° and south of 80° UTM and MGRS are replaced with decimal degrees.",
                    ));
                });
            });
            group(ui, tr(lang, "Appearance"), |ui| {
                egui::Grid::new("properties-look").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(tr(lang, "Appearance"));
                    let shown = if settings.dark { tr(lang, "Dark") } else { tr(lang, "Light") };
                    egui::ComboBox::from_id_salt("properties-theme").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.dark, false, tr(lang, "Light"));
                        ui.selectable_value(&mut settings.dark, true, tr(lang, "Dark"));
                    })
                    .response
                    .on_hover_text(tr(lang, "Look of the globe window. Light - grey plates with dark icons and light windows. Dark - dark plates and dark windows. The look changes at once."));
                    ui.end_row();
                    ui.label(tr(lang, "Interface language"));
                    let current = settings.language.as_deref().and_then(Lang::from_code);
                    let shown = current.map_or(tr(lang, "As in the system"), Lang::native_name);
                    egui::ComboBox::from_id_salt("settings-language").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.language, None, tr(lang, "As in the system"));
                        for (code, name) in LANGUAGES {
                            ui.selectable_value(&mut settings.language, Some(code.to_string()), name);
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "Language of menus, windows and messages. \"As in the system\" takes the language of the system."));
                    ui.end_row();
                });
            });
            group(ui, tr(lang, "Inside the Earth"), |ui| {
                let button = ui.button(tr(lang, "Subsurface mode…")).on_hover_text(tr(
                    lang,
                    "The Subsurface mode window - surface transparency above the model, the block cut and the camera under the ground. The data is a GeoPackage file with drill holes, bed roofs, sections and tunnels, open it like any other file.",
                ));
                if button.clicked() {
                    under = true;
                }
            });
            // Связь с RustGIS - только у программы: у страницы нет сокетов.
            if !cfg!(target_arch = "wasm32") {
                group(ui, "RustGIS", |ui| {
                    ui.checkbox(&mut settings.rustgis, tr(lang, "Link with the open RustGIS")).on_hover_text(tr(
                        lang,
                        "Layers of the RustGIS project are listed in the RustGIS project section, switched off: a check shows a layer on the globe in its RustGIS colours. The globe follows the RustGIS map, and the map follows the globe. A click with Identify selects the feature in RustGIS. In RustGIS start the MCP Relay command or tick Let your Agents Control RustGIS in the assistant settings.",
                    ));
                    // «Слои как на карте QGIS» окна свойств модуля.
                    ui.checkbox(&mut settings.rustgis_map, tr(lang, "Layers as on the RustGIS map")).on_hover_text(tr(
                        lang,
                        "With the check the globe shows the layers switched on in RustGIS, a check in the RustGIS project section switches the layer on the map too, and the opacity sets its transparency there. Without the check the checks of the globe are its own, kept for each project, and do not change the map.",
                    ));
                    // Направление синхронизации, как «Синхронизация»
                    // свойств вида модуля.
                    #[cfg(not(target_arch = "wasm32"))]
                    ui.horizontal(|ui| {
                        let current = crate::rustgislink::Direction::from_key(&settings.sync).unwrap_or_default();
                        ui.label(tr(lang, "Synchronisation"));
                        egui::ComboBox::from_id_salt("planetx-sync-direction")
                            .selected_text(tr(lang, current.name()))
                            .show_ui(ui, |ui| {
                                for d in crate::rustgislink::Direction::ALL {
                                    if ui.selectable_label(d == current, tr(lang, d.name())).clicked() {
                                        settings.sync = d.key().to_string();
                                    }
                                }
                            })
                            .response
                            .on_hover_text(tr(
                                lang,
                                "Who follows whom when the synchronisation is on by the icon in the corner of the view. Map leads the globe - a shift and zoom of the map move the globe to the same area, the tilt and heading of the globe stay. Globe leads the map - after the globe stops, the map goes to its look-at point.",
                            ));
                    });
                });
            }
            group(ui, tr(lang, "Search"), |ui| {
                let button = ui.add_enabled(!settings.history.is_empty(), egui::Button::new(tr(lang, "Clear search history")));
                if button.on_hover_text(tr(lang, "Erase the previous queries of the Search field.")).clicked() {
                    settings.history.clear();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(tr(lang, "Close")).clicked() {
                    close = true;
                }
            });
        });
    if close {
        *open = false;
    }
    ((settings.clone(), *basemap) != before, sources, under)
}

/// Форматы координат строки состояния и меню глобуса, `FORMATS` модуля.
pub const COORD_FORMATS: [&str; 4] = ["decimal", "dms", "utm", "mgrs"];

/// Группа окна, как QGroupBox модуля: заголовок и рамка.
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).strong());
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(340.0);
        add(ui);
    });
    ui.add_space(4.0);
}
#[cfg(test)]
mod tests {
    use super::{SCALE_RANGE, Settings};

    #[test]
    fn settings_round_trip_and_hostile_text() {
        let s = Settings { section_colors: vec![("Уголь_top".into(), Some([1, 2, 255]), None), ("@host".into(), None, Some([9, 8, 7]))], language: Some("de".into()), labels: "local".into(), dark: false, scale: 2.5, sun: true, shading: 0.75, light_azimuth: 280.0, grid: true, air: false, stars: false, sea: false, relief: false, vector: "roads,railways;1".into(), coords: "utm".into(), history: vec!["Вологда".into(), "Paris".into()], places_height: 240.0, maps: vec!["emag".into(), "hiking".into()], basemap: "esri_topo".into(), dots: vec!["quakes".into()], route: "foot".into(), opacity: vec![("emag".into(), 0.5)], stretch: false, own: Vec::new(), wms: vec![crate::maps::WmsSaved { name: "Orthos".into(), template: "https://x.org/wms?SERVICE=WMS&BBOX={bbox}".into(), bbox: Some([-76.1, 39.5, -74.4, 40.6]), credit: "X".into(), max_level: None }, crate::maps::WmsSaved { name: "Снимок".into(), template: "https://tiles.openaerialmap.org/a/0/b/{z}/{x}/{y}".into(), bbox: Some([120.1, -8.4, 120.2, -8.3]), credit: "Y, CC BY 4.0".into(), max_level: Some(21) }], geoid_scale: 2000.0, links: vec![("OSM".into(), "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map={zoom}/{lat}/{lon}".into())], sat_groups: vec!["gnss".into(), "geo".into()], constellations: false, deepsky: false, sky_imagery: false, ai: crate::ai::AiSettings { provider: "anthropic".into(), keys: vec![("anthropic".into(), "sk-ant-1".into())], search: false, ..Default::default() }, analysis_ok: vec!["https://server.arcgisonline.com/x/{z}/{y}/{x}".into()], terms_ok: vec!["google_satellite".into(), "dgis_map".into()], earthdata: "eyJ0.test".into(), rustgis: true, sync: "globe".into(), rustgis_map: false, rustgis_shown: vec![("World Atlas".into(), vec!["Cities".into(), "Реки и озёра".into()])] };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        // Отмывка: сила в пределах 0..1, азимут по кругу.
        let h = Settings::from_text("shading=7\nlight_azimuth=-45\n");
        assert_eq!((h.shading, h.light_azimuth), (1.0, 315.0));
        assert_eq!((Settings::default().shading, Settings::default().light_azimuth), (0.5, 315.0));
        // «Слои как на карте RustGIS» включены по умолчанию, и в старых
        // настройках с прежним ключом rustgis_follow тоже.
        assert!(Settings::default().rustgis_map && Settings::from_text("rustgis_follow=C:/a.rgis\n").rustgis_map);
        // Отмеченные слои - по ключу проекта.
        let mut p = Settings::default();
        p.set_rustgis_shown("w", vec!["x\ty".into()]);
        assert_eq!(p.rustgis_shown_of("w"), ["xy"]);
        p.set_rustgis_shown("w", Vec::new());
        assert!(p.rustgis_shown_of("w").is_empty());
        for i in 0..super::RUSTGIS_PROJECTS + 5 {
            p.set_rustgis_shown(&format!("p{i}"), vec!["x".into()]);
        }
        assert_eq!(p.rustgis_shown.len(), super::RUSTGIS_PROJECTS);
        assert!(p.rustgis_shown_of("p0").is_empty() && !p.rustgis_shown_of(&format!("p{}", super::RUSTGIS_PROJECTS + 4)).is_empty());
        let bad = Settings::from_text("language=xx\nlabels=../x\ntheme=dark\nscale=1e9\nscale=NaN\nfoo=bar\nno equals\nplaces_height=-5\nplaces_height=NaN\nmaps=emag,../x,gibs:MODIS_Terra_NDSI_Snow_Cover\nwms=a\tftp://x/{bbox}\t\tb\nanalysis_ok=a b\nanalysis_ok=");
        assert!(bad.analysis_ok.is_empty());
        assert_eq!(bad.maps, ["emag", "gibs:MODIS_Terra_NDSI_Snow_Cover"]);
        assert!(bad.wms.is_empty());
        assert_eq!(bad.places_height, 0.0);
        assert_eq!(bad.language, None);
        assert_eq!(bad.labels, "");
        assert!(bad.dark);
        assert_eq!(bad.scale, SCALE_RANGE.1);
    }
}
