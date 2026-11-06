// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Связь с открытым RustGIS - аналог связи модуля с проектом QGIS,
//! решение автора от 5 ноября 2026 года. Слои проекта RustGIS - на
//! глобусе папкой «RustGIS», вид карты и глобуса - в обе стороны,
//! выбранный на глобусе объект выделяется в RustGIS, слой Lite уходит
//! в проект RustGIS.
//!
//! Путь - ретранслятор MCP RustGIS (команда «MCP Relay» или флажок «Let
//! your Agents Control RustGIS» помощника RustGIS): 127.0.0.1, порт
//! [`DEFAULT_PORT`] или `RUSTGIS_MCP_PORT`, ключ - файл [`token_path`],
//! его пишет RustGIS. Первая строка - `rustgis/attach` с ключом, дальше -
//! сообщения MCP JSON-RPC по строке, как у `rustgis-cli mcp --attach`.
//!
//! Поток опрашивает `list_layers` каждые [`POLL_SECONDS`]: событий
//! изменения карты RustGIS не шлёт. Объекты слоя читаются заново, когда
//! меняется их ключ ([`data::rustgis::Layer::data_key`]): `data.convert`
//! пишет GeoJSON во временную папку, ответ команды длиной ограничен.
//! Стиль - `share.webStyle` тем же путём, только при смене отрисовки.
//! Только программа: у страницы нет ни сокетов, ни файлов.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::Arc;
use std::time::Duration;

use data::layers::LayerData;
use data::rustgis::{State, Style};

/// Причина «нет связи», когда файла ключа нет: ретранслятор не запущен.
/// Ключ перевода.
pub const RELAY_OFF: &str = "the MCP relay of RustGIS is off";
/// Порт ретранслятора RustGIS по умолчанию.
pub const DEFAULT_PORT: u16 = 8767;
/// Секунд между опросами проекта.
pub const POLL_SECONDS: f64 = 0.5;
/// Секунд между попытками подключиться, пока ретранслятор выключен.
const RETRY_SECONDS: u64 = 2;
/// Секунд на ответ RustGIS: долгая команда держит окно RustGIS.
const REPLY_SECONDS: u64 = 60;
/// Строка ответа не длиннее, байт.
const MAX_LINE: usize = 64 << 20;
/// Файл объектов слоя не больше, байт.
const MAX_FILE: u64 = 512 << 20;
/// Слой Lite в проект RustGIS не больше, байт текста GeoJSON: строка
/// ретранслятора RustGIS - до 64 МБ (`framing::MAX_LINE`), запас - на
/// обёртку запроса.
const MAX_OUT: usize = 60 << 20;

/// Порт: `RUSTGIS_MCP_PORT` или [`DEFAULT_PORT`], как у RustGIS.
pub fn port() -> u16 {
    std::env::var("RUSTGIS_MCP_PORT").ok().and_then(|p| p.trim().parse().ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT)
}

/// Файл ключа ретранслятора на порту port - `token_path` RustGIS:
/// папка `rustgis-mcp-<пользователь>` во временной папке системы или
/// `$XDG_RUNTIME_DIR/rustgis`.
pub fn token_path(port: u16) -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()) {
        return runtime.join("rustgis").join(format!("{port}.token"));
    }
    let user: String = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let user = if user.is_empty() { "user".to_string() } else { user };
    std::env::temp_dir().join(format!("rustgis-mcp-{user}")).join(format!("{port}.token"))
}

/// Просьба окна к RustGIS.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Карта - в точку с масштабом.
    GoTo { lat: f64, lon: f64, scale: f64 },
    /// Выделить строки слоя.
    Select { layer: i64, rows: Vec<usize> },
    /// Слой Lite - в проект RustGIS текстом GeoJSON.
    AddLayer { name: String, geojson: String },
    /// Расчёт окна анализа - в проект RustGIS подгруппой group группы top,
    /// как результаты инструментов модуля - в проект QGIS: векторы
    /// текстом GeoJSON (название, текст), растры файлами GeoTIFF
    /// (название, путь). Сверху вниз - векторы, под ними растры.
    AddResult { top: String, group: String, vectors: Vec<(String, String)>, rasters: Vec<(String, String)> },
    /// Видимость слоя на карте RustGIS: отметка глобуса в режиме «Слои как
    /// на карте RustGIS», `set_visible_on_map` модуля.
    Visibility { layer: i64, visible: bool },
    /// Прозрачность слоя на карте RustGIS в процентах, 0 - непрозрачный.
    Transparency { layer: i64, percent: u8 },
}

