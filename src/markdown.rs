use crate::json::{Json, object, parse};
use crate::store::{Error, Result, invalid};
use std::collections::{BTreeMap, BTreeSet};

pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        out.push_str(match c {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                out.push(c);
                continue;
            }
        });
    }
    out
}
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}
fn safe_url(s: &str, image: bool) -> bool {
    if s.chars().any(|c| c.is_control() || c == '\\') || s.starts_with("//") {
        return false;
    }
    s.starts_with('/')
        || s.starts_with('#')
        || s.starts_with("https://")
        || (!image && (s.starts_with("http://") || s.starts_with("mailto:")))
}
pub fn inline(s: &str) -> String {
    inline_depth(s, 0)
}
fn inline_depth(s: &str, depth: usize) -> String {
    if depth > 24 {
        return escape(s);
    }
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let tail = &s[i..];
        if let Some(rest) = tail.strip_prefix('`')
            && let Some(end) = rest.find('`')
        {
            out.push_str(&format!("<code>{}</code>", escape(&rest[..end])));
            i += end + 2;
            continue;
        }
        if let Some(rest) = tail.strip_prefix("[[")
            && let Some(end) = rest.find("]]")
        {
            let content = rest[..end].replace("\\|", "|");
            let (target, label) = content.split_once('|').unwrap_or((&content, &content));
            let (page, anchor) = target.split_once('#').unwrap_or((target, ""));
            if crate::store::id(page) && !anchor.contains(['"', '<', '>', ' ']) {
                out.push_str(&format!(
                    "<a href=\"/wiki/{}/{}\">{}</a>",
                    escape(page),
                    if anchor.is_empty() {
                        String::new()
                    } else {
                        format!("#{}", escape(anchor))
                    },
                    escape(label)
                ));
                i += end + 4;
                continue;
            }
        }
        let image = tail.starts_with("![");
        let start = if image { 2 } else { 1 };
        if (image || tail.starts_with('['))
            && tail.len() > start
            && let Some(end) = tail[start..].find("](")
        {
            let label = &tail[start..start + end];
            let url_start = start + end + 2;
            if let Some(close) = tail[url_start..].find(')') {
                let url = &tail[url_start..url_start + close];
                if safe_url(url, image) {
                    if image {
                        out.push_str(&format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\"><img src=\"{}\" alt=\"{}\" loading=\"lazy\"></a>",escape(url),escape(url),escape(label)));
                    } else {
                        out.push_str(&format!(
                            "<a href=\"{}\" rel=\"noopener noreferrer\">{}</a>",
                            escape(url),
                            escape(label)
                        ));
                    }
                    i += url_start + close + 1;
                    continue;
                }
            }
        }
        let mut matched = false;
        for (marker, tag) in [
            ("**", "strong"),
            ("__", "strong"),
            ("~~", "del"),
            ("*", "em"),
            ("_", "em"),
        ] {
            if let Some(rest) = tail.strip_prefix(marker)
                && let Some(end) = rest.find(marker)
                && end > 0
            {
                out.push_str(&format!(
                    "<{tag}>{}</{tag}>",
                    inline_depth(&rest[..end], depth + 1)
                ));
                i += end + marker.len() * 2;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        let c = tail.chars().next().unwrap_or_default();
        if c == '\\'
            && let Some(c) = tail[1..].chars().next()
        {
            out.push_str(&escape(&c.to_string()));
            i += 1 + c.len_utf8();
            continue;
        }
        out.push_str(&escape(&c.to_string()));
        i += c.len_utf8();
    }
    out
}
#[derive(Clone)]
pub struct Section {
    pub anchor: String,
    pub heading: String,
    pub depth: usize,
    pub start: usize,
    pub end: usize,
}
fn heading(line: &str) -> Option<(usize, &str)> {
    let n = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&n) && line.as_bytes().get(n) == Some(&b' ') {
        Some((n, line[n + 1..].trim().trim_end_matches('#').trim()))
    } else {
        None
    }
}
pub fn sections(body: &str) -> Vec<Section> {
    let mut out = vec![Section {
        anchor: String::new(),
        heading: "Overview".into(),
        depth: 0,
        start: 0,
        end: body.len(),
    }];
    let mut used = BTreeMap::new();
    let mut offset = 0;
    let mut fence: Option<char> = None;
    for line in body.split_inclusive('\n') {
        let trim = line.trim_end();
        if trim.starts_with("```") || trim.starts_with("~~~") {
            let c = trim.chars().next().unwrap_or('`');
            if fence == Some(c) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(c);
            }
        } else if fence.is_none()
            && let Some((depth, label)) = heading(trim)
        {
            let base = format!("section-{}", slug(label));
            let count = used.entry(base.clone()).or_insert(0);
            *count += 1;
            let anchor = if *count == 1 {
                base
            } else {
                format!("{base}-{count}")
            };
            out.push(Section {
                anchor,
                heading: label.into(),
                depth,
                start: offset,
                end: body.len(),
            });
        }
        offset += line.len();
    }
    for i in 1..out.len() {
        if let Some(j) = (i + 1..out.len()).find(|j| out[*j].depth <= out[i].depth) {
            out[i].end = out[j].start;
        }
    }
    if out.len() > 1 {
        out[0].end = out[1].start;
    }
    out
}
pub fn outline(body: &str) -> Json {
    sections(body)
        .into_iter()
        .map(|s| {
            object([
                ("anchor", s.anchor.into()),
                ("heading", s.heading.into()),
                ("depth", s.depth.into()),
            ])
        })
        .collect::<Vec<_>>()
        .into()
}
pub fn selected(body: &str, anchor: &str) -> Result<String> {
    let s = sections(body)
        .into_iter()
        .find(|s| s.anchor == anchor)
        .ok_or_else(|| Error::new(404, "SECTION_NOT_FOUND", "Section not found"))?;
    Ok(body[s.start..s.end].into())
}
pub fn references(body: &str) -> BTreeSet<String> {
    render(body)
        .split("href=\"/wiki/")
        .skip(1)
        .filter_map(|part| {
            let target = part.split(['/', '#', '?', '\"']).next().unwrap_or("");
            crate::store::id(target).then(|| target.to_string())
        })
        .collect()
}

