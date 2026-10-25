use crate::digest::{random, sha256};
use crate::json::{Json, object, parse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn json(&self) -> Json {
        object([(
            "error",
            object([
                ("code", self.code.into()),
                ("message", self.message.clone().into()),
            ]),
        )])
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(500, "IO_ERROR", e.to_string())
    }
}
impl From<String> for Error {
    fn from(e: String) -> Self {
        Self::new(400, "INVALID_INPUT", e)
    }
}
pub fn invalid(s: impl Into<String>) -> Error {
    Error::new(400, "INVALID_INPUT", s)
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
pub fn hash_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn text<'a>(v: &'a Json, k: &str, max: usize) -> Result<&'a str> {
    let s = v
        .get(k)
        .string()
        .ok_or_else(|| invalid(format!("{k} must be a string")))?;
    if s.len() > max || s.contains('\0') {
        return Err(invalid(format!("Invalid or oversized {k}")));
    }
    Ok(s)
}
pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
pub fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Missing parent directory"))?;
    let temp = parent.join(format!(".pending-{}", random()?));
    let result = (|| {
        private_file(&temp, bytes)?;
        fs::rename(&temp, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn load_json(path: &Path) -> Result<Json> {
    Ok(parse(&fs::read_to_string(path)?)?)
}

pub fn validate_update(u: &Json) -> Result<()> {
    if !id(text(u, "id", 100)?) {
        return Err(invalid("Invalid article ID"));
    }
    text(u, "summary", 1000)?;
    if u.get("deleted").boolean() {
        return Ok(());
    }
    if text(u, "title", 200)?.trim().is_empty() || text(u, "body", 100000)?.trim().is_empty() {
        return Err(invalid("Title and body must not be empty"));
    }
    text(u, "description", 600)?;
    text(u, "topic", 100)?;
    for field in ["related", "questions", "aliases"] {
        if u.has(field)
            && (!matches!(u.get(field), Json::Array(_))
                || u.get(field)
                    .array()
                    .iter()
                    .any(|v| v.string().is_none() || (field == "related" && !id(v.s()))))
        {
            return Err(invalid(format!("Invalid {field}")));
        }
    }
    if u.has("sources")
        && (!matches!(u.get("sources"), Json::Array(_))
            || u.get("sources")
                .array()
                .iter()
                .any(|v| v.get("url").string().is_none()))
    {
        return Err(invalid(
            "sources must be objects with a URL; export complex YAML as JSON frontmatter",
        ));
    }
    if u.has("evidence") {
        crate::evidence::validate_quotes(u.get("evidence"))?;
    }
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
    pub head: String,
    pub pages: Arc<BTreeMap<String, Json>>,
    pub history: Arc<BTreeMap<String, Vec<Json>>>,
    pub receipts: Arc<BTreeMap<String, Json>>,
    pub fenced: bool,
    _lock: Arc<File>,
}
impl Store {
    pub fn init(path: &Path) -> Result<()> {
        fs::create_dir(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        for dir in ["commits", "traces", "media"] {
            fs::create_dir(path.join(dir))?;
        }
        private_file(&path.join("format"), b"agent-wiki-rs:1\n")?;
        private_file(&path.join("HEAD"), b"")?;
        private_file(
            &path.join("auth.json"),
            object([
                ("credentials", Vec::new().into()),
                ("sessions", Vec::new().into()),
            ])
            .encode()
            .as_bytes(),
        )?;
        sync_dir(path)
    }
    pub fn open(path: &Path) -> Result<Self> {
        if fs::read_to_string(path.join("format"))? != "agent-wiki-rs:1\n" {
            return Err(invalid(
                "Not an agent-wiki-rs store; never point this at an existing wiki installation",
            ));
        }
        let root = path.canonicalize()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(root.join("lock"))?;
        lock.try_lock()
            .map_err(|e| Error::new(409, "STORE_BUSY", format!("Store is in use: {e}")))?;
        let head = fs::read_to_string(root.join("HEAD"))?.trim().to_string();
        let mut s = Self {
            root,
            head: head.clone(),
            pages: Arc::new(BTreeMap::new()),
            history: Arc::new(BTreeMap::new()),
            receipts: Arc::new(BTreeMap::new()),
            fenced: false,
            _lock: Arc::new(lock),
        };
        let mut current = head;
        let mut commits = Vec::new();
        let mut seen = BTreeSet::new();
        while !current.is_empty() {
            if !hash_id(&current) || !seen.insert(current.clone()) {
                return Err(invalid("Corrupt revision chain"));
            }
            let bytes = fs::read(s.root.join("commits").join(format!("{current}.json")))?;
            if sha256(&bytes) != current {
                return Err(invalid("Revision integrity mismatch"));
            }
            let mut commit =
                parse(std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?)?;
            commit.insert("commit", current.clone().into());
            current = commit.get("parent").s().to_string();
            commits.push(commit);
        }
        for c in commits.into_iter().rev() {
            if !id(c.get("operation_id").s())
                || !hash_id(c.get("fingerprint").s())
                || c.get("actor").string().is_none()
                || c.get("author").string().is_none()
                || c.get("created_at").number().is_none()
                || c.get("updates").array().is_empty()
                || c.get("updates").array().len() > 10
            {
                return Err(invalid("Corrupt journal record"));
            }
            let mut ids = BTreeSet::new();
            for update in c.get("updates").array() {
                validate_update(update)?;
                let key = update.get("id").s();
                if !ids.insert(key) || !update.has("expected_revision_id") {
                    return Err(invalid("Corrupt journal update"));
                }
                let expected = s
                    .history
                    .get(key)
                    .and_then(|h| h.last())
                    .map(|p| p.get("revision_id"))
                    .unwrap_or(&Json::Null);
                if update.get("expected_revision_id") != expected {
                    return Err(invalid("Journal revision chain does not match its updates"));
                }
            }
            s.apply(&c);
        }
        Ok(s)
    }
    fn apply(&mut self, c: &Json) {
        let commit = c.get("commit").s();
        for update in c.get("updates").array() {
            let mut page = update.clone();
            let key = page.get("id").s().to_string();
            let history = Arc::make_mut(&mut self.history)
                .entry(key.clone())
                .or_default();
            page.remove("expected_revision_id");
            page.insert("number", (history.len() + 1).into());
            page.insert("revision_id", commit.into());
            page.insert("commit", commit.into());
            page.insert("created_at", c.get("created_at").clone());
            page.insert("author", c.get("author").clone());
            history.push(page.clone());
            if page.get("deleted").boolean() {
                Arc::make_mut(&mut self.pages).remove(&key);
            } else {
                Arc::make_mut(&mut self.pages).insert(key, page);
            }
        }
        Arc::make_mut(&mut self.receipts).insert(
            format!("{}:{}", c.get("actor").s(), c.get("operation_id").s()),
            c.clone(),
        );
    }
    pub fn save(&mut self, actor: &str, author: &str, input: &Json) -> Result<Json> {
        if self.fenced {
            return Err(Error::new(
                503,
                "STORE_UNCERTAIN",
                "Restart to recover an uncertain journal write",
            ));
        }
        let operation = text(input, "operation_id", 100)?;
        if !id(operation) {
            return Err(invalid("Invalid operation_id"));
        }
        let fingerprint = sha256(input.encode().as_bytes());
        let key = format!("{actor}:{operation}");
        if let Some(c) = self.receipts.get(&key) {
            if c.get("fingerprint").s() != fingerprint {
                return Err(Error::new(
                    409,
                    "OPERATION_CONFLICT",
                    "Operation ID was already used with different input",
                ));
            }
            return Ok(Self::receipt(c));
        }
        let updates = input.get("updates").array();
        if updates.is_empty() || updates.len() > 10 {
            return Err(invalid("Provide one to ten updates"));
        }
        let mut seen = BTreeSet::new();
        let mut checked = Vec::new();
        for u in updates {
            let key = text(u, "id", 100)?;
            if !id(key) || !seen.insert(key) {
                return Err(invalid("Invalid or repeated article ID"));
            }
            if !u.has("expected_revision_id") {
                return Err(invalid(
                    "expected_revision_id is required; null creates an article",
                ));
            }
            let prior = self.history.get(key).and_then(|h| h.last());
            let expected = prior.map(|p| p.get("revision_id")).unwrap_or(&Json::Null);
            if u.get("expected_revision_id") != expected {
                return Err(Error::new(
                    409,
                    "REVISION_CONFLICT",
                    format!("Article {key} changed; read its current revision"),
                ));
            }
            validate_update(u)?;
            if u.get("deleted").boolean() && prior.is_none() {
                return Err(Error::new(404, "NOT_FOUND", "Article not found"));
            }
            let mut page = prior.cloned().unwrap_or_else(|| object([]));
            if let Json::Object(fields) = u {
                for (k, v) in fields {
                    page.insert(k, v.clone());
                }
            }
            if u.has("evidence") {
                page.insert(
                    "evidence",
                    crate::evidence::verify_quotes(&self.root, u.get("evidence"))?,
                );
            }
            page.insert("deleted", u.get("deleted").boolean().into());
            checked.push(page);
        }
        let mut c = object([
            ("parent", self.head.clone().into()),
            ("actor", actor.into()),
            ("author", author.into()),
            ("operation_id", operation.into()),
            ("fingerprint", fingerprint.into()),
            ("created_at", now().into()),
            ("updates", checked.into()),
        ]);
        let bytes = c.encode();
        let commit = sha256(bytes.as_bytes());
        let dest = self.root.join("commits").join(format!("{commit}.json"));
        if !dest.exists() {
            atomic(&dest, bytes.as_bytes())?;
        }
        self.fenced = true;
        atomic(&self.root.join("HEAD"), commit.as_bytes())?;
        c.insert("commit", commit.clone().into());
        self.apply(&c);
        self.head = commit;
        self.fenced = false;
        Ok(Self::receipt(&c))
    }
    fn receipt(c: &Json) -> Json {
        object([
            ("operation_id", c.get("operation_id").clone()),
            ("commit", c.get("commit").clone()),
            ("committed", true.into()),
            ("published", true.into()),
            ("pushed", false.into()),
            ("storage", "local-journal".into()),
            (
                "articles",
                c.get("updates")
                    .array()
                    .iter()
                    .map(|u| u.get("id").clone())
                    .collect::<Vec<_>>()
                    .into(),
            ),
        ])
    }
    pub fn page(&self, key: &str, number: Option<usize>) -> Result<Json> {
        if !id(key) {
            return Err(invalid("Invalid article ID"));
        }
        let p = match number {
            Some(n) => self
                .history
                .get(key)
                .and_then(|h| n.checked_sub(1).and_then(|i| h.get(i))),
            None => self.pages.get(key),
        }
        .ok_or_else(|| Error::new(404, "NOT_FOUND", "Article or revision not found"))?;
        if p.get("deleted").boolean() {
            return Err(Error::new(
                410,
                "RETIRED",
                "Article was retired in this revision",
            ));
        }
        let mut p = p.clone();
        p.insert(
            "url",
            format!("/wiki/{key}/revision/{}/", p.get("number").s_number()).into(),
        );
        Ok(p)
    }
    pub fn backup(&self, destination: &Path) -> Result<()> {
        copy_tree(&self.root, destination)
    }
    pub fn verify_files(&self) -> Result<()> {
        if fs::read_to_string(self.root.join("format"))? != "agent-wiki-rs:1\n" {
            return Err(Error::new(
                503,
                "INTEGRITY_ERROR",
                "Store format changed while serving",
            ));
        }
        let mut head = fs::read_to_string(self.root.join("HEAD"))?
            .trim()
            .to_string();
        let mut seen = BTreeSet::new();
        // A concurrent save may advance HEAD after this read snapshot was taken.
        // Accept a verified descendant, never a rollback or an unrelated chain.
        while head != self.head {
            if !hash_id(&head) || !seen.insert(head.clone()) {
                return Err(Error::new(
                    503,
                    "INTEGRITY_ERROR",
                    "Journal HEAD no longer extends the serving revision",
                ));
            }
            let bytes = fs::read(self.root.join("commits").join(format!("{head}.json")))?;
            if sha256(&bytes) != head {
                return Err(Error::new(
                    503,
                    "INTEGRITY_ERROR",
                    "Journal HEAD checksum mismatch",
                ));
            }
            let commit = parse(std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?)?;
            head = commit
                .get("parent")
                .string()
                .ok_or_else(|| invalid("Journal parent must be a revision string"))?
                .to_string();
        }
        for commit in self.receipts.values() {
            let key = commit.get("commit").s();
            let bytes = fs::read(self.root.join("commits").join(format!("{key}.json")))?;
            if sha256(&bytes) != key {
                return Err(Error::new(
                    503,
                    "INTEGRITY_ERROR",
                    "Journal checksum mismatch",
                ));
            }
        }
        Ok(())
    }
}
impl Json {
    pub fn s_number(&self) -> String {
        self.number().map(|n| n.to_string()).unwrap_or_default()
    }
}
pub fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().unwrap_or(Path::new(".")).canonicalize()?;
    if parent.starts_with(source.canonicalize()?) {
        return Err(invalid("Backup must be outside its source"));
    }
    fs::create_dir(dest)?;
    fs::set_permissions(dest, fs::Permissions::from_mode(0o700))?;
    let staging = parent.join(format!(".pending-copy-{}", random()?));
    fs::create_dir(&staging)?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;
    fn copy(source: &Path, dest: &Path) -> Result<()> {
        for e in fs::read_dir(source)? {
            let e = e?;
            let name = e.file_name();
            let kind = e.file_type()?;
            if name == "lock" || name.to_string_lossy().starts_with(".pending-") {
                continue;
            }
            let to = dest.join(name);
            if kind.is_symlink() {
                return Err(invalid("Symlinks are not permitted in backups"));
            }
            if kind.is_dir() {
                fs::create_dir(&to)?;
                copy(&e.path(), &to)?;
            } else if kind.is_file() {
                let mut src = File::open(e.path())?;
                let mut target = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(to)?;
                std::io::copy(&mut src, &mut target)?;
                target.sync_all()?;
            }
        }
        sync_dir(dest)
    }
    let result = (|| {
        copy(source, &staging)?;
        fs::rename(&staging, dest)?;
        sync_dir(&parent)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        // Only remove the empty reservation. Never remove a published backup.
        let _ = fs::remove_dir(dest);
    }
    result
}