/// Весть потока окну.
pub enum News {
    /// Подключено к RustGIS.
    Connected,
    /// Связи нет: ретранслятор выключен или RustGIS закрыт.
    Waiting(String),
    /// Вид и слои проекта.
    State(State),
    /// Объекты и стиль слоя номер id. В коробке: слой велик рядом с другими
    /// вестями.
    Layer { id: i64, data: Box<LayerData>, style: Box<Style> },
    /// Просьба выполнена, текст - для строки сообщения.
    Done(String),
    /// Расчёт лёг в проект RustGIS группой с этим названием.
    Added(String),
    /// Просьба или чтение слоя не удались.
    Failed(String),
}

/// Связь с RustGIS: поток и его каналы. Остановка - при удалении.
pub struct Link {
    asks: Sender<Ask>,
    news: Receiver<News>,
    stop: Arc<AtomicBool>,
}

impl Link {
    /// Запустить поток. wake будит окно, когда пришла весть.
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Link {
        let (asks, ask_rx) = channel();
        let (news_tx, news) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || worker(&ask_rx, &news_tx, &flag, &*wake));
        Link { asks, news, stop }
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    /// Следующая весть или None.
    pub fn take(&self) -> Option<News> {
        self.news.try_recv().ok()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Соединение MCP с ретранслятором.
struct Session {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next: u64,
}

impl Session {
    fn open(port: u16) -> Result<Session, String> {
        let token = std::fs::read_to_string(token_path(port)).map_err(|_| RELAY_OFF.to_string())?;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(REPLY_SECONDS))).map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut s = Session { writer, reader: BufReader::new(stream), next: 1 };
        // access - чтение и запись файлов: объекты слоя идут временным
        // файлом, ответ команды длиной ограничен.
        s.send(&serde_json::json!({"jsonrpc": "2.0", "method": "rustgis/attach", "params": {"access": true, "token": token.trim(), "timeout_s": REPLY_SECONDS}}))?;
        s.request("initialize", serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "planetx-lite", "version": env!("CARGO_PKG_VERSION")}}))?;
        s.send(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(s)
    }

    fn send(&mut self, msg: &serde_json::Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(|e| e.to_string())
    }

    /// Запрос и его ответ. Уведомления без номера пропускаются.
    fn request(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let id = self.next;
        self.next += 1;
        self.send(&serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let mut line = String::new();
            let n = std::io::Read::take(&mut self.reader, MAX_LINE as u64).read_line(&mut line).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("RustGIS closed the connection".to_string());
            }
            let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if reply["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(e) = reply.get("error") {
                return Err(e["message"].as_str().unwrap_or("error").to_string());
            }
            return Ok(reply["result"].clone());
        }
    }

    /// Инструмент MCP: текст ответа. Ответ с isError - ошибка с его текстом.
    fn tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<String, String> {
        let result = self.request("tools/call", serde_json::json!({"name": name, "arguments": arguments}))?;
        let text = result["content"].as_array().and_then(|c| c.iter().find_map(|p| p["text"].as_str())).unwrap_or("").to_string();
        if result["isError"] == true { Err(text) } else { Ok(text) }
    }

    fn command(&mut self, command: &str, params: serde_json::Value) -> Result<String, String> {
        self.tool("run_command", serde_json::json!({"command": command, "params": params}))
    }
}

/// Папка временных файлов связи.
fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("planetx-lite-rustgis")
}

