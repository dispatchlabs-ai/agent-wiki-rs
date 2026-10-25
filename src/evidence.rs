use crate::digest::{Sha256, hex, random};
use crate::json::{Json, object, parse};
use crate::store::{Error, Result, atomic, hash_id, invalid, load_json, sync_dir, text};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub fn import(root: &Path, path: &Path, title: &str, provenance: &Json) -> Result<Json> {
    if provenance.has("parent") {
        let parent = snapshot(text(provenance, "parent", 100)?)?;
        load_json(&root.join("traces").join(format!("{parent}.json")))?;
    }
    if provenance.has("session") {
        text(provenance, "session", 300)?;
    }
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
        scan(root, &key, |_, _| Ok(()))?;
        if meta_path.exists() {
            let meta = load_json(&meta_path)?;
            for field in ["parent", "session"] {
                if provenance.has(field) && meta.get(field) != provenance.get(field) {
                    return Err(invalid(
                        "This immutable snapshot already has different provenance",
                    ));
                }
            }
            return Ok(meta);
        }
        let mut meta = object([
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
        for field in ["parent", "session"] {
            if provenance.has(field) {
                meta.insert(field, provenance.get(field).clone());
            }
        }
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
            let meta = load_json(&path)?;
            let hash = snapshot(meta.get("id").s())?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(hash)
                || meta.get("snapshot").s() != hash
            {
                return Err(invalid("Trace metadata identity mismatch"));
            }
            out.push(meta);
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
pub fn validate_quotes(value: &Json) -> Result<()> {
    if !matches!(value, Json::Array(_)) || value.array().len() > 20 {
        return Err(invalid("evidence must contain at most twenty quotations"));
    }
    for quote in value.array() {
        snapshot(text(quote, "conversation", 100)?)?;
        let line = text(quote, "event", 100)?
            .strip_prefix("line-")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0);
        if line.is_none() || text(quote, "quote", 10000)?.trim().is_empty() {
            return Err(invalid(
                "Evidence requires an original line-N event and a nonempty exact quote",
            ));
        }
    }
    Ok(())
}
fn recorded_text(record: &Json) -> String {
    if let Some((_, text)) = dialogue(record) {
        return text;
    }
    if let Some(text) = record.string() {
        return text.into();
    }
    if let Json::Array(values) = record {
        return values
            .iter()
            .map(recorded_text)
            .collect::<Vec<_>>()
            .join("\n");
    }
    for key in ["message", "payload", "content", "text", "output", "result"] {
        if record.has(key) {
            let text = recorded_text(record.get(key));
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}
pub fn verify_quotes(root: &Path, quotes: &Json) -> Result<Json> {
    validate_quotes(quotes)?;
    let mut verified = Vec::new();
    for quote in quotes.array() {
        let key = quote.get("conversation").s();
        let line = quote
            .get("event")
            .s()
            .strip_prefix("line-")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let mut found = None;
        scan(root, key, |number, raw| {
            if number == line {
                let record = parse(raw)?;
                if recorded_text(&record).contains(quote.get("quote").s()) {
                    found = Some(record);
                }
            }
            Ok(())
        })?;
        let record = found.ok_or_else(|| {
            Error::new(
                422,
                "EVIDENCE_MISMATCH",
                "The exact quotation was not found in the specified original record",
            )
        })?;
        let kind = if dialogue(&record).is_some() {
            "dialogue"
        } else {
            "raw"
        };
        verified.push(object([
            ("conversation", key.into()),
            ("event", quote.get("event").clone()),
            ("quote", quote.get("quote").clone()),
            ("snapshot", snapshot(key)?.into()),
            ("line", line.into()),
            (
                "attribution",
                dialogue(&record)
                    .map(|(role, _)| role)
                    .unwrap_or_else(|| "recorded activity".into())
                    .into(),
            ),
            (
                "url",
                format!(
                    "/conversations/{key}/?offset={}&kind={kind}#line-{line}",
                    line - 1
                )
                .into(),
            ),
        ]));
    }
    Ok(verified.into())
}
fn scan(root: &Path, key: &str, mut visit: impl FnMut(usize, &str) -> Result<()>) -> Result<usize> {
    let hash = snapshot(key)?;
    let path = root.join("traces").join(format!("{hash}.jsonl"));
    if !path.exists() {
        return Err(Error::new(404, "NOT_FOUND", "Trace not found"));
    }
    let mut reader = BufReader::new(File::open(path)?);
    let mut hash_state = Sha256::new();
    let mut bytes = Vec::new();
    let mut number = 0;
    loop {
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        hash_state.update(&bytes);
        number += 1;
        let line = std::str::from_utf8(&bytes).map_err(|_| invalid("Trace is not UTF-8"))?;
        visit(number, line)?;
    }
    if hex(&hash_state.finish()) != hash {
        return Err(Error::new(
            503,
            "INTEGRITY_ERROR",
            "Trace checksum mismatch; restore the original evidence before serving it",
        ));
    }
    Ok(number)
}
pub fn verify_all(root: &Path, conversations: bool) -> Result<Json> {
    let mut traces = 0usize;
    if conversations {
        for trace in catalog(root)? {
            let key = trace.get("id").s();
            if trace.get("snapshot").s() != snapshot(key)? {
                return Err(invalid("Trace metadata does not match its identity"));
            }
            let lines = scan(root, key, |_, line| {
                if !line.trim().is_empty() {
                    parse(line)?;
                }
                Ok(())
            })?;
            if trace.get("lines").number() != Some(lines) {
                return Err(invalid("Trace line count does not match original evidence"));
            }
            for file in attachments(root, key)? {
                attachment(root, key, file.get("sha256").s())?;
            }
            traces += 1;
        }
    }
    let mut assets = 0usize;
    for entry in fs::read_dir(root.join("media"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            let meta = load_json(&path)?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(meta.get("sha256").s()) {
                return Err(invalid("Media metadata identity mismatch"));
            }
            media(root, meta.get("file").s())?;
            assets += 1;
        }
    }
    Ok(object([
        ("traces", traces.into()),
        ("media", assets.into()),
    ]))
}
pub fn read(
    root: &Path,
    key: &str,
    offset: usize,
    limit: usize,
    raw: bool,
    window: (usize, usize),
) -> Result<Json> {
    let hash = snapshot(key)?;
    let (text_offset, text_limit) = window;
    if text_limit == 0 || text_limit > 64000 {
        return Err(invalid("text_limit must be 1–64000 Unicode characters"));
    }
    let mut messages = Vec::new();
    let mut next = Json::Null;
    let mut next_text = Json::Null;
    let mut stopped = false;
    let total = scan(root, key, |number, line| {
        if stopped || number <= offset || line.trim().is_empty() {
            return Ok(());
        }
        let record = parse(line)?;
        let selected = if raw {
            Some((
                "record".to_string(),
                line.trim_end_matches(['\r', '\n']).to_string(),
            ))
        } else {
            dialogue(&record)
        };
        if let Some((role, text)) = selected {
            if messages.len() == limit {
                next = (number - 1).into();
                stopped = true;
                return Ok(());
            }
            let start = if messages.is_empty() { text_offset } else { 0 };
            let length = text.chars().count();
            if start > length {
                return Err(invalid("text_offset is beyond this record"));
            }
            let selected = text
                .chars()
                .skip(start)
                .take(text_limit)
                .collect::<String>();
            let end = start + selected.chars().count();
            let partial = start != 0 || end < length;
            let kind = if raw { "raw" } else { "dialogue" };
            let mut message = object([
                ("role", role.into()), ("text", selected.into()),
                ("event", format!("line-{number}").into()), ("line", number.into()),
                ("textOffset", start.into()), ("textLength", length.into()),
                ("partial", partial.into()),
                ("url", format!("/conversations/{key}/?offset={}&kind={kind}&text_offset={start}#line-{number}", number-1).into()),
            ]);
            if raw && !partial {
                message.insert("record", record);
            }
            if end < length {
                next = (number - 1).into();
                next_text = end.into();
                stopped = true;
            }
            messages.push(message);
        }
        Ok(())
    })?;
    let mut result = object([
        ("id", key.into()),
        ("snapshot", hash.into()),
        ("messages", messages.into()),
        ("nextOffset", next),
        ("nextTextOffset", next_text),
        ("scannedThrough", total.into()),
        ("kind", if raw { "raw" } else { "dialogue" }.into()),
    ]);
    let meta = load_json(&root.join("traces").join(format!("{hash}.json")))?;
    result.insert("provenance", meta);
    result.insert("attachments", attachments(root, key)?.into());
    Ok(result)
}
pub fn attach(root: &Path, key: &str, event: usize, source: &Path) -> Result<Json> {
    if event == 0 || event > scan(root, key, |_, _| Ok(()))? {
        return Err(invalid(
            "Attachment event must be an existing original physical line",
        ));
    }
    let bytes = fs::read(source)?;
    let hash = crate::digest::sha256(&bytes);
    let directory = root.join("attachments").join(snapshot(key)?);
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{hash}.blob"));
    if path.exists() {
        if crate::digest::sha256(&fs::read(&path)?) != hash {
            return Err(Error::new(
                503,
                "INTEGRITY_ERROR",
                "Captured attachment checksum mismatch",
            ));
        }
    } else {
        atomic(&path, &bytes)?;
    }
    let meta_path = directory.join(format!("{hash}.json"));
    if meta_path.exists() {
        let existing = load_json(&meta_path)?;
        if existing.get("line").number() != Some(event) {
            return Err(invalid(
                "This captured file is already associated with another source event",
            ));
        }
        return Ok(existing);
    }
    let meta = object([
        ("sha256", hash.clone().into()),
        ("conversation", key.into()),
        ("line", event.into()),
        ("size", bytes.len().into()),
        (
            "name",
            source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
        ),
        ("url", format!("/conversation-files/{key}/{hash}").into()),
    ]);
    atomic(&meta_path, meta.encode().as_bytes())?;
    Ok(meta)
}
pub fn attachments(root: &Path, key: &str) -> Result<Vec<Json>> {
    let directory = root.join("attachments").join(snapshot(key)?);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            let meta = load_json(&path)?;
            if !hash_id(meta.get("sha256").s())
                || meta.get("conversation").s() != key
                || path.file_stem().and_then(|s| s.to_str()) != Some(meta.get("sha256").s())
            {
                return Err(invalid("Invalid captured file metadata"));
            }
            files.push(meta);
        }
    }
    files.sort_by_key(|f| f.get("name").s().to_string());
    Ok(files)
}
pub fn attachment(root: &Path, key: &str, hash: &str) -> Result<(Json, Vec<u8>)> {
    let snapshot = snapshot(key)?;
    if !hash_id(hash) {
        return Err(invalid("Invalid attachment identity"));
    }
    let directory = root.join("attachments").join(snapshot);
    let meta = load_json(&directory.join(format!("{hash}.json")))?;
    if meta.get("conversation").s() != key || meta.get("sha256").s() != hash {
        return Err(invalid("Captured file provenance mismatch"));
    }
    let bytes = fs::read(directory.join(format!("{hash}.blob")))?;
    if crate::digest::sha256(&bytes) != hash || meta.get("size").number() != Some(bytes.len()) {
        return Err(Error::new(
            503,
            "INTEGRITY_ERROR",
            "Captured attachment checksum mismatch",
        ));
    }
    Ok((meta, bytes))
}
pub fn attachment_window(root: &Path, input: &Json) -> Result<Json> {
    let (mut meta, bytes) = attachment(root, text(input, "id", 100)?, text(input, "sha256", 64)?)?;
    let offset = input.get("offset").number().unwrap_or(0);
    let limit = input.get("limit").number().unwrap_or(16384);
    if offset > bytes.len() || limit == 0 || limit > 65536 {
        return Err(invalid("Invalid attachment byte window (limit 1–65536)"));
    }
    let end = offset.saturating_add(limit).min(bytes.len());
    let chunk = &bytes[offset..end];
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for group in chunk.chunks(3) {
        let n = ((group[0] as u32) << 16)
            | ((group.get(1).copied().unwrap_or(0) as u32) << 8)
            | group.get(2).copied().unwrap_or(0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            encoded.push(if i > group.len() {
                '='
            } else {
                ALPHABET[((n >> shift) & 63) as usize] as char
            });
        }
    }
    meta.insert("base64", encoded.into());
    meta.insert("offset", offset.into());
    meta.insert(
        "nextOffset",
        if end < bytes.len() {
            end.into()
        } else {
            Json::Null
        },
    );
    Ok(meta)
}
pub fn search(root: &Path, q: &str, offset: usize, limit: usize) -> Result<Json> {
    let needle = q.to_lowercase();
    let mut matched = Vec::new();
    for mut trace in catalog(root)? {
        let mut found = if trace.get("title").s().to_lowercase().contains(&needle) {
            Some((0, String::new()))
        } else {
            None
        };
        scan(root, trace.get("id").s(), |number, line| {
            if found.is_none()
                && !line.trim().is_empty()
                && let Some((_, text)) = dialogue(&parse(line)?)
                && text.to_lowercase().contains(&needle)
            {
                found = Some((number, text.chars().take(300).collect::<String>()));
            }
            Ok(())
        })?;
        if let Some((line, snippet)) = found {
            trace.insert("snippet", snippet.into());
            trace.insert("line", line.into());
            trace.insert(
                "url",
                format!(
                    "/conversations/{}/?offset={}&kind=dialogue#line-{line}",
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
