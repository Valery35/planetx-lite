// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Связь с открытым RustGIS - аналог связи модуля с проектом QGIS,
//! решение автора от 5 ноября 2026 года. Слои проекта RustGIS - на
//! глобусе папкой «RustGIS», вид карты и глобуса - в обе стороны,
//! выбранный на глобусе объект выделяется в RustGIS, слой Lite уходит
//! в проект RustGIS.
//!
//! Путей два, канал [`Channel`].
//!
//! Сессия расширения ([`Pipe`]) - расширение-процесс RustGIS с ключом
//! `"session": true` (opengeos/RustGIS#1553, запрос opengeos/RustGIS#1518).
//! RustGIS держит программу запущенной, связь - её стандартные ввод
//! и вывод, строками JSON. Ни порта, ни ключа: другая программа к связи
//! не подключится. Состояние проекта - команда `info.project` (её
//! отвечает и `list_layers`), объекты слоя - `host.features`, без
//! временного файла. Перечитывается проект по событиям RustGIS (`view`,
//! `layers`, `selection`, `project`), а не по опросу. Ввод закрылся -
//! RustGIS закрыт, окно закрывается.
//!
//! Ретранслятор MCP ([`Relay`]) - для RustGIS без сессий и для
//! `--rustgis` (команда «MCP Relay» или флажок «Let your Agents Control
//! RustGIS» помощника RustGIS): 127.0.0.1, порт [`DEFAULT_PORT`] или
//! `RUSTGIS_MCP_PORT`, ключ - файл [`token_path`], его пишет RustGIS.
//! Первая строка - `rustgis/attach` с ключом, дальше - сообщения MCP
//! JSON-RPC по строке, как у `rustgis-cli mcp --attach`. Поток опрашивает
//! `list_layers` каждые [`POLL_SECONDS`]: событий ретранслятор не шлёт.
//! Объекты слоя пишет `data.convert` GeoJSON во временную папку: ответ
//! команды длиной ограничен.
//!
//! Объекты слоя читаются заново, когда меняется их ключ
//! ([`data::rustgis::Layer::data_key`]). Стиль - `share.webStyle` файлом,
//! только при смене отрисовки. Только программа: у страницы нет ни
//! сокетов, ни файлов.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
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
    /// Растр слоя - тайлами в оформлении RustGIS до уровня max_zoom, как
    /// QGIS рисует растр модулю (`imagery.hostedImagery`).
    /// RustGIS ищет растр по названию, а не по номеру.
    Raster { layer: i64, name: String, max_zoom: u32 },
    /// Видимость тайлового слоя на карте RustGIS (`map.setTiles`).
    TilesVisibility { id: String, visible: bool },
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
    /// Тайлы растра слоя layer нарезаны: шаблон адреса file:/// с {z},
    /// {x} и {y} и глубший уровень.
    Tiles { layer: i64, template: String, max_zoom: u32 },
    /// Нарезка растра layer не удалась: следующая отметка просит снова.
    CutFailed { layer: i64, error: String },
    /// Просьба или чтение слоя не удались.
    Failed(String),
    /// Команда расширения пришла снова: окно - вперёд.
    Raise,
    /// Сессия расширения кончилась: RustGIS закрыт или остановил
    /// расширение. Окно закрывается.
    Gone(String),
}

/// Связь с RustGIS: поток и его каналы. Остановка - при удалении.
pub struct Link {
    asks: Sender<Ask>,
    news: Receiver<News>,
    stop: Arc<AtomicBool>,
}

