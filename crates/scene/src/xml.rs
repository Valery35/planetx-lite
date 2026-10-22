// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разбор XML в лёгкое дерево, как `_parse` и `_Node` в `core/kml.py`
//! модуля.
//!
//! Разбор идёт через quick-xml. Имя узла - без пространства имён
//! и приставки: «gx:Tour» становится «Tour». Текст узла - все его
//! символы между детьми подряд, CDATA тоже. Объявления сущностей в DTD
//! запрещены, файл с ними не читается: так чужой файл не раздует
//! память вложенными сущностями. Из ссылок на сущности понимаются
//! только пять сущностей XML и коды символов, прочие - ошибка.
//! Вложенность узлов не больше [`MAX_DEPTH`]: враждебный файл
//! с миллионом вложенных узлов даёт ошибку, а не переполнение стека.
//!
//! Кодировка - UTF-8 или UTF-16 по метке порядка байтов. По объявлению
//! XML понимаются ещё ISO-8859-1 и windows-1251.

use std::fmt;

use quick_xml::XmlVersion;
use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// Наибольшая вложенность узлов.
pub const MAX_DEPTH: usize = 1000;

/// Файл не разбирается как XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError(pub String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

fn error(text: impl fmt::Display) -> XmlError {
    XmlError(text.to_string())
}

/// Узел XML: имя без пространства имён, атрибуты, текст, дети.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub tag: String,
    pub attrib: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

/// Имя без пространства имён «{адрес}» и приставки «gx:».
pub fn local_name(tag: &str) -> &str {
    let tag = tag.rsplit_once('}').map_or(tag, |(_, t)| t);
    tag.rsplit_once(':').map_or(tag, |(_, t)| t)
}

impl Node {
    /// Узел с именем tag без приставки и атрибутами.
    pub fn new(tag: &str, attrib: Vec<(String, String)>) -> Self {
        Self { tag: local_name(tag).to_string(), attrib, text: String::new(), children: Vec::new() }
    }

    /// Значение атрибута name или None.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrib.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Первый ребёнок с именем name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.tag == name)
    }

    /// Все дети с именем name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.tag == name)
    }

    /// Текст ребёнка name без пробелов по краям, пустая строка без него.
    pub fn text_of(&self, name: &str) -> String {
        self.child(name).map_or(String::new(), |c| c.text.trim().to_string())
    }

    /// Сам узел и все его потомки в порядке документа.
    pub fn iter(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            out.push(node);
            stack.extend(node.children.iter().rev());
        }
        out
    }
}

/// windows-1251, знаки 0x80-0xBF. С 0xC0 - буквы А-я подряд.
const CP1251: [u16; 64] = [
    0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409, 0x2039, 0x040A, 0x040C,
    0x040B, 0x040F, 0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0xFFFD, 0x2122, 0x0459, 0x203A,
    0x045A, 0x045C, 0x045B, 0x045F, 0x00A0, 0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9,
    0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407, 0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7,
    0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
];

fn cp1251(byte: u8) -> char {
    let code = match byte {
        0x00..=0x7F => u32::from(byte),
        0x80..=0xBF => u32::from(CP1251[usize::from(byte - 0x80)]),
        _ => 0x0410 + u32::from(byte - 0xC0),
    };
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

/// Кодировка из объявления XML в начале байтов, строчными буквами.
fn declared_encoding(data: &[u8]) -> Option<String> {
    let head = &data[..data.len().min(512)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = String::from_utf8_lossy(&head[..end]).to_lowercase();
    let rest = decl.split_once("encoding")?.1.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    let value = rest.get(1..)?.split(quote).next()?;
    Some(value.to_string())
}

/// Текст из байтов XML или ошибка, если кодировка неверна.
pub fn decode(data: &[u8]) -> Result<String, XmlError> {
    if let Some(rest) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(rest.to_vec()).map_err(error);
    }
    let utf16 = |little: bool, rest: &[u8]| {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| if little { u16::from_le_bytes(*p) } else { u16::from_be_bytes(*p) })
            .collect();
        String::from_utf16(&units).map_err(error)
    };
    if let Some(rest) = data.strip_prefix(b"\xFF\xFE") {
        return utf16(true, rest);
    }
    if let Some(rest) = data.strip_prefix(b"\xFE\xFF") {
        return utf16(false, rest);
    }
    match declared_encoding(data).as_deref() {
        Some("iso-8859-1" | "latin-1" | "latin1") if std::str::from_utf8(data).is_err() => {
            Ok(data.iter().map(|b| char::from(*b)).collect())
        }
        Some("windows-1251" | "cp1251") if std::str::from_utf8(data).is_err() => {
            Ok(data.iter().map(|b| cp1251(*b)).collect())
        }
        _ => String::from_utf8(data.to_vec()).map_err(error),
    }
}

fn node_of(start: &BytesStart<'_>) -> Result<Node, XmlError> {
    let mut attrib = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(error)?;
        let value = attr.normalized_value_with(XmlVersion::Implicit1_0, 1, resolve_xml_entity).map_err(error)?;
        attrib.push((attr.key.0.to_string(), value.into_owned()));
    }
    Ok(Node::new(start.name().0, attrib))
}

/// Корень дерева из байтов XML.
pub fn parse(data: &[u8]) -> Result<Node, XmlError> {
    parse_str(&decode(data)?)
}

/// Корень дерева из текста XML.
pub fn parse_str(text: &str) -> Result<Node, XmlError> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let attach = |node: Node, stack: &mut Vec<Node>, root: &mut Option<Node>| -> Result<(), XmlError> {
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None if root.is_none() => *root = Some(node),
            None => return Err(error("junk after document element")),
        }
        Ok(())
    };
    let add_text = |piece: &str, stack: &mut Vec<Node>| -> Result<(), XmlError> {
        match stack.last_mut() {
            Some(node) => node.text.push_str(piece),
            None if piece.trim().is_empty() => {}
            None => return Err(error("text outside the document element")),
        }
        Ok(())
    };
    loop {
        match reader.read_event().map_err(error)? {
            Event::Start(start) => {
                if stack.is_empty() && root.is_some() {
                    return Err(error("junk after document element"));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err(error("XML is nested too deep"));
                }
                stack.push(node_of(&start)?);
            }
            Event::Empty(start) => {
                let node = node_of(&start)?;
                attach(node, &mut stack, &mut root)?;
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| error("unexpected end tag"))?;
                attach(node, &mut stack, &mut root)?;
            }
            Event::Text(t) => add_text(&t.xml10_content(), &mut stack)?,
            Event::CData(c) => add_text(&c.xml10_content(), &mut stack)?,
            Event::GeneralRef(r) => {
                let piece = match r.resolve_char_ref().map_err(error)? {
                    Some(c) => c.to_string(),
                    None => resolve_xml_entity(&r).ok_or_else(|| error(format!("undefined entity &{};", &*r)))?.to_string(),
                };
                add_text(&piece, &mut stack)?;
            }
            Event::DocType(d) => {
                if d.contains("<!ENTITY") {
                    return Err(error("DTD entities are not allowed"));
                }
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(error("unclosed element"));
    }
    root.ok_or_else(|| error("empty XML"))
}
