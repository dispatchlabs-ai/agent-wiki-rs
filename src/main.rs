#![forbid(unsafe_code)]

mod app;
mod auth;
mod digest;
mod evidence;
mod http;
mod json;
mod markdown;
mod store;
mod ui;

use app::App;
use auth::Auth;
use json::{Json, object, parse};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use store::{Error, Result, Store, invalid, private_file};

const HELP: &str = r#"agent-wiki 0.1.0-alpha.1
Created by Chris Reynolds, cofounder of Dispatch Labs AI.
Dependency-free Rust wiki. Light-only UI. No Git, Node or helper executables.

agent-wiki init --data DIR --token-file FILE
agent-wiki serve --data DIR [--listen 127.0.0.1:8080] [--origin ORIGIN]
                 [--read-only] [--no-conversations]
agent-wiki call TOOL [JSON | @FILE | -] --token-file FILE
                [--url http://127.0.0.1:8080]
agent-wiki mcp --token-file FILE [--url http://127.0.0.1:8080]

Filesystem-authorized operator commands (stop the service first):
agent-wiki grant --data DIR --name NAME --role reader|editor|manager
                 --token-file FILE [--scopes read,write,trace,manage]
agent-wiki revoke --data DIR --id CREDENTIAL_ID
agent-wiki retire --data DIR --id ARTICLE_ID --revision REVISION_ID
                  --summary REASON
agent-wiki import --data DIR --directory MARKDOWN_DIR
agent-wiki export --data DIR --directory NEW_DIR
agent-wiki import-trace --data DIR --file JSONL --title TITLE
agent-wiki publish-media --data DIR --file FILE --source PROVENANCE
agent-wiki backup --data DIR --to NEW_DIR
agent-wiki restore --from BACKUP --data NEW_DIR --token-file FILE
agent-wiki verify --data DIR

init refuses existing directories and token files. Keep credentials private.
serve binds only to loopback. No native TLS, OIDC, password or passkey login.
The browser accepts the issued credential; reader/editor/manager roles and
read/write/trace/manage scopes apply across browser, HTTP, MCP and CLI operations.
mcp bridges newline JSON-RPC over stdin/stdout to the running HTTP service.
call prints JSON. Input '-' reads stdin. Errors go to stderr with nonzero exit.
The authenticated /api/tools and /api/openapi.json expose live schemas.

Storage is an atomic local revision journal, not Git. import reads current
Markdown only; it does not migrate old revisions, users, receipts or citations.
Imports accept JSON frontmatter and a strict simple-YAML subset; unsupported
YAML is rejected. Complex diagrams and mathematics remain source text.
backup copies durable state while the service is stopped. restore verifies
the journal, revokes restored credentials/sessions and issues a new manager.
This is a source alpha, not a drop-in replacement for existing Agent Wiki.
"#;

struct Args {
    command: String,
    options: BTreeMap<String, String>,
    positional: Vec<String>,
}
impl Args {
    fn parse() -> Result<Self> {
        let mut iter = std::env::args().skip(1);
        let command = iter.next().unwrap_or_else(|| "--help".into());
        let mut options = BTreeMap::new();
        let mut positional = Vec::new();
        while let Some(arg) = iter.next() {
            if arg.starts_with("--") {
                let value =
                    if ["--help", "--read-only", "--no-conversations"].contains(&arg.as_str()) {
                        String::new()
                    } else {
                        iter.next()
                            .ok_or_else(|| invalid(format!("Missing value for {arg}")))?
                    };
                if options.insert(arg, value).is_some() {
                    return Err(invalid("Repeated option"));
                }
            } else {
                positional.push(arg);
            }
        }
        Ok(Self {
            command,
            options,
            positional,
        })
    }
    fn required(&self, key: &str) -> Result<&str> {
        self.options
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| invalid(format!("{key} is required")))
    }
    fn option<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.options.get(key).map(String::as_str).unwrap_or(default)
    }
    fn check(&self, allowed: &[&str], positionals: usize) -> Result<()> {
        if self.options.keys().any(|k| !allowed.contains(&k.as_str()))
            || self.positional.len() > positionals
        {
            return Err(invalid("Unknown option or unexpected argument; use --help"));
        }
        Ok(())
    }
}
fn output(v: Json) {
    println!("{}", v.encode());
}
fn token_path(args: &Args) -> Result<PathBuf> {
    let path = PathBuf::from(args.required("--token-file")?);
    if path.exists() {
        return Err(invalid(
            "Credential output file already exists; it will not be overwritten",
        ));
    }
    Ok(path)
}
fn issue(auth: &mut Auth, name: &str, role: &str, scopes: &[String], path: &Path) -> Result<Json> {
    let mut result = auth.grant(name, role, scopes)?;
    if let Err(e) = private_file(path, format!("{}\n", result.get("token").s()).as_bytes()) {
        auth.revoke(result.get("id").s())?;
        return Err(e);
    }
    result.remove("token");
    result.insert("token_file", path.to_string_lossy().into_owned().into());
    Ok(result)
}
fn token(args: &Args) -> Result<String> {
    let path = Path::new(args.required("--token-file")?);
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(invalid(
            "Credential file must be a regular private file (chmod 600)",
        ));
    }
    let secret = fs::read_to_string(path)?.trim().to_string();
    if !store::hash_id(&secret) {
        return Err(invalid("Invalid credential file"));
    }
    Ok(secret)
}
fn input(s: &str) -> Result<Json> {
    let s = if s == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if let Some(path) = s.strip_prefix('@') {
        fs::read_to_string(path)?
    } else {
        s.to_string()
    };
    Ok(parse(&s)?)
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.json().encode());
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse()?;
    if args.command == "--help" || args.command == "help" || args.options.contains_key("--help") {
        print!("{HELP}");
        return Ok(());
    }
    if args.command == "--version" {
        println!("agent-wiki {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match args.command.as_str() {
        "init" => {
            args.check(&["--data", "--token-file"], 0)?;
            let file = token_path(&args)?;
            let root = Path::new(args.required("--data")?);
            Store::init(root)?;
            let _store = Store::open(root)?;
            let mut auth = Auth::open(root)?;
            output(issue(
                &mut auth,
                "Operator",
                "manager",
                &[
                    "read".into(),
                    "write".into(),
                    "trace".into(),
                    "manage".into(),
                ],
                &file,
            )?);
        }
        "serve" => {
            args.check(
                &[
                    "--data",
                    "--listen",
                    "--origin",
                    "--read-only",
                    "--no-conversations",
                ],
                0,
            )?;
            let address = args
                .option("--listen", "127.0.0.1:8080")
                .parse()
                .map_err(|_| invalid("Invalid listen address"))?;
            let app = App::open(
                Path::new(args.required("--data")?),
                !args.options.contains_key("--no-conversations"),
                !args.options.contains_key("--read-only"),
            )?;
            http::serve(app, address, args.options.get("--origin").cloned())?;
        }
        "call" | "mcp" => {
            args.check(
                &["--url", "--token-file"],
                if args.command == "call" { 2 } else { 0 },
            )?;
            let secret = token(&args)?;
            let url = args.option("--url", "http://127.0.0.1:8080");
            if args.command == "call" {
                let tool = args
                    .positional
                    .first()
                    .ok_or_else(|| invalid("Provide an operation name"))?;
                if !tool.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
                    return Err(invalid("Invalid operation name"));
                }
                let data = input(args.positional.get(1).map(String::as_str).unwrap_or("{}"))?;
                let (status, result) =
                    http::client(url, &secret, &format!("/api/tools/{tool}"), &data)?;
                if status >= 400 {
                    return Err(Error::new(status, "REMOTE_ERROR", result.encode()));
                }
                output(result);
            } else {
                for line in std::io::stdin().lock().lines() {
                    let line = line?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    let parsed = parse(&line);
                    match parsed {
                        Ok(request) => match http::client(url, &secret, "/mcp", &request) {
                            Ok((status, response)) if status < 400 => {
                                if response != Json::Null {
                                    output(response);
                                }
                            }
                            result => {
                                let message = match result {
                                    Ok((_, v)) => v.encode(),
                                    Err(e) => e.to_string(),
                                };
                                output(object([
                                    ("jsonrpc", "2.0".into()),
                                    ("id", request.get("id").clone()),
                                    (
                                        "error",
                                        object([
                                            ("code", Json::Number("-32000".into())),
                                            ("message", message.into()),
                                        ]),
                                    ),
                                ]));
                            }
                        },
                        Err(e) => output(object([
                            ("jsonrpc", "2.0".into()),
                            ("id", Json::Null),
                            (
                                "error",
                                object([
                                    ("code", Json::Number("-32700".into())),
                                    ("message", e.into()),
                                ]),
                            ),
                        ])),
                    }
                }
            }
        }
        "grant" => {
            args.check(
                &["--data", "--name", "--role", "--scopes", "--token-file"],
                0,
            )?;
            let file = token_path(&args)?;
            let root = Path::new(args.required("--data")?);
            let _store = Store::open(root)?;
            let mut auth = Auth::open(root)?;
            let role = args.required("--role")?;
            let default = match role {
                "reader" => "read",
                "manager" => "read,write,trace,manage",
                _ => "read,write,trace",
            };
            let scopes = args
                .option("--scopes", default)
                .split(',')
                .map(String::from)
                .collect::<Vec<_>>();
            output(issue(
                &mut auth,
                args.required("--name")?,
                role,
                &scopes,
                &file,
            )?);
        }
        "revoke" => {
            args.check(&["--data", "--id"], 0)?;
            let root = Path::new(args.required("--data")?);
            let _store = Store::open(root)?;
            Auth::open(root)?.revoke(args.required("--id")?)?;
            output(object([("revoked", true.into())]));
        }
        "retire" => {
            args.check(&["--data", "--id", "--revision", "--summary"], 0)?;
            let mut store = Store::open(Path::new(args.required("--data")?))?;
            let key = args.required("--id")?;
            let links = store
                .pages
                .values()
                .filter(|p| {
                    p.get("id").s() != key && markdown::references(p.get("body").s()).contains(key)
                })
                .map(|p| p.get("id").s())
                .collect::<Vec<_>>();
            if !links.is_empty() {
                return Err(invalid(format!(
                    "Repair incoming links before retirement: {}",
                    links.join(", ")
                )));
            }
            output(
                store.save(
                    "operator",
                    "Filesystem operator",
                    &object([
                        (
                            "operation_id",
                            format!("retire-{}", digest::random()?).into(),
                        ),
                        (
                            "updates",
                            vec![object([
                                ("id", key.into()),
                                ("expected_revision_id", args.required("--revision")?.into()),
                                ("summary", args.required("--summary")?.into()),
                                ("deleted", true.into()),
                            ])]
                            .into(),
                        ),
                    ]),
                )?,
            );
        }
        "backup" => {
            args.check(&["--data", "--to"], 0)?;
            let store = Store::open(Path::new(args.required("--data")?))?;
            store.backup(Path::new(args.required("--to")?))?;
            output(object([("backup", args.required("--to")?.into())]));
        }
        "restore" => {
            args.check(&["--data", "--from", "--token-file"], 0)?;
            let file = token_path(&args)?;
            let root = Path::new(args.required("--data")?);
            let original = Store::open(Path::new(args.required("--from")?))?;
            original.backup(root)?;
            let _restored = Store::open(root)?;
            let mut auth = Auth::open(root)?;
            auth.reset_after_restore()?;
            output(issue(
                &mut auth,
                "Restored operator",
                "manager",
                &[
                    "read".into(),
                    "write".into(),
                    "trace".into(),
                    "manage".into(),
                ],
                &file,
            )?);
        }
        "import" => {
            args.check(&["--data", "--directory"], 0)?;
            let mut store = Store::open(Path::new(args.required("--data")?))?;
            if !store.history.is_empty() {
                return Err(invalid(
                    "Import requires an empty store and never merges or overwrites existing history",
                ));
            }
            let directory = Path::new(args.required("--directory")?);
            let mut updates = Vec::new();
            let mut entries =
                fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if e.path().extension().is_some_and(|s| s == "md") {
                    if !e.file_type()?.is_file() {
                        return Err(invalid("Markdown import rejects symlinks and non-files"));
                    }
                    let path = e.path();
                    let id = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .ok_or_else(|| invalid("Invalid Markdown filename"))?;
                    if !store::id(id) {
                        return Err(invalid(format!("Invalid article filename: {id}")));
                    }
                    updates.push(markdown::from_markdown(id, &fs::read_to_string(&path)?)?);
                }
            }
            for u in &updates {
                store::validate_update(u)?;
            }
            let mut receipts = Vec::new();
            for (i, batch) in updates.chunks(10).enumerate() {
                receipts.push(store.save(
                    "operator",
                    "Filesystem operator",
                    &object([
                        (
                            "operation_id",
                            format!("import-{}-{i}", digest::random()?).into(),
                        ),
                        ("updates", batch.to_vec().into()),
                    ]),
                )?);
            }
            output(object([
                ("imported", updates.len().into()),
                ("prior_history_migrated", false.into()),
                ("receipts", receipts.into()),
            ]));
        }
        "export" => {
            args.check(&["--data", "--directory"], 0)?;
            let store = Store::open(Path::new(args.required("--data")?))?;
            let dest = Path::new(args.required("--directory")?);
            fs::create_dir(dest)?;
            fs::set_permissions(dest, fs::Permissions::from_mode(0o700))?;
            for (id, page) in &store.pages {
                private_file(
                    &dest.join(format!("{id}.md")),
                    markdown::to_markdown(page).as_bytes(),
                )?;
            }
            output(object([("exported", store.pages.len().into())]));
        }
        "import-trace" => {
            args.check(&["--data", "--file", "--title"], 0)?;
            let store = Store::open(Path::new(args.required("--data")?))?;
            output(evidence::import(
                &store.root,
                Path::new(args.required("--file")?),
                args.required("--title")?,
            )?);
        }
        "publish-media" => {
            args.check(&["--data", "--file", "--source"], 0)?;
            let store = Store::open(Path::new(args.required("--data")?))?;
            output(evidence::publish_media(
                &store.root,
                Path::new(args.required("--file")?),
                args.required("--source")?,
            )?);
        }
        "verify" => {
            args.check(&["--data"], 0)?;
            let root = Path::new(args.required("--data")?);
            let app = App::open(root, true, true)?;
            let mut traces = 0usize;
            let mut media = 0usize;
            for trace in evidence::catalog(&app.store.root)? {
                let snapshot = trace.get("snapshot").s();
                if !store::hash_id(snapshot) {
                    return Err(invalid("Invalid trace metadata"));
                }
                let mut f = fs::File::open(
                    app.store
                        .root
                        .join("traces")
                        .join(format!("{snapshot}.jsonl")),
                )?;
                let mut buffer = [0; 65536];
                let mut hash = digest::Sha256::new();
                loop {
                    let n = f.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buffer[..n]);
                }
                if digest::hex(&hash.finish()) != snapshot {
                    return Err(invalid("Trace checksum mismatch"));
                }
                traces += 1;
            }
            for entry in fs::read_dir(app.store.root.join("media"))? {
                let path = entry?.path();
                if path.extension().is_some_and(|s| s == "json") {
                    let meta = store::load_json(&path)?;
                    evidence::media(&app.store.root, meta.get("file").s())?;
                    media += 1;
                }
            }
            output(object([
                ("verified", true.into()),
                ("head", app.store.head.into()),
                ("articles", app.store.pages.len().into()),
                ("traces", traces.into()),
                ("media", media.into()),
            ]));
        }
        _ => return Err(invalid("Unknown command; use --help")),
    }
    Ok(())
}