impl Link {
    /// Запустить поток. wake будит окно, когда пришла весть.
    /// Сессия расширения, заданная [`set_session`], - связь через неё,
    /// иначе через ретранслятор MCP.
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Link {
        let (asks, ask_rx) = channel();
        let (news_tx, news) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let session = SESSION.lock().ok().and_then(|mut s| s.take());
        std::thread::spawn(move || match session {
            Some((input, output)) => serve_session(Pipe::start(input, output), &ask_rx, &news_tx, &flag, &*wake),
            None => worker(&ask_rx, &news_tx, &flag, &*wake),
        });
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

/// Ввод и вывод сессии расширения до запуска связи.
type Streams = (Box<dyn BufRead + Send>, Box<dyn Write + Send>);

/// Сессия расширения, которую возьмёт [`Link::start`].
static SESSION: Mutex<Option<Streams>> = Mutex::new(None);

/// Связь пойдёт через сессию расширения: input - строки RustGIS после
/// первой, output - строки программы. Так `rustgis-plugin` открывает
/// глобус в своём процессе.
pub fn set_session(input: Box<dyn BufRead + Send>, output: Box<dyn Write + Send>) {
    if let Ok(mut s) = SESSION.lock() {
        *s = Some((input, output));
    }
}

/// Первая строка RustGIS у `rustgis-plugin`: начало сессии
/// `{"session": {"abi", "plugin", "rustgis", "lang"}}` или запрос
/// команды одного запуска.
#[derive(Debug, Clone, PartialEq)]
pub enum Hello {
    /// Сессия, код языка RustGIS.
    Session { lang: String },
    /// Запрос одного запуска: RustGIS без сессий.
    Request,
}

/// Разобрать первую строку RustGIS. Строка может начинаться с метки
/// порядка байтов UTF-8.
pub fn hello(line: &str) -> Hello {
    let value: serde_json::Value = serde_json::from_str(line.trim_start_matches('\u{feff}').trim()).unwrap_or_default();
    match value.get("session") {
        Some(s) if s.is_object() => Hello::Session { lang: s["lang"].as_str().unwrap_or("en").to_string() },
        _ => Hello::Request,
    }
}

/// Канал связи с RustGIS: ретранслятор MCP или сессия расширения.
trait Channel {
    /// Команда RustGIS: текст ответа.
    fn command(&mut self, command: &str, params: serde_json::Value) -> Result<String, String>;
    /// Состояние проекта текстом ответа `list_layers`.
    fn state(&mut self) -> Result<String, String>;
    /// Объекты слоя id.
    fn features(&mut self, id: i64) -> Result<LayerData, String>;
    /// Нужно ли перечитать проект. У ретранслятора - всегда: событий нет.
    fn changed(&mut self) -> bool;
    /// Ждать до seconds секунд. false - поток остановлен.
    fn wait(&mut self, seconds: f64, stop: &AtomicBool) -> bool;
    /// Вести окну, накопленные каналом.
    fn news(&mut self) -> Vec<News> {
        Vec::new()
    }
    /// Связь оборвалась: причина.
    fn broken(&self) -> Option<String> {
        None
    }
}

/// Соединение MCP с ретранслятором.
pub struct Relay {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next: u64,
}

impl Channel for Relay {
    fn command(&mut self, command: &str, params: serde_json::Value) -> Result<String, String> {
        self.tool("run_command", serde_json::json!({"command": command, "params": params}))
    }

    fn state(&mut self) -> Result<String, String> {
        self.tool("list_layers", serde_json::json!({"include_fields": false}))
    }

    /// `data.convert` пишет GeoJSON во временный файл: ответ команды
    /// длиной ограничен.
    fn features(&mut self, id: i64) -> Result<LayerData, String> {
        let path = temp_dir().join(format!("layer-{id}.geojson"));
        self.command("data.convert", serde_json::json!({"layer": id, "format": "geojson", "path": slash(&path)}))?;
        first_layer(take_file(&path)?)
    }

    fn changed(&mut self) -> bool {
        true
    }

    fn wait(&mut self, seconds: f64, stop: &AtomicBool) -> bool {
        nap(seconds, stop)
    }
}

/// Первый слой текста GeoJSON.
fn first_layer(bytes: Vec<u8>) -> Result<LayerData, String> {
    let mut layers = data::layers::read(bytes, "layer.geojson")?;
    if layers.is_empty() {
        return Err("no features".to_string());
    }
    Ok(layers.remove(0))
}

/// Что поток чтения сессии передаёт потоку связи.
#[derive(Debug, Clone, PartialEq)]
enum Signal {
    /// Ответ RustGIS на запрос с номером id.
    Reply { id: String, result: Result<serde_json::Value, String> },
    /// Карта, слои, выделение или проект изменились.
    Event,
    /// Команда «globe» пришла снова.
    Raise,
    /// Ввод закрылся: RustGIS закрыт.
    Gone,
}

/// Строка RustGIS в сессии: сигнал потоку связи и ответ RustGIS, если
/// это команда. Строка считается враждебной: не JSON - пропуск.
fn session_line(line: &str) -> (Option<Signal>, Option<serde_json::Value>) {
    let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(line.trim_start_matches('\u{feff}').trim()) else {
        return (None, None);
    };
    let id = m.get("id").cloned().unwrap_or(serde_json::Value::Null);
    if let Some(request) = m.get("command") {
        // Глобус уже открыт: команда выносит окно вперёд.
        return match request["id"].as_str().unwrap_or("") {
            "globe" => (Some(Signal::Raise), Some(serde_json::json!({"ok": {"started": true, "session": true}, "id": id}))),
            other => (None, Some(serde_json::json!({"error": format!("PlanetX: no command '{other}'"), "id": id}))),
        };
    }
    if m.contains_key("event") {
        return (Some(Signal::Event), None);
    }
    let id = match id {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => return (None, None),
        other => other.to_string(),
    };
    if let Some(e) = m.get("error") {
        let text = e.as_str().map_or_else(|| e.to_string(), str::to_string);
        return (Some(Signal::Reply { id, result: Err(text) }), None);
    }
    match m.get("ok") {
        Some(v) => (Some(Signal::Reply { id, result: Ok(v.clone()) }), None),
        None => (None, None),
    }
}

/// Строку JSON - в вывод сессии.
fn write_line(out: &Mutex<Box<dyn Write + Send>>, msg: &serde_json::Value) -> Result<(), String> {
    let mut out = out.lock().map_err(|e| e.to_string())?;
    writeln!(out, "{msg}").and_then(|()| out.flush()).map_err(|e| e.to_string())
}

/// Поток чтения сессии: строки RustGIS - сигналами, на команды - ответ.
fn listen(mut input: Box<dyn BufRead + Send>, out: &Mutex<Box<dyn Write + Send>>, signals: &Sender<Signal>) {
    loop {
        let mut line = String::new();
        match std::io::Read::take(&mut input, MAX_LINE as u64).read_line(&mut line) {
            Ok(0) | Err(_) => {
                let _ = signals.send(Signal::Gone);
                return;
            }
            Ok(_) => {}
        }
        let (signal, answer) = session_line(&line);
        if let Some(answer) = answer
            && write_line(out, &answer).is_err()
        {
            let _ = signals.send(Signal::Gone);
            return;
        }
        if let Some(signal) = signal
            && signals.send(signal).is_err()
        {
            return;
        }
    }
}

/// Проект перечитывается и без событий раз в столько секунд: событие
/// могло потеряться.
const SESSION_REREAD_SECONDS: f64 = 30.0;
/// Причина конца сессии.
pub const SESSION_GONE: &str = "RustGIS closed the plugin session";

/// Сессия расширения: запросы RustGIS строками на вывод, ответы
/// и события - сигналами потока чтения.
pub struct Pipe {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    signals: Receiver<Signal>,
    next: u64,
    /// Было событие или проект ещё не читался.
    dirty: bool,
    /// Когда проект читался.
    read_at: Option<web_time::Instant>,
    news: Vec<News>,
    gone: bool,
}

impl Pipe {
    /// Запустить поток чтения сессии.
    fn start(input: Box<dyn BufRead + Send>, output: Box<dyn Write + Send>) -> Pipe {
        let out = Arc::new(Mutex::new(output));
        let (tx, signals) = channel();
        let writer = out.clone();
        std::thread::spawn(move || listen(input, &writer, &tx));
        Pipe { out, signals, next: 1, dirty: true, read_at: None, news: Vec::new(), gone: false }
    }

    /// Сигнал не про ответ: событие, окно вперёд, конец сессии.
    fn note(&mut self, signal: Signal) {
        match signal {
            Signal::Event => self.dirty = true,
            Signal::Raise => self.news.push(News::Raise),
            Signal::Gone => self.gone = true,
            Signal::Reply { .. } => {}
        }
    }

    /// Запрос RustGIS `{"call": …, "id": …}` и его ответ. События,
    /// пришедшие до ответа, запоминаются.
    fn call(&mut self, op: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        if self.gone {
            return Err(SESSION_GONE.to_string());
        }
        let id = format!("px{}", self.next);
        self.next += 1;
        write_line(&self.out, &serde_json::json!({"call": {"op": op, "params": params}, "id": id}))?;
        let until = web_time::Instant::now() + Duration::from_secs(REPLY_SECONDS);
        loop {
            let left = until.saturating_duration_since(web_time::Instant::now());
            match self.signals.recv_timeout(left) {
                Ok(Signal::Reply { id: got, result }) if got == id => return result,
                Ok(signal) => {
                    self.note(signal);
                    if self.gone {
                        return Err(SESSION_GONE.to_string());
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(format!("RustGIS did not answer {op}")),
                Err(RecvTimeoutError::Disconnected) => {
                    self.gone = true;
                    return Err(SESSION_GONE.to_string());
                }
            }
        }
    }
}

/// Ответ RustGIS текстом: строка как есть, прочее - JSON.
fn answer_text(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Channel for Pipe {
    fn command(&mut self, command: &str, params: serde_json::Value) -> Result<String, String> {
        self.call(command, params).map(answer_text)
    }

    /// `info.project` - та же команда, что отвечает `list_layers`.
    fn state(&mut self) -> Result<String, String> {
        self.read_at = Some(web_time::Instant::now());
        self.call("info.project", serde_json::json!({"includeFields": false})).map(answer_text)
    }

    /// `host.features` отвечает GeoJSON в WGS 84 сразу, без файла.
    fn features(&mut self, id: i64) -> Result<LayerData, String> {
        let geojson = self.call("host.features", serde_json::json!({"layer": id}))?;
        first_layer(geojson.to_string().into_bytes())
    }

    fn changed(&mut self) -> bool {
        let stale = self.read_at.is_none_or(|t| t.elapsed().as_secs_f64() >= SESSION_REREAD_SECONDS);
        std::mem::take(&mut self.dirty) || stale
    }

    /// Ждать события до seconds секунд, просыпаясь на остановку.
    fn wait(&mut self, seconds: f64, stop: &AtomicBool) -> bool {
        let until = web_time::Instant::now() + Duration::from_secs_f64(seconds.max(0.0));
        while !self.gone && !self.dirty && self.news.is_empty() {
            if stop.load(Ordering::SeqCst) {
                return false;
            }
            let left = until.saturating_duration_since(web_time::Instant::now());
            if left.is_zero() {
                break;
            }
            match self.signals.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(signal) => self.note(signal),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.gone = true,
            }
        }
        !stop.load(Ordering::SeqCst)
    }

    fn news(&mut self) -> Vec<News> {
        std::mem::take(&mut self.news)
    }

    fn broken(&self) -> Option<String> {
        self.gone.then(|| SESSION_GONE.to_string())
    }
}

impl Relay {
    fn open(port: u16) -> Result<Relay, String> {
        let token = std::fs::read_to_string(token_path(port)).map_err(|_| RELAY_OFF.to_string())?;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(REPLY_SECONDS))).map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut s = Relay { writer, reader: BufReader::new(stream), next: 1 };
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
fn read_style(s: &mut dyn Channel, id: i64) -> Result<Style, String> {
    let path = temp_dir().join(format!("style-{id}.json"));
    s.command("share.webStyle", serde_json::json!({"layer": id, "path": slash(&path)}))?;
    let bytes = take_file(&path)?;
    Ok(data::rustgis::parse_style(&String::from_utf8_lossy(&bytes)))
}

/// Выполнить просьбу окна. Ok - текст для строки сообщения.
fn perform(s: &mut dyn Channel, ask: Ask) -> Result<String, String> {
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
        Ask::TilesVisibility { id, visible } => s.command("map.setTiles", serde_json::json!({"id": id, "visible": visible})).map(|_| String::new()),
        // Нарезку выполняет cut_raster, весть у него своя.
        Ask::Raster { .. } => Ok(String::new()),
    }
}

/// Номер нарезки: у каждой своя папка, кэш тайлов глобуса не путает
/// прежнее оформление растра с новым.
static CUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Растр слоя layer - тайлами в оформлении RustGIS: архив
/// `imagery.hostedImagery` распаковывается во временную папку, тайлы
/// `z/x/y.png` берёт глобус. Ok - шаблон адреса file:/// и глубший
/// уровень.
fn cut_raster(s: &mut dyn Channel, layer: i64, name: &str, max_zoom: u32) -> Result<(String, u32), String> {
    let zip_path = temp_dir().join(format!("raster-{layer}.zip"));
    s.command("imagery.hostedImagery", serde_json::json!({"layer": name, "path": slash(&zip_path), "minZoom": 0, "maxZoom": max_zoom}))?;
    let bytes = take_file(&zip_path)?;
    let n = CUTS.fetch_add(1, Ordering::SeqCst);
    let dir = temp_dir().join(format!("raster-{layer}-{}-{n}", std::process::id()));
    // Прежние нарезки этого слоя не нужны.
    if let Ok(entries) = std::fs::read_dir(temp_dir()) {
        let prefix = format!("raster-{layer}-");
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) && e.path().is_dir() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let deepest = unpack_tiles(&bytes, &dir)?;
    let root = slash(&dir);
    let root = root.trim_start_matches('/');
    Ok((format!("file:///{root}/{{z}}/{{x}}/{{y}}.png"), deepest))
}

/// Наибольшее количество тайлов в архиве нарезки.
const MAX_TILES: usize = 1 << 16;

/// Тайлы `z/x/y.png` архива нарезки - в папку dir как `dir/z/x/y.png`.
/// Архив считается враждебным: имена - только числа, размер тайла
/// ограничен. Ok - глубший уровень.
fn unpack_tiles(bytes: &[u8], dir: &std::path::Path) -> Result<u32, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut deepest = None;
    let mut count = 0usize;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        // Имя не в UTF-8 - не тайл.
        let Ok(name) = file.name().map(|n| n.replace('\\', "/")) else {
            continue;
        };
        let Some((z, x, y)) = tile_name(&name) else {
            continue;
        };
        count += 1;
        if count > MAX_TILES {
            break;
        }
        let mut data = Vec::new();
        std::io::Read::take(&mut file, 4 << 20).read_to_end(&mut data).map_err(|e| e.to_string())?;
        let folder = dir.join(z.to_string()).join(x.to_string());
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        std::fs::write(folder.join(format!("{y}.png")), &data).map_err(|e| e.to_string())?;
        deepest = Some(deepest.map_or(z, |d: u32| d.max(z)));
    }
    deepest.ok_or_else(|| "the raster gave no tiles".to_string())
}

/// Уровень, столбец и строка тайла по имени `…/z/x/y.png` в архиве.
fn tile_name(name: &str) -> Option<(u32, u32, u32)> {
    let mut parts = name.strip_suffix(".png")?.rsplit('/');
    let y: u32 = parts.next()?.parse().ok()?;
    let x: u32 = parts.next()?.parse().ok()?;
    let z: u32 = parts.next()?.parse().ok()?;
    let side = 1u64 << z.min(30);
    (z <= 22 && u64::from(x) < side && u64::from(y) < side).then_some((z, x, y))
}

/// Слой текстом GeoJSON в проект RustGIS. Строка ретранслятора - до 64 МБ.
fn add_text(s: &mut dyn Channel, name: &str, geojson: &str) -> Result<(), String> {
    if geojson.len() > MAX_OUT {
        return Err(format!("{name}: the layer is larger than {} MB", MAX_OUT >> 20));
    }
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    s.command("layer.addDataText", serde_json::json!({"fileName": format!("{file}.geojson"), "text": geojson, "name": name})).map(|_| ())
}

/// Расчёт - в проект RustGIS, `results.add_group` модуля: слои, затем
/// подгруппа с ними в группе top. Группы top ещё нет - она заводится.
/// Ok - название подгруппы.
fn perform_result(s: &mut dyn Channel, top: &str, group: &str, vectors: &[(String, String)], rasters: &[(String, String)]) -> Result<String, String> {
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

/// Связь через канал s, пока он жив: просьбы окна, состояние проекта,
/// объекты и стили слоёв. Ok - поток остановлен, Err - связь оборвалась.
fn serve(s: &mut dyn Channel, asks: &Receiver<Ask>, stop: &AtomicBool, tell: &dyn Fn(News)) -> Result<(), String> {
    let mut data_keys: HashMap<i64, String> = HashMap::new();
    let mut styles: HashMap<i64, (String, Style)> = HashMap::new();
    loop {
        loop {
            match asks.try_recv() {
                Ok(Ask::AddResult { top, group, vectors, rasters }) => match perform_result(s, &top, &group, &vectors, &rasters) {
                    Ok(name) => tell(News::Added(name)),
                    Err(e) => tell(News::Failed(e)),
                },
                Ok(Ask::Raster { layer, name, max_zoom }) => match cut_raster(s, layer, &name, max_zoom) {
                    Ok((template, max_zoom)) => tell(News::Tiles { layer, template, max_zoom }),
                    Err(error) => tell(News::CutFailed { layer, error: format!("{name}: {error}") }),
                },
                Ok(ask) => match perform(s, ask) {
                    Ok(text) => {
                        if !text.is_empty() {
                            tell(News::Done(text));
                        }
                    }
                    Err(e) => tell(News::Failed(e)),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        for n in s.news() {
            tell(n);
        }
        if let Some(e) = s.broken() {
            return Err(e);
        }
        if s.changed() {
            let text = s.state()?;
            if let Some(state) = data::rustgis::parse_state(&text) {
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
                        let style = read_style(s, layer.id).unwrap_or_default();
                        styles.insert(layer.id, (style_key, style));
                    }
                    match s.features(layer.id) {
                        Ok(data) => {
                            let style = styles.get(&layer.id).map(|(_, st)| st.clone()).unwrap_or_default();
                            tell(News::Layer { id: layer.id, data: Box::new(data), style: Box::new(style) });
                        }
                        Err(e) => tell(News::Failed(format!("{}: {e}", layer.name))),
                    }
                    // Слой, который не прочёлся, второй раз не просится,
                    // пока не изменится.
                    data_keys.insert(layer.id, layer.data_key());
                }
            }
        }
        if !s.wait(POLL_SECONDS, stop) {
            return Ok(());
        }
    }
}

/// Связь через сессию расширения. Сессия кончилась - окну весть
/// [`News::Gone`], второй сессии не будет.
fn serve_session(mut pipe: Pipe, asks: &Receiver<Ask>, news: &Sender<News>, stop: &AtomicBool, wake: &(dyn Fn() + Send + Sync)) {
    let tell = |n: News| {
        let _ = news.send(n);
        wake();
    };
    let _ = std::fs::create_dir_all(temp_dir());
    tell(News::Connected);
    if let Err(e) = serve(&mut pipe, asks, stop, &tell) {
        tell(News::Gone(e));
    }
}

/// Связь через ретранслятор MCP: подключение, пока он не включён,
/// и снова после обрыва.
fn worker(asks: &Receiver<Ask>, news: &Sender<News>, stop: &AtomicBool, wake: &(dyn Fn() + Send + Sync)) {
    let tell = |n: News| {
        let _ = news.send(n);
        wake();
    };
    let _ = std::fs::create_dir_all(temp_dir());
    let mut last_wait = String::new();
    while !stop.load(Ordering::SeqCst) {
        let mut s = match Relay::open(port()) {
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
        match serve(&mut s, asks, stop, &tell) {
            Ok(()) => return,
            Err(failed) => tell(News::Waiting(failed)),
        }
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

/// Ключ строки раздела «Проект RustGIS»: слой карты по номеру или
/// тайловый слой по ключу RustGIS.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Layer(i64),
    Tiles(String),
}

/// Вид слоя в строке, как `layer_kind` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Points,
    Lines,
    Polygons,
    Raster,
    Tiles,
    /// Таблица и прочее: на глобус не идёт.
    Table,
}

impl RowKind {
    /// Слово вида после названия, ключ перевода.
    pub fn word(self) -> &'static str {
        match self {
            RowKind::Points => "points",
            RowKind::Lines => "lines",
            RowKind::Polygons => "polygons",
            RowKind::Raster => "raster",
            RowKind::Tiles => "tiles",
            RowKind::Table => "table",
        }
    }

    /// Слой рисуется картинкой тайлов, а не объектами.
    pub fn is_picture(self) -> bool {
        matches!(self, RowKind::Raster | RowKind::Tiles)
    }
}

/// Строка раздела «Проект RustGIS»: все слои проекта в порядке карты,
/// как список «Проект QGIS» модуля.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: RowKey,
    pub name: String,
    pub kind: RowKind,
    /// Охват в градусах для перелёта.
    pub extent: Option<[f64; 4]>,
    /// Слой включён на карте RustGIS.
    pub on_map: bool,
    /// Ключ оформления: другой - растр режется заново.
    pub style: String,
}

/// Тайлов глубшего уровня в одной нарезке растра не больше: RustGIS
/// режет их, держа своё окно, около 50 тайлов в секунду (6 ноября 2026
/// года, RustGIS 0.5.0: 2913 тайлов - минута).
pub const CUT_TILES: f64 = 1024.0;
/// Глубший уровень нарезки растра.
pub const CUT_LEVEL: u32 = 18;

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
    /// Все слои проекта в порядке карты, тайловые - под слоями.
    pub rows: Vec<Row>,
    /// Тайловые слои по ключу.
    pub tiles: HashMap<String, data::rustgis::TileLayer>,
    /// Растры и тайловые слои, заведённые строками карт глобуса: номер
    /// строки [`crate::maps`] и глубший уровень.
    pub pictures: HashMap<RowKey, usize>,
    /// Растры и тайловые слои, которые должны быть на глобусе.
    pub pictures_on: std::collections::HashSet<RowKey>,
    /// Растры, нарезка которых попрошена: ключ оформления.
    pub cut: HashMap<i64, String>,
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
    /// Режим «Слои как на карте RustGIS», по которому расставлены флажки
    /// глобуса. Другой в настройках - флажки расставляются заново.
    pub follow: Option<bool>,
    /// Открытое меню строки раздела «Проект RustGIS».
    pub menu: Option<String>,
    /// Прозрачность слоя для карты RustGIS, в процентах: уходит, когда
    /// ползунок отпущен, а не на каждом кадре протяжки.
    pub fade: Option<(i64, u8)>,
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
            rows: Vec::new(),
            tiles: HashMap::new(),
            pictures: HashMap::new(),
            pictures_on: std::collections::HashSet::new(),
            cut: HashMap::new(),
            last_map: None,
            last_sent: None,
            last_globe: None,
            moved_at: 0.0,
            pending: false,
            sync: false,
            direction: Direction::Both,
            push_now: false,
            follow: None,
            menu: None,
            fade: None,
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
        self.rows = state
            .layers
            .iter()
            .map(|l| Row {
                key: RowKey::Layer(l.id),
                name: l.name.clone(),
                kind: match l.kind {
                    data::rustgis::Kind::Point => RowKind::Points,
                    data::rustgis::Kind::Line => RowKind::Lines,
                    data::rustgis::Kind::Polygon => RowKind::Polygons,
                    data::rustgis::Kind::Raster => RowKind::Raster,
                    data::rustgis::Kind::Other => RowKind::Table,
                },
                extent: l.extent,
                on_map: l.visible,
                style: l.style_key(),
            })
            .chain(state.tiles.iter().map(|t| Row { key: RowKey::Tiles(t.id.clone()), name: t.name.clone(), kind: RowKind::Tiles, extent: None, on_map: t.visible, style: t.url.clone() }))
            .collect();
        self.tiles = state.tiles.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let other = self.project != state.project;
        if other {
            self.pictures_on.clear();
        }
        self.project.clone_from(&state.project);
        other
    }

