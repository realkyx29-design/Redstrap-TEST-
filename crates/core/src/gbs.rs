//! `GlobalBasicSettings_13.xml` reader/writer (quick-xml, no DOM).
//!
//! The file stores typed `<Item class="..."><Property name="...">value`
//! triples. Parsing and writing both stream through quick-xml, replacing
//! only the values whose keys are supplied and appending missing keys just
//! before the closing root tag.

use std::collections::BTreeMap;
use std::path::Path;

use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::QName;
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use crate::error::{Error, Result};

/// Well-known property keys with their owning classes.
pub const PROP_FULLSCREEN: (&str, &str) = ("GraphicsMode", "RenderSettings");
pub const PROP_VSYNC: (&str, &str) = ("VSyncDisabled", "RenderSettings");
pub const PROP_MSAALEVEL: (&str, &str) = ("MSAALevel", "RenderSettings");
pub const PROP_FRAMERATE_CAP: (&str, &str) = ("FramerateCap", "RenderSettings");
pub const PROP_TEXTURE_QUALITY: (&str, &str) = ("TextureQuality", "RenderSettings");
pub const PROP_MESH_CACHE_SIZE: (&str, &str) = ("MeshCacheSize", "RenderSettings");

/// Parsed value of one `<Property>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub class: String,
    pub name: String,
    pub value: String,
}

/// Parse every `<Item class><Property name>` pair in the document.
/// Unknown structure is skipped rather than failing the whole read.
pub fn read_properties(xml: &str) -> Result<Vec<Property>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out = Vec::new();
    let mut current_class: Option<String> = None;
    let mut current_name: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::GameSettings(format!("could not parse settings XML: {e}")))?;
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if tag.name() == QName(b"Item") {
                    current_class = attr_value(&tag, b"class")?;
                    current_name = None;
                } else if tag.name() == QName(b"Property") {
                    current_name = attr_value(&tag, b"name")?;
                } else {
                    current_name = None;
                }
            }
            Event::Text(text) => {
                if let (Some(class), Some(name)) = (current_class.clone(), current_name.clone()) {
                    let value = text
                        .decode()
                        .map_err(|e| Error::GameSettings(format!("could not decode XML text: {e}")))?
                        .into_owned();
                    out.push(Property { class, name, value });
                    current_name = None;
                }
            }
            Event::End(tag) => {
                if tag.name() == QName(b"Item") {
                    current_class = None;
                }
                current_name = None;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

/// Read properties from a file. A missing file yields an empty list.
pub fn read_file(path: &Path) -> Result<Vec<Property>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::with_path(&path.to_path_buf(), e))?;
    read_properties(&text)
}

/// Write `updates` (`(class, name) -> value`) into the document at `path`,
/// creating a minimal document when the file is missing or unparseable.
pub fn write_properties(path: &Path, updates: &BTreeMap<(String, String), String>) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
    }

    let existing = if path.is_file() {
        std::fs::read_to_string(path).map_err(|e| Error::with_path(&path.to_path_buf(), e))?
    } else {
        String::new()
    };

    let rewritten = if existing.trim().is_empty() {
        build_document(updates)
    } else {
        match merge_document(&existing, updates) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::warn!("settings XML unparseable ({}), rebuilding minimal file", e);
                build_document(updates)
            }
        }
    };

    crate::util::atomic_write(path, rewritten.as_bytes())?;
    Ok(())
}

