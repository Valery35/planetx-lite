// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ретранслятор MCP: агент управляет открытым глобусом программы.
//! Образец - ретранслятор RustGIS (`crates/assistant/src/relay.rs`,
//! Apache 2.0), без HTTP, грантов и журнала.
//!
//! Выключен, пока в «Настройках помощника» не стоит флажок «Пускать
//! агентов MCP». Слушает только 127.0.0.1, порт [`DEFAULT_PORT`] или
//! `PLANETX_MCP_PORT`. Ключ - 32 знака hex из случайных ключей хеша
//! системы, в файле [`token_path`] папки программы, в командной строке
//! он не появляется. Первая строка соединения - `planetx/attach`
//! с ключом, в течение [`ATTACH_SECONDS`], иначе соединение закрывается.
//! Дальше - сообщения JSON-RPC по строке, каждое уходит в кадр окна
//! ([`Relay::take`]), ответ ждётся до [`REPLY_SECONDS`]. Клиент
//! подключается командой `planetx-lite mcp`: она соединяет его stdio
//! с этим сокетом. Только программа: у страницы входящих соединений нет.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Порт по умолчанию.
pub const DEFAULT_PORT: u16 = 8770;
/// Секунд на строку подключения с ключом.
pub const ATTACH_SECONDS: u64 = 5;
/// Секунд на ответ окна.
pub const REPLY_SECONDS: u64 = 120;
/// Секунд тишины, после которых соединение закрывается.
pub const IDLE_SECONDS: u64 = 1800;
/// Соединений сразу не больше.
pub const MAX_CLIENTS: usize = 8;
/// Строка сообщения не длиннее, байт.
pub const MAX_LINE: usize = 16 << 20;

/// Порт: `PLANETX_MCP_PORT` или [`DEFAULT_PORT`].
pub fn port() -> u16 {
    std::env::var("PLANETX_MCP_PORT").ok().and_then(|p| p.trim().parse().ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT)
}

/// Файл ключа ретранслятора на порту port: папка программы рядом
/// с кэшем тайлов.
pub fn token_path(port: u16) -> Option<PathBuf> {
    Some(net::native::default_cache_dir()?.parent()?.join(format!("mcp-{port}.token")))
}

/// 32 знака hex из случайных ключей хеша системы.
pub fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    (0..2u8)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(nanos);
            h.write_u8(i);
            h.write_u32(std::process::id());
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Сообщение агента для кадра окна и куда отдать ответ.
pub struct Job {
    pub message: String,
    pub reply: Sender<Option<String>>,
}

/// Работающий ретранслятор. Drop останавливает его и убирает ключ.
pub struct Relay {
    pub port: u16,
    jobs: Receiver<Job>,
    stop: Arc<AtomicBool>,
    clients: Arc<AtomicUsize>,
    token_file: Option<PathBuf>,
    /// Последний вызов агента для строки состояния.
    pub last: Arc<Mutex<Option<String>>>,
}

