use crate::digest::{Sha256, hex, random};
use crate::json::{Json, object, parse};
use crate::store::{Error, Result, atomic, hash_id, invalid, load_json, sync_dir};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub fn import(root: &Path, path: &Path, title: &str) -> Result<Json> {
    let directory = root.join("traces");
    let temp = directory.join(format!(".pending-{}", random()?));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let mut reader = BufReader::new(File::open(path)?);
        let mut bytes = Vec::new();
        let mut digest = Sha256::new();
        let mut lines = 0usize;
        loop {
            bytes.clear();
            if reader.read_until(b'\n', &mut bytes)? == 0 {
                break;
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| invalid(format!("Line {} is not UTF-8: {e}", lines + 1)))?;
            if !text.trim().is_empty() {
                parse(text)
                    .map_err(|e| invalid(format!("Invalid JSONL at line {}: {e}", lines + 1)))?;
            }
            digest.update(&bytes);
            output.write_all(&bytes)?;
            lines += 1;
        }
        output.sync_all()?;
        let snapshot = hex(&digest.finish());
        let key = format!("chat-{snapshot}");
        let dest = directory.join(format!("{snapshot}.jsonl"));
        if dest.exists() {
            fs::remove_file(&temp)?;
        } else {
            fs::rename(&temp, &dest)?;
            sync_dir(&directory)?;
        }
        let meta_path = directory.join(format!("{snapshot}.json"));
        if meta_path.exists() {
            return load_json(&meta_path);
        }
        let meta = object([
            ("id", key.into()),
            ("snapshot", snapshot.into()),
            ("title", title.into()),
            ("lines", lines.into()),
            (
                "source_name",
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            ),
        ]);
        atomic(&meta_path, meta.encode().as_bytes())?;
        Ok(meta)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn snapshot(key: &str) -> Result<&str> {
    key.strip_prefix("chat-")
        .filter(|s| hash_id(s))
        .ok_or_else(|| invalid("Invalid trace ID"))
}
pub fn catalog(root: &Path) -> Result<Vec<Json>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root.join("traces"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            out.push(load_json(&path)?);
        }
    }
    out.sort_by_key(|v| v.get("title").s().to_lowercase());
    Ok(out)
}
fn dialogue(record: &Json) -> Option<(String, String)> {
    let message = if record.has("message") {
        record.get("message")
    } else if record.has("payload") {
        record.get("payload")
    } else {
        record
    };
    let kind = message.get("type").s();
    if ["user_message", "agent_message"].contains(&kind) {
        return Some((
            if kind == "user_message" {
                "user"
            } else {
                "assistant"
            }
            .into(),
            message.get("message").s().into(),
        ));
    }
    let role = message.get("role").s();
    if role != "user" && role != "assistant" {
        return None;
    }
    let content = message.get("content");
    let text = if let Some(s) = content.string() {
        s.to_string()
    } else {
        content
            .array()
            .iter()
            .filter(|c| ["text", "input_text", "output_text"].contains(&c.get("type").s()))
            .map(|c| c.get("text").s())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if text.is_empty() {
        None
    } else {
        Some((role.into(), text))
    }
}
pub fn read(root: &Path, key: &str, offset: usize, limit: usize, raw: bool) -> Result<Json> {
    let hash = snapshot(key)?;
    let path = root.join("traces").join(format!("{hash}.jsonl"));
    if !path.exists() {
        return Err(Error::new(404, "NOT_FOUND", "Trace not found"));
    }
    let reader = BufReader::new(File::open(path)?);
    let mut messages = Vec::new();
    let mut next = Json::Null;
    let mut total = 0;
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let number = i + 1;
        total = number;
        if number <= offset || line.trim().is_empty() {
            continue;
        }
        let record = parse(&line)?;
        let message = if raw {
            Some(object([("record", record)]))
        } else {
            dialogue(&record)
                .map(|(role, text)| object([("role", role.into()), ("text", text.into())]))
        };
        if let Some(mut message) = message {
            if messages.len() >= limit {
                next = (number - 1).into();
                break;
            }
            message.insert("event", format!("line-{number}").into());
            message.insert("line", number.into());
            message.insert(
                "url",
                format!("/conversations/{key}/?offset={}#line-{number}", number - 1).into(),
            );
            messages.push(message);
        }
    }
    Ok(object([
        ("id", key.into()),
        ("snapshot", hash.into()),
        ("messages", messages.into()),
        ("nextOffset", next),
        ("scannedThrough", total.into()),
        ("kind", if raw { "raw" } else { "dialogue" }.into()),
    ]))
}
pub fn search(root: &Path, q: &str, offset: usize, limit: usize) -> Result<Json> {
    let needle = q.to_lowercase();
    let mut matched = Vec::new();
    for mut trace in catalog(root)? {
        let hash = snapshot(trace.get("id").s())?;
        let reader = BufReader::new(File::open(
            root.join("traces").join(format!("{hash}.jsonl")),
        )?);
        let mut found = if trace.get("title").s().to_lowercase().contains(&needle) {
            Some((0, String::new()))
        } else {
            None
        };
        if found.is_none() {
            for (i, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Some((_, text)) = dialogue(&parse(&line)?)
                    && text.to_lowercase().contains(&needle)
                {
                    found = Some((i + 1, text.chars().take(300).collect::<String>()));
                    break;
                }
            }
        }
        if let Some((line, snippet)) = found {
            trace.insert("snippet", snippet.into());
            trace.insert("line", line.into());
            trace.insert(
                "url",
                format!(
                    "/conversations/{}/?offset={}",
                    trace.get("id").s(),
                    line.saturating_sub(1)
                )
                .into(),
            );
            matched.push(trace);
        }
    }
    let total = matched.len();
    Ok(object([
        ("total", total.into()),
        (
            "results",
            matched
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>()
                .into(),
        ),
        (
            "nextOffset",
            if offset + limit < total {
                (offset + limit).into()
            } else {
                Json::Null
            },
        ),
    ]))
}
pub fn publish_media(root: &Path, path: &Path, source: &str) -> Result<Json> {
    let mut file = File::open(path)?;
    let mut head = [0u8; 16];
    let count = file.read(&mut head)?;
    let ext = if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if head.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        "gif"
    } else if count >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        "webp"
    } else if head.starts_with(b"%PDF-") {
        "pdf"
    } else {
        return Err(invalid(
            "Only PNG, JPEG, GIF, WebP and PDF assets are accepted",
        ));
    };
    let dir = root.join("media");
    let temp = dir.join(format!(".pending-{}", random()?));
    let result = (|| {
        let mut input = File::open(path)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        output.sync_all()?;
        let hash = hex(&hash.finish());
        let name = format!("{hash}.{ext}");
        let dest = dir.join(&name);
        if dest.exists() {
            fs::remove_file(&temp)?;
        } else {
            fs::rename(&temp, &dest)?;
            sync_dir(&dir)?;
        }
        let meta = dir.join(format!("{hash}.json"));
        if !meta.exists() {
            atomic(
                &meta,
                object([
                    ("source", source.into()),
                    ("sha256", hash.clone().into()),
                    ("file", name.clone().into()),
                ])
                .encode()
                .as_bytes(),
            )?;
        }
        Ok(object([
            ("sha256", hash.into()),
            ("url", format!("/article-media/{name}").into()),
        ]))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn media(root: &Path, name: &str) -> Result<(Vec<u8>, &'static str)> {
    let (hash, ext) = name
        .split_once('.')
        .ok_or_else(|| invalid("Invalid media URL"))?;
    if !hash_id(hash) {
        return Err(invalid("Invalid media hash"));
    }
    let mime = match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => return Err(invalid("Unsupported media format")),
    };
    let path = root.join("media").join(name);
    if !path.exists() {
        return Err(Error::new(404, "NOT_FOUND", "Media not found"));
    }
    let meta = load_json(&root.join("media").join(format!("{hash}.json")))?;
    if meta.get("file").s() != name {
        return Err(invalid("Incomplete media publication"));
    }
    let bytes = fs::read(path)?;
    if crate::digest::sha256(&bytes) != hash {
        return Err(Error::new(
            500,
            "INTEGRITY_ERROR",
            "Media checksum mismatch",
        ));
    }
    Ok((bytes, mime))
}