fn cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    let bytes = line.as_bytes();
    let mut cells = Vec::new();
    let mut start = usize::from(bytes.first() == Some(&b'|'));
    let mut i = start;
    let mut brackets = 0usize;
    let mut code = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && code == 0 {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            let length = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if code == 0 {
                code = length;
            } else if code == length {
                code = 0;
            }
            i += length;
            continue;
        }
        if code == 0 {
            match bytes[i] {
                b'[' => brackets += 1,
                b']' => brackets = brackets.saturating_sub(1),
                b'|' if brackets == 0 => {
                    cells.push(line[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if start < line.len() {
        cells.push(line[start..].trim());
    }
    cells
}
fn rule(line: &str) -> bool {
    let c = cells(line);
    !c.is_empty()
        && c.iter().all(|s| {
            s.trim_matches(':').len() >= 3 && s.trim_matches(':').bytes().all(|b| b == b'-')
        })
}
pub fn render(body: &str) -> String {
    let lines = body.lines().collect::<Vec<_>>();
    let outline = sections(body);
    let mut heading_index = 1;
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if line.is_empty() {
            i += 1;
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            let fence = &line[..3];
            let language = escape(line[3..].trim());
            i += 1;
            let mut code = String::new();
            while i < lines.len() && !lines[i].starts_with(fence) {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            out.push_str(&format!(
                "<pre><code data-language=\"{language}\">{}</code></pre>",
                escape(&code)
            ));
            if i < lines.len() {
                i += 1;
            }
            continue;
        }
        if let Some((level, label)) = heading(line) {
            let anchor = outline
                .get(heading_index)
                .map(|s| s.anchor.clone())
                .unwrap_or_default();
            heading_index += 1;
            out.push_str(&format!(
                "<h{level} id=\"{anchor}\">{}</h{level}>",
                inline(label)
            ));
            i += 1;
            continue;
        }
        if ["---", "***", "___"].contains(&line.trim()) {
            out.push_str("<hr>");
            i += 1;
            continue;
        }
        if i + 1 < lines.len() && line.contains('|') && rule(lines[i + 1]) {
            out.push_str("<div class=\"table-wrap\"><table><thead><tr>");
            for c in cells(line) {
                out.push_str(&format!("<th>{}</th>", inline(c)));
            }
            out.push_str("</tr></thead><tbody>");
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                out.push_str("<tr>");
                for c in cells(lines[i]) {
                    out.push_str(&format!("<td>{}</td>", inline(c)));
                }
                out.push_str("</tr>");
                i += 1;
            }
            out.push_str("</tbody></table></div>");
            continue;
        }
        if line.starts_with("> ") {
            out.push_str("<blockquote>");
            while i < lines.len() && lines[i].starts_with("> ") {
                out.push_str(&format!("<p>{}</p>", inline(&lines[i][2..])));
                i += 1;
            }
            out.push_str("</blockquote>");
            continue;
        }
        if let Some((ordered, _)) = list_item(line) {
            let tag = if ordered { "ol" } else { "ul" };
            out.push_str(&format!("<{tag}>"));
            while i < lines.len() {
                let Some((kind, item)) = list_item(lines[i]) else {
                    break;
                };
                if kind != ordered {
                    break;
                }
                let item = if let Some(t) = item.strip_prefix("[ ] ") {
                    format!("<input type=\"checkbox\" disabled> {}", inline(t))
                } else if let Some(t) = item
                    .strip_prefix("[x] ")
                    .or_else(|| item.strip_prefix("[X] "))
                {
                    format!("<input type=\"checkbox\" disabled checked> {}", inline(t))
                } else if let Some(t) = item.strip_prefix("[w] ") {
                    format!("<span aria-label=\"In progress\">◐</span> {}", inline(t))
                } else {
                    inline(item)
                };
                out.push_str(&format!("<li>{item}</li>"));
                i += 1;
            }
            out.push_str(&format!("</{tag}>"));
            continue;
        }
        let mut paragraph = String::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            if !paragraph.is_empty()
                && (heading(lines[i]).is_some()
                    || list_item(lines[i]).is_some()
                    || lines[i].starts_with("```")
                    || lines[i].starts_with("~~~")
                    || lines[i].starts_with("> "))
            {
                break;
            }
            if !paragraph.is_empty() {
                paragraph.push('\n');
            }
            paragraph.push_str(lines[i]);
            i += 1;
        }
        out.push_str(&format!("<p>{}</p>", inline(&paragraph)));
    }
    out
}
fn list_item(s: &str) -> Option<(bool, &str)> {
    let s = s.trim_start();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some((false, rest));
        }
    }
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n > 0 && s[n..].starts_with(". ") {
        Some((true, &s[n + 2..]))
    } else {
        None
    }
}
pub fn from_markdown(id: &str, raw: &str) -> Result<Json> {
    let mut data = object([]);
    let body;
    if let Some(rest) = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
    {
        let end = rest
            .find("\n---\n")
            .or_else(|| rest.find("\n---\r\n"))
            .ok_or_else(|| invalid("Unclosed frontmatter"))?;
        let header = &rest[..end];
        let after = &rest[end + 4..];
        body = after
            .strip_prefix("\r\n")
            .or_else(|| after.strip_prefix('\n'))
            .unwrap_or(after)
            .to_string();
        if header.trim_start().starts_with('{') {
            data = parse(header)?;
        } else {
            let mut last = String::new();
            for line in header.lines() {
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    continue;
                }
                if let Some(value) = line.trim_start().strip_prefix("- ") {
                    if last.is_empty() || !matches!(data.get(&last), Json::Array(_)) {
                        return Err(invalid("Unrecognized frontmatter list"));
                    }
                    let mut values = data.get(&last).array().to_vec();
                    values.push(yaml_scalar(value)?);
                    data.insert(&last, values.into());
                    continue;
                }
                if line.starts_with(char::is_whitespace) {
                    return Err(invalid(
                        "Complex YAML must be exported as JSON frontmatter before import",
                    ));
                }
                let (key, value) = line
                    .split_once(':')
                    .ok_or_else(|| invalid("Invalid frontmatter"))?;
                last = key.trim().into();
                if last.is_empty() || data.has(&last) {
                    return Err(invalid("Empty or repeated frontmatter key"));
                }
                let value = value.trim();
                let value = if value.starts_with('[') || value.starts_with('{') {
                    parse(value)?
                } else if value.is_empty() {
                    Json::Array(Vec::new())
                } else if value == "|" || value == ">" || value.starts_with(['&', '*', '!']) {
                    return Err(invalid(
                        "Complex YAML must be exported as JSON frontmatter before import",
                    ));
                } else {
                    yaml_scalar(value)?
                };
                data.insert(&last, value);
            }
        }
        if !matches!(data, Json::Object(_)) {
            return Err(invalid("Frontmatter must be an object"));
        }
    } else {
        body = raw.into();
    }
    if !data.has("title") {
        data.insert(
            "title",
            body.lines()
                .find_map(|l| heading(l).map(|(_, s)| s.to_string()))
                .unwrap_or_else(|| id.replace('-', " "))
                .into(),
        );
    }
    for (key, default) in [("description", ""), ("topic", "Wiki")] {
        if !data.has(key) {
            data.insert(key, default.into());
        }
    }
    data.insert("id", id.into());
    data.insert("body", body.into());
    data.insert("expected_revision_id", Json::Null);
    data.insert(
        "summary",
        "Imported Markdown; prior Git history was not migrated".into(),
    );
    Ok(data)
}
fn yaml_scalar(s: &str) -> Result<Json> {
    if s.starts_with('"') {
        return Ok(parse(s)?);
    }
    if s.starts_with('\'') {
        if !s.ends_with('\'') || s.len() < 2 {
            return Err(invalid("Unclosed frontmatter quote"));
        }
        if s[1..s.len() - 1].replace("''", "").contains('\'') {
            return Err(invalid("Unescaped frontmatter quote"));
        }
        return Ok(s[1..s.len() - 1].replace("''", "'").into());
    }
    if s.starts_with(['{', '[']) {
        return Ok(parse(s)?);
    }
    if s.contains(": ")
        || s.ends_with(':')
        || s.contains(" #")
        || s.starts_with(['&', '*', '!', '|', '>'])
    {
        return Err(invalid(
            "Complex YAML must be exported as JSON frontmatter before import",
        ));
    }
    if let Ok(value) = parse(s) {
        return Ok(value);
    }
    Ok(s.into())
}
pub fn to_markdown(page: &Json) -> String {
    let mut meta = page.clone();
    for key in [
        "body",
        "id",
        "number",
        "revision_id",
        "commit",
        "created_at",
        "author",
        "expected_revision_id",
        "deleted",
        "url",
    ] {
        meta.remove(key);
    }
    format!("---\n{}\n---\n{}", meta.encode(), page.get("body").s())
}