/// Прочесть и удалить временный файл.
fn take_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE {
        let _ = std::fs::remove_file(path);
        return Err("the layer file is too large".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(path);
    bytes
}

/// Путь для RustGIS: косая черта вместо обратной.
fn slash(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Прочесть стиль слоя id.
fn read_style(s: &mut Session, id: i64) -> Result<Style, String> {
    let path = temp_dir().join(format!("style-{id}.json"));
    s.command("share.webStyle", serde_json::json!({"layer": id, "path": slash(&path)}))?;
    let bytes = take_file(&path)?;
    Ok(data::rustgis::parse_style(&String::from_utf8_lossy(&bytes)))
}

/// Прочесть объекты слоя id.
fn read_layer(s: &mut Session, id: i64) -> Result<LayerData, String> {
    let path = temp_dir().join(format!("layer-{id}.geojson"));
    s.command("data.convert", serde_json::json!({"layer": id, "format": "geojson", "path": slash(&path)}))?;
    let bytes = take_file(&path)?;
    let mut layers = data::layers::read(bytes, "layer.geojson")?;
    if layers.is_empty() {
        return Err("no features".to_string());
    }
    Ok(layers.remove(0))
}

/// Выполнить просьбу окна. Ok - текст для строки сообщения.
fn perform(s: &mut Session, ask: Ask) -> Result<String, String> {
    match ask {
        Ask::GoTo { lat, lon, scale } => s.command("map.goToXY", serde_json::json!({"lon": lon, "lat": lat, "scale": scale})).map(|_| String::new()),
        Ask::Select { layer, rows } => s.command("select.rows", serde_json::json!({"layer": layer, "rows": rows, "mode": "new"})).map(|_| String::new()),
        // Слой - текстом: в проекте он без ссылки на файл, как слой,
        // нарисованный в RustGIS. Строка ретранслятора - до 64 МБ.
        Ask::AddLayer { name, geojson } => {
            if geojson.len() > MAX_OUT {
                return Err(format!("{name}: the layer is larger than {} MB", MAX_OUT >> 20));
            }
            add_text(s, &name, &geojson).map(|_| name)
        }
        // Расчёт выполняет perform_result, весть у него своя.
        Ask::AddResult { top, group, vectors, rasters } => perform_result(s, &top, &group, &vectors, &rasters),
        Ask::Visibility { layer, visible } => s.command("layer.visibility", serde_json::json!({"layer": layer, "visible": visible})).map(|_| String::new()),
        Ask::Transparency { layer, percent } => s.command("layer.transparency", serde_json::json!({"layer": layer, "value": percent.min(100)})).map(|_| String::new()),
    }
}

/// Слой текстом GeoJSON в проект RustGIS. Строка ретранслятора - до 64 МБ.
fn add_text(s: &mut Session, name: &str, geojson: &str) -> Result<(), String> {
    if geojson.len() > MAX_OUT {
        return Err(format!("{name}: the layer is larger than {} MB", MAX_OUT >> 20));
    }
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    s.command("layer.addDataText", serde_json::json!({"fileName": format!("{file}.geojson"), "text": geojson, "name": name})).map(|_| ())
}

/// Расчёт - в проект RustGIS, `results.add_group` модуля: слои, затем
/// подгруппа с ними в группе top. Группы top ещё нет - она заводится.
/// Ok - название подгруппы.
fn perform_result(s: &mut Session, top: &str, group: &str, vectors: &[(String, String)], rasters: &[(String, String)]) -> Result<String, String> {
    let mut names = Vec::new();
    for (name, geojson) in vectors {
        add_text(s, name, geojson)?;
        names.push(name.clone());
    }
    for (name, path) in rasters {
        s.command("layer.addData", serde_json::json!({"path": path.replace('\\', "/"), "name": name}))?;
        names.push(name.clone());
    }
    let sub = serde_json::json!({"name": group, "parent": top, "layers": names});
    if s.command("group.add", sub.clone()).is_err() {
        s.command("group.add", serde_json::json!({"name": top}))?;
        s.command("group.add", sub)?;
    }
    Ok(group.to_string())
}

/// Ждать до секунд seconds, просыпаясь на просьбы и остановку. false -
/// поток остановлен.
fn nap(seconds: f64, stop: &AtomicBool) -> bool {
    let steps = (seconds / 0.05).ceil().max(1.0) as u32;
    for _ in 0..steps {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !stop.load(Ordering::SeqCst)
}

fn worker(asks: &Receiver<Ask>, news: &Sender<News>, stop: &AtomicBool, wake: &(dyn Fn() + Send + Sync)) {
    let tell = |n: News| {
        let _ = news.send(n);
        wake();
    };
    let _ = std::fs::create_dir_all(temp_dir());
    let mut last_wait = String::new();
    while !stop.load(Ordering::SeqCst) {
        let mut s = match Session::open(port()) {
            Ok(s) => s,
            Err(e) => {
                if e != last_wait {
                    tell(News::Waiting(e.clone()));
                    last_wait = e;
                }
                // Просьбы без связи не копятся.
                while asks.try_recv().is_ok() {}
                if !nap(RETRY_SECONDS as f64, stop) {
                    return;
                }
                continue;
            }
        };
        last_wait.clear();
        tell(News::Connected);
        let mut data_keys: HashMap<i64, String> = HashMap::new();
        let mut styles: HashMap<i64, (String, Style)> = HashMap::new();
        let failed = loop {
            loop {
                match asks.try_recv() {
                    Ok(Ask::AddResult { top, group, vectors, rasters }) => match perform_result(&mut s, &top, &group, &vectors, &rasters) {
                        Ok(name) => tell(News::Added(name)),
                        Err(e) => tell(News::Failed(e)),
                    },
                    Ok(ask) => match perform(&mut s, ask) {
                        Ok(text) => {
                            if !text.is_empty() {
                                tell(News::Done(text));
                            }
                        }
                        Err(e) => tell(News::Failed(e)),
                    },
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            let text = match s.tool("list_layers", serde_json::json!({"include_fields": false})) {
                Ok(t) => t,
                Err(e) => break e,
            };
            let Some(state) = data::rustgis::parse_state(&text) else {
                if !nap(POLL_SECONDS, stop) {
                    return;
                }
                continue;
            };
            let drawn: Vec<data::rustgis::Layer> = state.layers.iter().filter(|l| l.drawn()).cloned().collect();
            data_keys.retain(|id, _| drawn.iter().any(|l| l.id == *id));
            styles.retain(|id, _| drawn.iter().any(|l| l.id == *id));
            tell(News::State(state));
            for layer in drawn {
                let style_key = layer.style_key();
                let style_new = styles.get(&layer.id).is_none_or(|(k, _)| *k != style_key);
                let data_new = data_keys.get(&layer.id).is_none_or(|k| *k != layer.data_key());
                if !style_new && !data_new {
                    continue;
                }
                if style_new {
                    let style = read_style(&mut s, layer.id).unwrap_or_default();
                    styles.insert(layer.id, (style_key, style));
                }
                match read_layer(&mut s, layer.id) {
                    Ok(data) => {
                        let style = styles.get(&layer.id).map(|(_, st)| st.clone()).unwrap_or_default();
                        tell(News::Layer { id: layer.id, data: Box::new(data), style: Box::new(style) });
                    }
                    Err(e) => tell(News::Failed(format!("{}: {e}", layer.name))),
                }
                // Слой, который не прочёлся, второй раз не просится, пока
                // не изменится.
                data_keys.insert(layer.id, layer.data_key());
            }
            if !nap(POLL_SECONDS, stop) {
                return;
            }
        };
        tell(News::Waiting(failed));
    }
}

/// Папка живых слоёв в «Моих слоях».
pub const FOLDER: &str = "RustGIS";
/// Секунд покоя глобуса, после которых вид уходит в RustGIS: карта не
/// дёргается на каждом кадре перелёта.
pub const SETTLE_SECONDS: f64 = 0.3;

/// Вид: широта, долгота, ширина полосы на местности в метрах.
pub type Spot = (f64, f64, f64);

/// Кто за кем следует при включённой синхронизации, `core/sync.py`
/// модуля. Выбор - в «Свойствах вида», по умолчанию в обе стороны.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Both,
    /// Карта ведёт глобус: глобус переходит к охвату карты, наклон
    /// и азимут глобуса остаются.
    MapToGlobe,
    /// Глобус ведёт карту: карта встаёт в точку взгляда глобуса, когда
    /// он остановится.
    GlobeToMap,
}

impl Direction {
    pub const ALL: [Direction; 3] = [Direction::Both, Direction::MapToGlobe, Direction::GlobeToMap];

    /// Ключ в настройках, как у модуля.
    pub fn key(self) -> &'static str {
        match self {
            Direction::Both => "both",
            Direction::MapToGlobe => "map",
            Direction::GlobeToMap => "globe",
        }
    }

    pub fn from_key(key: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.key() == key)
    }

    /// Название в списке «Свойств вида», ключ перевода.
    pub fn name(self) -> &'static str {
        match self {
            Direction::Both => "Both ways",
            Direction::MapToGlobe => "Map leads the globe",
            Direction::GlobeToMap => "Globe leads the map",
        }
    }

    fn map_leads(self) -> bool {
        self != Direction::GlobeToMap
    }

    fn globe_leads(self) -> bool {
        self != Direction::MapToGlobe
    }
}

/// Связь окна с RustGIS: поток, состояние и синхронизация видов,
/// как `ui/sync.py` модуля в обе стороны.
pub struct Bridge {
    pub link: Link,
    pub connected: bool,
    /// Почему связи нет.
    pub status: String,
    /// Последний вид карты RustGIS.
    pub view: Option<data::rustgis::View>,
    /// Слои проекта: номер и название.
    pub names: Vec<(i64, String)>,
    /// Слои, включённые на карте RustGIS.
    pub on_map: Vec<i64>,
    /// Ключ открытого проекта, [`data::rustgis::State::project`].
    pub project: String,
    /// Вид карты, уже показанный на глобусе.
    last_map: Option<Spot>,
    /// Вид глобуса, отправленный в RustGIS.
    last_sent: Option<Spot>,
    /// Вид глобуса на прошлом кадре, когда он сдвинулся и ждёт ли отправки.
    last_globe: Option<Spot>,
    moved_at: f64,
    pending: bool,
    /// Синхронизация включена кнопкой панели значков, как у модуля:
    /// без неё виды карты и глобуса независимы.
    pub sync: bool,
    pub direction: Direction,
    /// Синхронизацию только включили, глобус ведёт: его вид уходит
    /// на карту сразу, без ожидания покоя.
    push_now: bool,
}

impl Bridge {
    pub fn new(link: Link) -> Bridge {
        Bridge {
            link,
            connected: false,
            status: String::new(),
            view: None,
            names: Vec::new(),
            on_map: Vec::new(),
            project: String::new(),
            last_map: None,
            last_sent: None,
            last_globe: None,
            moved_at: 0.0,
            pending: false,
            sync: false,
            direction: Direction::Both,
            push_now: false,
        }
    }

    /// Кнопка синхронизации, `MapSync.set_enabled` модуля. При включении
    /// ведущая сторона сразу ставит ведомую: глобус ведёт карту только
    /// в направлении «Глобус ведёт карту», иначе ведёт карта.
    pub fn set_sync(&mut self, on: bool) {
        if on == self.sync {
            return;
        }
        self.sync = on;
        self.last_map = None;
        self.last_sent = None;
        self.pending = false;
        // Глобус ведёт - его вид уходит на карту на ближайшем кадре.
        self.push_now = on && self.direction == Direction::GlobeToMap;
    }

    /// Пришёл вид карты. Some - синхронизация включена, карта ведёт и её
    /// сдвинул пользователь RustGIS (или синхронизацию только включили):
    /// глобус идёт за ней - широта, долгота, ширина и высота полосы
    /// в метрах. None - карта на месте или это ответ на шаг глобуса.
    pub fn map_view(&mut self, view: data::rustgis::View) -> Option<(f64, f64, f64, f64)> {
        let (w, h) = data::rustgis::ground_size(&view);
        let spot = (view.lat, view.lon, w);
        self.view = Some(view);
        if !self.sync || !self.direction.map_leads() {
            return None;
        }
        if self.last_map.is_some_and(|m| data::rustgis::same_view(m, spot)) {
            return None;
        }
        self.last_map = Some(spot);
        if self.last_sent.is_some_and(|s| data::rustgis::same_view(s, spot)) {
            return None;
        }
        Some((view.lat, view.lon, w, h))
    }

    /// Глобус поставлен по карте: его новый вид - не шаг пользователя.
    pub fn applied(&mut self, globe: Spot) {
        self.last_globe = Some(globe);
        self.pending = false;
    }

    /// Вид глобуса на этом кадре. Some - глобус сдвинули и он постоял
    /// [`SETTLE_SECONDS`]: просьба передвинуть карту.
    pub fn globe_view(&mut self, globe: Spot, now: f64) -> Option<Ask> {
        if !self.sync || !self.direction.globe_leads() {
            self.last_globe = Some(globe);
            return None;
        }
        // Только включили, глобус ведёт: сразу, как только известен вид
        // карты (масштаб считается от него).
        if self.push_now && self.view.is_some() {
            self.push_now = false;
            self.last_globe = Some(globe);
            self.pending = true;
            self.moved_at = f64::NEG_INFINITY;
        }
        if !self.last_globe.is_some_and(|g| data::rustgis::same_view(g, globe)) {
            self.last_globe = Some(globe);
            self.moved_at = now;
            self.pending = true;
            return None;
        }
        if !self.pending || now - self.moved_at < SETTLE_SECONDS {
            return None;
        }
        self.pending = false;
        let view = self.view?;
        if self.last_map.is_some_and(|m| data::rustgis::same_view(m, globe)) {
            return None;
        }
        self.last_sent = Some(globe);
        self.last_map = Some(globe);
        Some(Ask::GoTo { lat: globe.0, lon: globe.1, scale: data::rustgis::scale_for(globe.2, &view) })
    }

    /// Название слоя проекта номер id.
    pub fn name(&self, id: i64) -> String {
        self.names.iter().find(|(i, _)| *i == id).map_or_else(|| format!("RustGIS {id}"), |(_, n)| n.clone())
    }

    /// Состояние проекта пришло: слои, их видимость на карте и ключ
    /// проекта. true - открыт другой проект, его настройки глобуса
    /// читаются заново, `_project_reloaded` модуля.
    pub fn take_state(&mut self, state: &data::rustgis::State) -> bool {
        let drawn = || state.layers.iter().filter(|l| l.drawn());
        self.names = drawn().map(|l| (l.id, l.name.clone())).collect();
        self.on_map = drawn().filter(|l| l.visible).map(|l| l.id).collect();
        let other = self.project != state.project;
        self.project.clone_from(&state.project);
        other
    }

    /// Должен ли слой id быть на глобусе: в режиме «Слои как на карте
    /// RustGIS» (follow) - как на карте, иначе - отмечен ли он
    /// в настройках проекта (saved - названия слоёв).
    pub fn wanted(&self, id: i64, follow: bool, saved: &[String]) -> bool {
        if follow {
            self.on_map.contains(&id)
        } else {
            self.names.iter().any(|(i, n)| *i == id && saved.contains(n))
        }
    }
}

/// Раскраска слоя RustGIS для «Моих слоёв»: цвета объектов, стиль слоя
/// и номер поля подписей.
pub fn paint(data: &LayerData, style: &Style, n: usize) -> (Vec<[u8; 3]>, crate::datalayers::Style, Option<usize>) {
    let mut colors = Vec::with_capacity(data.items.len());
    let mut first = None;
    for item in &data.items {
        let props: serde_json::Map<String, serde_json::Value> = data
            .fields
            .iter()
            .zip(&item.values)
            .enumerate()
            .map(|(i, (f, v))| {
                let value = match (data.numeric.get(i).copied().unwrap_or(false), v.trim().parse::<f64>()) {
                    (true, Ok(x)) => serde_json::json!(x),
                    _ => serde_json::Value::String(v.clone()),
                };
                (f.clone(), value)
            })
            .collect();
        let c = style.color_of(&props);
        first = first.or(c);
        colors.push(c.map_or([0, 0, 0], |[r, g, b, _]| [r, g, b]));
    }
    let mut out = crate::datalayers::Style::new(n);
    if let Some([r, g, b, a]) = first {
        out.color = [r, g, b];
        out.opacity = f32::from(a) / 255.0;
    } else {
        // Стиль не прочёлся: цвет слоя по кругу, без своих цветов объектов.
        colors.clear();
    }
    if let Some(o) = style.fill_opacity() {
        out.fill = o as f32;
    }
    if let Some(w) = style.line_width() {
        out.width = (w as f32).max(1.0);
    }
    if let Some(r) = style.circle_radius() {
        out.size = (2.0 * r as f32).clamp(2.0, 60.0);
    }
    let label = style.label_field.as_ref().and_then(|f| data.fields.iter().position(|x| x == f));
    (colors, out, label)
}

/// Команда расширения RustGIS «Глобус PlanetX» (`plugin.json` расширения
/// RustPlanetX): программа - расширение-процесс RustGIS, `planetx-lite
/// rustgis-plugin`. Строка запроса RustGIS - на input, ответ - строками
/// JSON на output: сообщение строки состояния RustGIS запросом
/// `host.notify` и итог. launch запускает глобус со связью.
pub fn plugin(input: &mut dyn BufRead, output: &mut dyn Write, relay_on: bool, launch: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
    let mut line = String::new();
    input.read_line(&mut line).map_err(|e| e.to_string())?;
    // Строка может начинаться с метки порядка байтов UTF-8.
    let request: serde_json::Value = serde_json::from_str(line.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let lang = request["lang"].as_str().and_then(crate::i18n::Lang::from_code).unwrap_or(crate::i18n::Lang::EN);
    let id = request["id"].as_str().unwrap_or("");
    let mut say = |msg: serde_json::Value| -> Result<(), String> { writeln!(output, "{msg}").map_err(|e| e.to_string()) };
    if id != "globe" {
        return say(serde_json::json!({"error": format!("PlanetX: no command '{id}'")}));
    }
    if let Err(e) = launch() {
        return say(serde_json::json!({"error": crate::i18n::trf(lang, "PlanetX Lite did not start: {error}", &[("error", &e)])}));
    }
    let (text, error) = if relay_on {
        (crate::i18n::tr(lang, "The PlanetX globe opens linked with this map.").to_string(), false)
    } else {
        (crate::i18n::tr(lang, "The PlanetX globe opens. For the link start the MCP Relay command (Ctrl+K) or tick Let your Agents Control RustGIS in the assistant settings.").to_string(), true)
    };
    say(serde_json::json!({"call": {"op": "host.notify", "params": {"text": text, "error": error}}}))?;
    // Ответ RustGIS на сообщение не нужен, но читается: ему отвечают строкой.
    let mut reply = String::new();
    let _ = input.read_line(&mut reply);
    say(serde_json::json!({"ok": {"started": true, "relay": relay_on}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_protocol() {
        let run = |request: &str, relay: bool, ok: bool| -> String {
            let mut input = std::io::Cursor::new(format!("{request}\n{{\"ok\":null}}\n"));
            let mut out = Vec::new();
            let launch = move || if ok { Ok(()) } else { Err("no file".to_string()) };
            let _ = plugin(&mut input, &mut out, relay, &launch);
            String::from_utf8_lossy(&out).into_owned()
        };
        let started = run(r#"{"abi":1,"kind":"command","id":"globe","params":{},"lang":"ru"}"#, true, true);
        let lines: Vec<&str> = started.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("host.notify") && lines[0].contains("\"error\":false"));
        assert!(lines[1].contains("\"started\":true"));
        assert!(run(r#"{"id":"globe","lang":"en"}"#, false, true).contains("MCP Relay"));
        assert!(run(r#"{"id":"globe"}"#, true, false).starts_with("{\"error\""));
        assert!(run(r#"{"id":"other"}"#, true, true).starts_with("{\"error\""));
    }

    fn view(lat: f64, lon: f64, half: f64) -> data::rustgis::View {
        data::rustgis::View { lat, lon, extent: [lon - half, lat - half / 2.0, lon + half, lat + half / 2.0], scale: 1.0e6 }
    }

    #[test]
    fn views_both_ways() {
        let (_news_tx, news) = channel::<News>();
        let link = Link { asks: channel::<Ask>().0, news, stop: Arc::new(AtomicBool::new(false)) };
        let mut s = Bridge::new(link);
        s.set_sync(true);
        // Карта сдвинулась - глобус идёт за ней.
        let moved = s.map_view(view(58.0, 56.0, 0.5));
        assert!(moved.is_some());
        let (lat, lon, w, _) = moved.unwrap_or_default();
        s.applied((lat, lon, w));
        // Тот же вид карты - глобус стоит.
        assert!(s.map_view(view(58.0, 56.0, 0.5)).is_none());
        // Глобус постоял на виде карты - карта не двигается.
        assert!(s.globe_view((lat, lon, w), 1.0).is_none());
        assert!(s.globe_view((lat, lon, w), 2.0).is_none());
        // Глобус сдвинули: ждёт покоя, потом просит карту.
        assert!(s.globe_view((59.0, 56.0, w), 3.0).is_none());
        assert!(s.globe_view((59.0, 56.0, w), 3.1).is_none());
        let ask = s.globe_view((59.0, 56.0, w), 3.5);
        assert!(matches!(ask, Some(Ask::GoTo { lat, .. }) if lat == 59.0));
        // Ответ карты на этот шаг - та же ширина в метрах - глобус не
        // двигает.
        let half = 0.5 * 58f64.to_radians().cos() / 59f64.to_radians().cos();
        let echo = view(59.0, 56.0, half);
        assert!(s.map_view(echo).is_none());
    }

    fn bridge() -> Bridge {
        let (_news_tx, news) = channel::<News>();
        Bridge::new(Link { asks: channel::<Ask>().0, news, stop: Arc::new(AtomicBool::new(false)) })
    }

    /// Слои на глобусе как у модуля: в режиме карты - видимость с карты
    /// RustGIS, без него - отметки проекта из настроек. Другой проект -
    /// его отметки читаются заново.
    #[test]
    fn layers_follow_the_map_or_the_project() {
        let mut s = bridge();
        let state = data::rustgis::parse_state(
            r#"{"name":"Atlas","path":null,"layers":[{"id":1,"name":"Cities","type":"Point","visible":true},{"id":2,"name":"Lakes","type":"Polygon","visible":false},{"id":3,"name":"DEM","type":"Raster","visible":true}]}"#,
        )
        .unwrap_or(data::rustgis::State { view: None, layers: Vec::new(), project: String::new() });
        assert!(s.take_state(&state));
        assert!(!s.take_state(&state));
        assert_eq!(s.project, "Atlas");
        // Растр на глобус не идёт.
        assert_eq!(s.names.len(), 2);
        assert!(s.wanted(1, true, &[]) && !s.wanted(2, true, &[]));
        let saved = vec!["Lakes".to_string()];
        assert!(!s.wanted(1, false, &saved) && s.wanted(2, false, &saved));
        let mut other = state.clone();
        other.project = "C:/gis/other.rgis".into();
        assert!(s.take_state(&other));
    }

    /// Синхронизация как у модуля, `ui/sync.py`: без кнопки виды
    /// независимы, направление решает, кто ведёт, при включении ведущая
    /// сторона сразу ставит ведомую. Прежде связь гоняла виды всегда.
    #[test]
    fn sync_button_and_direction() {
        let mut s = bridge();
        // Кнопка не нажата - ни карта, ни глобус друг друга не двигают.
        assert!(s.map_view(view(58.0, 56.0, 0.5)).is_none());
        assert!(s.globe_view((59.0, 56.0, 50_000.0), 1.0).is_none());
        assert!(s.globe_view((59.0, 56.0, 50_000.0), 5.0).is_none());
        // Включена в обе стороны - карта сразу ставит глобус, даже на
        // прежнем виде.
        s.set_sync(true);
        assert!(s.map_view(view(58.0, 56.0, 0.5)).is_some());
        // Карта ведёт глобус - глобус карту не двигает.
        s.direction = Direction::MapToGlobe;
        assert!(s.globe_view((61.0, 56.0, 50_000.0), 6.0).is_none());
        assert!(s.globe_view((61.0, 56.0, 50_000.0), 7.0).is_none());
        // Глобус ведёт карту - карта глобус не двигает, а при включении
        // глобус уходит на карту на ближайшем кадре.
        s.set_sync(false);
        s.direction = Direction::GlobeToMap;
        s.set_sync(true);
        assert!(s.map_view(view(10.0, 20.0, 0.5)).is_none());
        let ask = s.globe_view((61.0, 56.0, 50_000.0), 8.0);
        assert!(matches!(ask, Some(Ask::GoTo { lat, .. }) if lat == 61.0));
        // Ключи настроек - как у модуля.
        assert_eq!(Direction::ALL.map(Direction::key), ["both", "map", "globe"]);
        assert_eq!(Direction::from_key("globe"), Some(Direction::GlobeToMap));
        assert_eq!(Direction::from_key("off"), None);
    }

    #[test]
    fn paint_graduated() {
        let data = LayerData {
            name: "Countries".into(),
            epsg: Some(4326),
            fields: vec!["name".into(), "pop".into()],
            numeric: vec![false, true],
            items: vec![
                data::layers::Item { shape: data::layers::Shape::Polygon, parts: vec![vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]], heights: Vec::new(), values: vec!["A".into(), "10".into()] },
                data::layers::Item { shape: data::layers::Shape::Polygon, parts: vec![vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]], heights: Vec::new(), values: vec!["B".into(), "1000".into()] },
            ],
        };
        let style = data::rustgis::parse_style(
            r##"{"layers":[{"type":"fill","paint":{"fill-color":["step",["get","pop"],"#000000",100,"#ffffff"],"fill-opacity":0.5}},{"type":"symbol","layout":{"text-field":["get","name"]}}]}"##,
        );
        let (colors, st, label) = paint(&data, &style, 0);
        assert_eq!(colors, vec![[0, 0, 0], [255, 255, 255]]);
        assert_eq!(st.fill, 0.5);
        assert_eq!(label, Some(0));
        // Без стиля - цвет по кругу, своих цветов нет.
        let (colors, _, label) = paint(&data, &Style::default(), 1);
        assert!(colors.is_empty());
        assert_eq!(label, None);
    }

    #[test]
    fn token_file_as_rustgis() {
        let path = token_path(8767);
        assert!(path.ends_with("8767.token"));
        assert!(path.to_string_lossy().contains("rustgis"));
    }

    #[test]
    fn slashes_for_rustgis() {
        assert_eq!(slash(std::path::Path::new("C:\\a\\b.json")), "C:/a/b.json");
    }
}