fn attr_value(tag: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attr in tag.attributes() {
        let attr = attr.map_err(|e| Error::GameSettings(format!("bad XML attribute: {e}")))?;
        if attr.key == QName(key) {
            let value = attr
                .unescape_value()
                .map_err(|e| Error::GameSettings(format!("bad XML attribute value: {e}")))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

/// Minimal but faithful document skeleton for the keys we manage.
fn build_document(updates: &BTreeMap<(String, String), String>) -> String {
    let mut classes: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for ((class, name), value) in updates {
        classes
            .entry(class.as_str())
            .or_default()
            .push((name.as_str(), value.as_str()));
    }

    let mut doc = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<roblox xmlns:xmime=\"http://www.w3.org/2005/05/xmlmime\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"http://www.roblox.com/roblox.xsd\" version=\"4\">\n\t<External>null</External>\n\t<External>nil</External>\n",
    );
    for (class, props) in &classes {
        doc.push_str(&format!("\t<Item class=\"{class}\">\n"));
        for (name, value) in props {
            doc.push_str(&format!(
                "\t\t<Property name=\"{}\">{}</Property>\n",
                xml_escape(name),
                xml_escape(value)
            ));
        }
        doc.push_str("\t</Item>\n");
    }
    doc.push_str("</roblox>\n");
    doc
}

/// Stream `existing` through, replacing updated values in place and
/// appending missing keys before `</roblox>`.
fn merge_document(
    existing: &str,
    updates: &BTreeMap<(String, String), String>,
) -> Result<String> {
    let mut reader = Reader::from_str(existing);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();

    let mut current_class: Option<String> = None;
    let mut current_name: Option<String> = None;
    // (class, name) pairs already replaced in the stream.
    let mut done: BTreeMap<(String, String), bool> = BTreeMap::new();


    loop {
        let event = reader.read_event_into(&mut buf).map_err(|e| {
            Error::GameSettings(format!("could not parse settings XML: {e}"))
        })?;
        match event {
            Event::Start(tag) => {
                if tag.name() == QName(b"Item") {
                    current_class = attr_value(&tag, b"class")?;
                    current_name = None;
                } else if tag.name() == QName(b"Property") {
                    current_name = attr_value(&tag, b"name")?;
                }
                writer.write_event(Event::Start(tag)).map_err(write_err)?;
            }
            Event::Empty(tag) => {
                writer.write_event(Event::Empty(tag)).map_err(write_err)?;
            }
            Event::Text(text) => {
                let key = match (current_class.clone(), current_name.clone()) {
                    (Some(class), Some(name)) => Some((class, name)),
                    _ => None,
                };
                let replacement = match key {
                    Some((class, name)) => match updates.get(&(class.clone(), name.clone())) {
                        Some(value) => {
                            done.insert((class, name), true);
                            Event::Text(BytesText::from_escaped(xml_escape(value)))
                        }
                        None => Event::Text(text),
                    },
                    None => Event::Text(text),
                };
                writer.write_event(replacement).map_err(write_err)?;
                current_name = None;
            }
            Event::End(tag) => {
                // Append missing keys just before the closing root tag.
                if tag.name() == QName(b"roblox") {
                    let mut by_class: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
                    for ((class, name), value) in updates {
                        if done.contains_key(&(class.clone(), name.clone())) {
                            continue;
                        }
                        by_class
                            .entry(class.as_str())
                            .or_default()
                            .push((name.as_str(), value.as_str()));
                    }
                    for (class, props) in &by_class {
                        let xml = format!(
                            "\t<Item class=\"{}\">{}</Item>\n",
                            xml_escape(class),
                            props
                                .iter()
                                .map(|(n, v)| format!(
                                    "<Property name=\"{}\">{}</Property>",
                                    xml_escape(n),
                                    xml_escape(v)
                                ))
                                .collect::<String>(),
                        );
                        writer
                            .write_event(Event::Text(BytesText::from_escaped(xml)))
                            .map_err(write_err)?;
                    }
                }
                if tag.name() == QName(b"Item") {
                    current_class = None;
                }
                current_name = None;
                writer.write_event(Event::End(tag)).map_err(write_err)?;
            }
            Event::Eof => break,
            other => {
                writer.write_event(other).map_err(write_err)?;
            }
        }
        buf.clear();
    }

    let bytes = writer.into_inner();
    String::from_utf8(bytes).map_err(|e| Error::GameSettings(format!("settings XML is not UTF-8: {e}")))
}

fn write_err(e: quick_xml::errors::Error) -> Error {
    Error::GameSettings(format!("settings XML error: {e}"))
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<roblox version="4">
	<Item class="RenderSettings">
		<Property name="MSAALevel">4</Property>
		<Property name="VSyncDisabled">False</Property>
	</Item>
	<Item class="Other">
		<Property name="Kept">yes</Property>
	</Item>
</roblox>
"#;

    #[test]
    fn reads_all_properties() {
        let props = read_properties(SAMPLE).expect("parse");
        assert_eq!(props.len(), 3);
        assert_eq!(
            props[0],
            Property {
                class: String::from("RenderSettings"),
                name: String::from("MSAALevel"),
                value: String::from("4"),
            }
        );
    }

    #[test]
    fn merge_replaces_and_appends() {
        let mut updates = BTreeMap::new();
        updates.insert(
            (String::from("RenderSettings"), String::from("MSAALevel")),
            String::from("0"),
        );
        updates.insert(
            (String::from("RenderSettings"), String::from("FramerateCap")),
            String::from("240"),
        );
        let merged = merge_document(SAMPLE, &updates).expect("merge");
        let props = read_properties(&merged).expect("reparse");
        let get = |name: &str| {
            props
                .iter()
                .find(|p| p.class == "RenderSettings" && p.name == name)
                .map(|p| p.value.as_str())
        };
        assert_eq!(get("MSAALevel"), Some("0"));
        assert_eq!(get("VSyncDisabled"), Some("False"));
        assert_eq!(get("FramerateCap"), Some("240"));
        assert!(props.iter().any(|p| p.name == "Kept"));
    }

    #[test]
    fn build_creates_valid_document() {
        let mut updates = BTreeMap::new();
        updates.insert(
            (String::from("RenderSettings"), String::from("MSAALevel")),
            String::from("2"),
        );
        let doc = build_document(&updates);
        let props = read_properties(&doc).expect("parse");
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].value, "2");
    }
}