impl Relay {
    /// Запустить на 127.0.0.1:port с ключом token. token_file - куда
    /// записать ключ, None - не записывать (тесты). wake будит окно.
    pub fn start(port: u16, token: String, token_file: Option<PathBuf>, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Relay, String> {
        let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        if let Some(path) = &token_file {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, &token).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let (send, jobs) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let (s, c, l) = (stop.clone(), clients.clone(), last.clone());
        std::thread::Builder::new()
            .name("mcp-relay".into())
            .spawn(move || accept(listener, token, send, s, c, l, wake))
            .map_err(|e| e.to_string())?;
        Ok(Relay { port, jobs, stop, clients, token_file, last })
    }

    /// Следующее сообщение агента или None.
    pub fn take(&self) -> Option<Job> {
        self.jobs.try_recv().ok()
    }

    /// Подключённых агентов.
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(path) = &self.token_file {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn accept(listener: TcpListener, token: String, send: Sender<Job>, stop: Arc<AtomicBool>, clients: Arc<AtomicUsize>, last: Arc<Mutex<Option<String>>>, wake: Arc<dyn Fn() + Send + Sync>) {
    let open = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if open.load(Ordering::Relaxed) >= MAX_CLIENTS {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                open.fetch_add(1, Ordering::Relaxed);
                let (token, send, stop, clients, open, last, wake) = (token.clone(), send.clone(), stop.clone(), clients.clone(), open.clone(), last.clone(), wake.clone());
                let _ = std::thread::Builder::new().name("mcp-client".into()).spawn(move || {
                    serve(stream, &token, &send, &stop, &clients, &last, &*wake);
                    open.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Прочитать строку не длиннее [`MAX_LINE`]. None - конец или ошибка.
fn read_line(reader: &mut BufReader<TcpStream>, stop: &AtomicBool) -> Option<String> {
    let mut buf = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => return None,
            Ok(_) if buf.ends_with(b"\n") => return Some(String::from_utf8_lossy(&buf).trim().to_string()),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return None,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
        if buf.len() > MAX_LINE {
            return None;
        }
    }
}

fn serve(stream: TcpStream, token: &str, send: &Sender<Job>, stop: &AtomicBool, clients: &AtomicUsize, last: &Mutex<Option<String>>, wake: &(dyn Fn() + Send + Sync)) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(ATTACH_SECONDS)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let attached = read_line(&mut reader, stop).and_then(|l| assistant::mcp::attach_token(&l)).is_some_and(|t| assistant::mcp::same_token(&t, token));
    if !attached {
        let _ = writer.shutdown(Shutdown::Both);
        return;
    }
    let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(IDLE_SECONDS)));
    clients.fetch_add(1, Ordering::Relaxed);
    wake();
    while let Some(line) = read_line(&mut reader, stop) {
        if line.is_empty() {
            continue;
        }
        if let Ok(mut slot) = last.lock() {
            *slot = serde_json::from_str::<serde_json::Value>(&line).ok().and_then(|v| v.pointer("/params/name").or_else(|| v.get("method")).and_then(|n| n.as_str()).map(str::to_string));
        }
        let wants = assistant::mcp::expects_reply(&line);
        let (reply, answer) = channel();
        if send.send(Job { message: line, reply }).is_err() {
            break;
        }
        wake();
        if !wants {
            continue;
        }
        let out = match answer.recv_timeout(Duration::from_secs(REPLY_SECONDS)) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(_) => "{\"jsonrpc\": \"2.0\", \"id\": null, \"error\": {\"code\": -32002, \"message\": \"PlanetX Lite did not answer in time\"}}".to_string(),
        };
        if writer.write_all(out.as_bytes()).and_then(|()| writer.write_all(b"\n")).and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
    clients.fetch_sub(1, Ordering::Relaxed);
    wake();
}

/// Мост stdio клиента и ретранслятора открытой программы - режим
/// `planetx-lite mcp`. Возвращает ошибку, если программа не слушает.
pub fn bridge(port: u16) -> Result<(), String> {
    let path = token_path(port).ok_or("no data folder")?;
    let token = std::fs::read_to_string(&path).map_err(|_| format!("PlanetX Lite is not letting agents in: no {}. Tick «Let MCP agents in» in the assistant settings.", path.display()))?;
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    let mut to_app = stream.try_clone().map_err(|e| e.to_string())?;
    to_app.write_all(assistant::mcp::attach_line(token.trim()).as_bytes()).and_then(|()| to_app.write_all(b"\n")).map_err(|e| e.to_string())?;
    // Ответы программы - в стандартный вывод.
    let from_app = std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if out.write_all(line.as_bytes()).and_then(|()| out.write_all(b"\n")).and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    });
    // Сообщения клиента - в программу.
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if to_app.write_all(line.as_bytes()).and_then(|()| to_app.write_all(b"\n")).is_err() {
            break;
        }
    }
    let _ = to_app.shutdown(Shutdown::Write);
    let _ = from_app.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn connect(port: u16) -> (BufReader<TcpStream>, TcpStream) {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        (BufReader::new(s.try_clone().unwrap()), s)
    }

    #[test]
    fn relay_runs_messages_with_the_token_only() {
        let relay = Relay::start(0, "k1".into(), None, Arc::new(|| {})).unwrap();
        // Без ключа соединение закрывается.
        let (mut r, mut w) = connect(relay.port);
        w.write_all(b"{\"jsonrpc\": \"2.0\", \"id\": 1, \"method\": \"ping\"}\n").unwrap();
        let mut line = String::new();
        assert_eq!(r.read_line(&mut line).unwrap_or(0), 0);
        // С ключом сообщение доходит до окна, ответ - до клиента.
        let (mut r, mut w) = connect(relay.port);
        w.write_all(format!("{}\n", assistant::mcp::attach_line("k1")).as_bytes()).unwrap();
        w.write_all(b"{\"jsonrpc\": \"2.0\", \"id\": 7, \"method\": \"ping\"}\n").unwrap();
        let job = (0..200).find_map(|_| {
            std::thread::sleep(Duration::from_millis(20));
            relay.take()
        });
        let job = job.unwrap();
        assert!(job.message.contains("ping"));
        assert_eq!(relay.clients(), 1);
        job.reply.send(Some("{\"jsonrpc\": \"2.0\", \"id\": 7, \"result\": {}}".into())).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("\"id\": 7"), "{line}");
        assert_eq!(relay.last.lock().unwrap().as_deref(), Some("ping"));
        // Уведомление ответа не ждёт.
        w.write_all(b"{\"jsonrpc\": \"2.0\", \"method\": \"notifications/initialized\"}\n").unwrap();
    }

    #[test]
    fn tokens_differ() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