    /// Должна ли строка быть на глобусе: в режиме «Слои как на карте
    /// RustGIS» (follow) - как на карте, иначе - отмечена ли она
    /// в настройках проекта (saved - названия слоёв).
    pub fn wanted_row(row: &Row, follow: bool, saved: &[String]) -> bool {
        if follow { row.on_map } else { saved.contains(&row.name) }
    }

    /// Строка по ключу.
    pub fn row(&self, key: &RowKey) -> Option<&Row> {
        self.rows.iter().find(|r| r.key == *key)
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
/// rustgis-plugin`, RustGIS без сессий ([`Hello::Request`]): программа
/// запускается на каждую команду. line - строка запроса RustGIS, уже
/// прочитанная, дальше строки RustGIS - на input, ответ - строками JSON
/// на output: сообщение строки состояния RustGIS запросом `host.notify`
/// и итог. launch запускает глобус со связью отдельным процессом.
pub fn plugin(line: &str, input: &mut dyn BufRead, output: &mut dyn Write, relay_on: bool, launch: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
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
            assert_eq!(hello(request), Hello::Request);
            let mut input = std::io::Cursor::new("{\"ok\":null}\n".to_string());
            let mut out = Vec::new();
            let launch = move || if ok { Ok(()) } else { Err("no file".to_string()) };
            let _ = plugin(request, &mut input, &mut out, relay, &launch);
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

    /// Первая строка RustGIS: сессия или запрос одного запуска.
    #[test]
    fn hello_line() {
        assert_eq!(hello("\u{feff}{\"session\":{\"abi\":1,\"plugin\":\"rustplanetx\",\"rustgis\":\"0.6.0\",\"lang\":\"ru\"}}\n"), Hello::Session { lang: "ru".into() });
        assert_eq!(hello(r#"{"abi":1,"kind":"command","id":"globe","params":{}}"#), Hello::Request);
        assert_eq!(hello(r#"{"session":true}"#), Hello::Request);
        assert_eq!(hello("not json"), Hello::Request);
    }

    /// Строки RustGIS в сессии: команда получает ответ со своим номером,
    /// событие и ответ на запрос - сигналы, прочее - пропуск.
    #[test]
    fn session_lines() {
        let (signal, answer) = session_line(r#"{"command":{"abi":1,"kind":"command","id":"globe","params":{}},"id":7}"#);
        assert_eq!(signal, Some(Signal::Raise));
        assert_eq!(answer, Some(serde_json::json!({"ok": {"started": true, "session": true}, "id": 7})));
        let (signal, answer) = session_line(r#"{"command":{"id":"other"},"id":8}"#);
        assert_eq!(signal, None);
        assert!(answer.is_some_and(|a| a["error"].is_string() && a["id"] == 8));
        assert_eq!(session_line(r#"{"event":"layers"}"#), (Some(Signal::Event), None));
        assert_eq!(session_line(r#"{"ok":{"a":1},"id":"px3"}"#), (Some(Signal::Reply { id: "px3".into(), result: Ok(serde_json::json!({"a": 1})) }), None));
        assert_eq!(session_line(r#"{"error":"no edit","id":"px4"}"#), (Some(Signal::Reply { id: "px4".into(), result: Err("no edit".into()) }), None));
        // Ответ без номера - не наш, строки библиотек - не протокол.
        assert_eq!(session_line(r#"{"ok":1}"#), (None, None));
        assert_eq!(session_line("loading tiles..."), (None, None));
        assert_eq!(session_line("[1,2]"), (None, None));
    }

    /// Ввод сессии из канала строк: закрытый канал - конец ввода.
    struct Lines {
        rx: Receiver<String>,
        buf: Vec<u8>,
        pos: usize,
    }

    impl Read for Lines {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.buf.len() {
                match self.rx.recv() {
                    Ok(s) => {
                        self.buf = s.into_bytes();
                        self.pos = 0;
                    }
                    Err(_) => return Ok(0),
                }
            }
            let n = out.len().min(self.buf.len() - self.pos);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// Вывод сессии в канал строк.
    struct Out(Sender<String>, Vec<u8>);

    impl Write for Out {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.1.extend_from_slice(data);
            while let Some(i) = self.1.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.1.drain(..=i).collect();
                let _ = self.0.send(String::from_utf8_lossy(&line).into_owned());
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Ответ имитации RustGIS на запрос программы.
    fn fake_rustgis(line: &str) -> Option<serde_json::Value> {
        let msg: serde_json::Value = serde_json::from_str(line).ok()?;
        let id = msg["id"].clone();
        Some(match msg["call"]["op"].as_str()? {
            "info.project" => serde_json::json!({"ok": {
                "name": "Atlas", "path": null,
                "center": {"lat": 58.0, "lon": 56.0}, "extent": [55.0, 57.5, 57.0, 58.5], "scale": 1.0e6,
                "layers": [{"id": 1, "name": "Cities", "type": "Point", "visible": true, "features": 1}]
            }, "id": id}),
            "host.features" => serde_json::json!({"ok": {"type": "FeatureCollection", "features": [
                {"type": "Feature", "row": 0, "geometry": {"type": "Point", "coordinates": [56.2, 58.0]}, "properties": {"name": "Perm"}}
            ]}, "id": id}),
            // Стиль файлом без разрешения files - отказ, слой рисуется
            // цветом по кругу.
            _ => serde_json::json!({"error": "no permission", "id": id}),
        })
    }

    /// Сессия расширения на имитации RustGIS: состояние и объекты слоя
    /// без ретранслятора, проект перечитывается по событию, повторная
    /// команда «globe» выносит окно вперёд, закрытый ввод закрывает окно.
    #[test]
    fn session_serves_the_map() {
        let (to_program, program_in) = channel::<String>();
        let (program_out, from_program) = channel::<String>();
        let input: Box<dyn BufRead + Send> = Box::new(BufReader::new(Lines { rx: program_in, buf: Vec::new(), pos: 0 }));
        let output: Box<dyn Write + Send> = Box::new(Out(program_out, Vec::new()));
        let (asks_tx, asks) = channel::<Ask>();
        let (news_tx, news) = channel::<News>();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = std::thread::spawn(move || serve_session(Pipe::start(input, output), &asks, &news_tx, &flag, &|| {}));
        let wait = Duration::from_secs(10);
        // Имитация RustGIS отвечает, пока не придёт весть what.
        let answer_until = |to: Option<&Sender<String>>, what: &dyn Fn(&News) -> bool| -> (usize, bool) {
            let mut states = 0;
            loop {
                while let Ok(line) = from_program.recv_timeout(Duration::from_millis(20)) {
                    if let (Some(reply), Some(to)) = (fake_rustgis(&line), to) {
                        let _ = to.send(format!("{reply}\n"));
                    }
                }
                match news.recv_timeout(Duration::from_millis(20)) {
                    Ok(n) => {
                        states += usize::from(matches!(n, News::State(_)));
                        if what(&n) {
                            return (states, true);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return (states, false),
                }
            }
        };
        let started = web_time::Instant::now();
        let (states, got) = answer_until(Some(&to_program), &|n| matches!(n, News::Layer { id: 1, data, .. } if data.items.len() == 1));
        assert!(got && states == 1, "{states}");
        assert!(started.elapsed() < wait);
        // Событие - проект перечитывается.
        let _ = to_program.send("{\"event\":\"layers\"}\n".into());
        let (states, got) = answer_until(Some(&to_program), &|n| matches!(n, News::State(_)));
        assert!(got && states == 1);
        // Просьба окна - запрос RustGIS со своим номером.
        asks_tx.send(Ask::Visibility { layer: 1, visible: false }).unwrap_or_default();
        let call = from_program.recv_timeout(wait).unwrap_or_default();
        assert!(call.contains("\"layer.visibility\"") && call.contains("\"id\":\"px"), "{call}");
        let _ = to_program.send(format!("{}\n", serde_json::json!({"ok": null, "id": serde_json::from_str::<serde_json::Value>(&call).unwrap_or_default()["id"]})));
        // Команда «globe» снова: ответ с её номером, окно вперёд.
        let _ = to_program.send("{\"command\":{\"id\":\"globe\",\"params\":{}},\"id\":2}\n".into());
        let (_, got) = answer_until(Some(&to_program), &|n| matches!(n, News::Raise));
        assert!(got);
        // RustGIS закрыт - сессия кончилась, окно закрывается.
        drop(to_program);
        let (_, got) = answer_until(None, &|n| matches!(n, News::Gone(_)));
        assert!(got);
        assert!(worker.join().is_ok());
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
            r#"{"name":"Atlas","path":null,"layers":[{"id":1,"name":"Cities","type":"Point","visible":true},{"id":2,"name":"Lakes","type":"Polygon","visible":false},{"id":3,"name":"DEM","type":"Raster","visible":true,"extent":[56,57,57,58]}],
                "tileLayers":[{"id":"sat","index":0,"kind":"raster","name":"Imagery","url":"https://t.example.org/{z}/{x}/{y}.png","visible":false}]}"#,
        )
        .unwrap_or(data::rustgis::State { view: None, layers: Vec::new(), tiles: Vec::new(), project: String::new() });
        assert!(s.take_state(&state));
        assert!(!s.take_state(&state));
        assert_eq!(s.project, "Atlas");
        // Объекты читаются только у векторов.
        assert_eq!(s.names.len(), 2);
        assert!(s.wanted(1, true, &[]) && !s.wanted(2, true, &[]));
        let saved = vec!["Lakes".to_string()];
        assert!(!s.wanted(1, false, &saved) && s.wanted(2, false, &saved));
        // В строках раздела - все слои в порядке карты, тайловые под ними.
        let kinds: Vec<RowKind> = s.rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RowKind::Points, RowKind::Polygons, RowKind::Raster, RowKind::Tiles]);
        let dem = s.row(&RowKey::Layer(3)).cloned();
        assert!(dem.as_ref().is_some_and(|r| r.kind.is_picture() && r.extent.is_some()));
        assert!(dem.as_ref().is_some_and(|r| Bridge::wanted_row(r, true, &[])));
        let sat = s.row(&RowKey::Tiles("sat".into())).cloned();
        assert!(sat.as_ref().is_some_and(|r| !Bridge::wanted_row(r, true, &[]) && Bridge::wanted_row(r, false, &["Imagery".to_string()])));
        let mut other = state.clone();
        other.project = "C:/gis/other.rgis".into();
        assert!(s.take_state(&other));
    }

    /// Тайлы нарезки растра: только числовые имена z/x/y.png в пределах
    /// уровня, прочее в архиве - пропуск.
    #[test]
    fn raster_tiles_unpack() {
        assert_eq!(tile_name("tiles/3/5/2.png"), Some((3, 5, 2)));
        assert_eq!(tile_name("0/0/0.png"), Some((0, 0, 0)));
        for bad in ["3/8/2.png", "a/1/1.png", "1/1/1.jpg", "tiles.json", "../../1/0/0.png.exe", "1/0.png"] {
            assert_eq!(tile_name(bad), None, "{bad}");
        }
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [("out/0/0/0.png", "a"), ("out/1/1/0.png", "b"), ("out/index.html", "c"), ("out/../../evil/1/0/0.png", "d")] {
            let _ = zip.start_file(name, options);
            let _ = zip.write_all(body.as_bytes());
        }
        let bytes = zip.finish().map(std::io::Cursor::into_inner).unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("planetx-unpack-test-{}", std::process::id()));
        assert_eq!(unpack_tiles(&bytes, &dir), Ok(1));
        assert_eq!(std::fs::read(dir.join("1").join("1").join("0.png")).unwrap_or_default(), b"b");
        // Имя с переходом наверх легло только по своим числам z/x/y.
        assert!(!dir.join("..").join("evil").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
