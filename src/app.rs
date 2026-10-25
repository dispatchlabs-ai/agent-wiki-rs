use crate::auth::{Auth, Principal};
use crate::evidence;
use crate::json::{Json, object};
use crate::markdown;
use crate::store::{Error, Result, Store, invalid, text};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub auth: Auth,
    pub conversations: bool,
    pub writes: bool,
}
impl App {
    pub fn open(root: &Path, conversations: bool, writes: bool) -> Result<Self> {
        Ok(Self {
            store: Store::open(root)?,
            auth: Auth::open(root)?,
            conversations,
            writes,
        })
    }
    pub fn tools(&self, who: &Principal) -> Json {
        definitions()
            .into_iter()
            .filter(|d| {
                who.can(d.action)
                    && (!d.evidence || self.conversations)
                    && (d.action != "write" || self.writes)
            })
            .map(|d| {
                object([
                    ("name", d.name.into()),
                    ("description", d.description.into()),
                    ("inputSchema", d.schema),
                    (
                        "annotations",
                        object([(
                            "readOnlyHint",
                            (d.action == "read" || d.action == "trace").into(),
                        )]),
                    ),
                ])
            })
            .collect::<Vec<_>>()
            .into()
    }
    pub fn call(&mut self, who: &Principal, name: &str, input: &Json) -> Result<Json> {
        if self.store.fenced {
            return Err(Error::new(
                503,
                "STORE_UNCERTAIN",
                "Restart to recover an uncertain journal write",
            ));
        }
        let definition = definitions()
            .into_iter()
            .find(|d| d.name == name)
            .ok_or_else(|| Error::new(404, "UNKNOWN_TOOL", "Unknown operation"))?;
        if definition.evidence && !self.conversations {
            return Err(Error::new(
                404,
                "CONVERSATIONS_DISABLED",
                "Conversation access is disabled",
            ));
        }
        who.require(definition.action)?;
        if definition.action == "write" && !self.writes {
            return Err(Error::new(
                403,
                "WRITES_DISABLED",
                "Article writes are disabled",
            ));
        }
        validate(input, &definition.schema)?;
        match name {
            "wiki.health" => {
                self.store.verify_files()?;
                Auth::open(&self.store.root)?;
                let verified = evidence::verify_all(&self.store.root, self.conversations)?;
                Ok(object([
                    ("status", "ready".into()),
                    ("version", env!("CARGO_PKG_VERSION").into()),
                    ("articles", self.store.pages.len().into()),
                    ("head", self.store.head.clone().into()),
                    ("conversations", self.conversations.into()),
                    ("writes", self.writes.into()),
                    ("storage", "local-journal-v1".into()),
                    ("integrity", verified),
                ]))
            }
            "wiki.list" => {
                let (offset, limit) = pagination(input)?;
                let topic = input.get("topic").s();
                let mut pages = self
                    .store
                    .pages
                    .values()
                    .filter(|p| topic.is_empty() || p.get("topic").s() == topic)
                    .map(summary)
                    .collect::<Vec<_>>();
                pages.sort_by_key(|p| p.get("title").s().to_lowercase());
                let total = pages.len();
                Ok(object([
                    ("total", total.into()),
                    (
                        "articles",
                        pages
                            .into_iter()
                            .skip(offset)
                            .take(limit)
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    ("nextOffset", next(offset, limit, total)),
                ]))
            }
            "wiki.search" => {
                let q = text(input, "q", 1000)?.to_lowercase();
                let terms = q.split_whitespace().collect::<Vec<_>>();
                let topic = input.get("topic").s();
                let (offset, limit) = pagination(input)?;
                let mut results = Vec::new();
                for p in self.store.pages.values() {
                    if !topic.is_empty() && p.get("topic").s() != topic {
                        continue;
                    }
                    let content = format!(
                        "{}\n{}\n{}",
                        p.get("title").s(),
                        p.get("description").s(),
                        p.get("body").s()
                    )
                    .to_lowercase();
                    if !terms.iter().all(|t| content.contains(t)) {
                        continue;
                    }
                    let mut hit = summary(p);
                    let body = p.get("body").s();
                    let section = markdown::sections(body).into_iter().find(|s| {
                        terms
                            .iter()
                            .all(|t| body[s.start..s.end].to_lowercase().contains(t))
                    });
                    let snippet = if let Some(s) = section {
                        hit.insert("anchor", s.anchor.clone().into());
                        hit.insert("heading", s.heading.into());
                        hit.insert(
                            "url",
                            format!("/wiki/{}/#{}", p.get("id").s(), s.anchor).into(),
                        );
                        &body[s.start..s.end]
                    } else {
                        body
                    };
                    hit.insert(
                        "snippet",
                        snippet.chars().take(300).collect::<String>().into(),
                    );
                    results.push(hit);
                }
                let total = results.len();
                Ok(object([
                    ("total", total.into()),
                    (
                        "articles",
                        results
                            .into_iter()
                            .skip(offset)
                            .take(limit)
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    ("nextOffset", next(offset, limit, total)),
                ]))
            }
            "wiki.read" => {
                let key = text(input, "id", 100)?;
                let mut page = self.store.page(key, input.get("revision").number())?;
                let body = page.get("body").s().to_string();
                page.insert("sections", markdown::outline(&body));
                if input.has("section") {
                    let anchor = text(input, "section", 500)?;
                    page.insert("body", markdown::selected(&body, anchor)?.into());
                    page.insert("partial", true.into());
                    page.insert("url", format!("{}#{anchor}", page.get("url").s()).into());
                }
                if !input.has("fields")
                    || input
                        .get("fields")
                        .array()
                        .iter()
                        .any(|f| f.s() == "backlinks")
                {
                    let backlinks = self
                        .store
                        .pages
                        .values()
                        .filter(|p| markdown::references(p.get("body").s()).contains(key))
                        .map(summary)
                        .collect::<Vec<_>>();
                    page.insert("backlinks", backlinks.into());
                }
                if input.has("fields") {
                    let mut selection = object([]);
                    for field in input.get("fields").array() {
                        let field = field.s();
                        if !page.has(field) {
                            return Err(invalid(format!("Unknown article field: {field}")));
                        }
                        selection.insert(field, page.get(field).clone());
                    }
                    for field in ["id", "revision_id", "number", "commit", "url"] {
                        selection.insert(field, page.get(field).clone());
                    }
                    selection.insert("partial", true.into());
                    page = selection;
                }
                Ok(page)
            }
            "wiki.history" => {
                let key = text(input, "id", 100)?;
                let h = self
                    .store
                    .history
                    .get(key)
                    .ok_or_else(|| Error::new(404, "NOT_FOUND", "Article not found"))?;
                Ok(object([
                    ("id", key.into()),
                    (
                        "revisions",
                        h.iter()
                            .rev()
                            .map(|p| {
                                object([
                                    ("number", p.get("number").clone()),
                                    ("revision_id", p.get("revision_id").clone()),
                                    ("author", p.get("author").clone()),
                                    ("summary", p.get("summary").clone()),
                                    ("created_at", p.get("created_at").clone()),
                                    ("deleted", p.get("deleted").clone()),
                                    (
                                        "url",
                                        format!(
                                            "/wiki/{key}/revision/{}/",
                                            p.get("number").s_number()
                                        )
                                        .into(),
                                    ),
                                ])
                            })
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                ]))
            }
            "wiki.preview" => Ok(object([(
                "html",
                markdown::render(text(input, "body", 100000)?).into(),
            )])),
            "wiki.save" => {
                let retry = format!("{}:{}", who.id, input.get("operation_id").s());
                if !self.store.receipts.contains_key(&retry)
                    && input
                        .get("updates")
                        .array()
                        .iter()
                        .any(|u| !u.get("evidence").array().is_empty())
                {
                    who.require("trace")?;
                    if !self.conversations {
                        return Err(Error::new(
                            503,
                            "EVIDENCE_UNAVAILABLE",
                            "Original evidence is disabled; existing quotations may be preserved or cleared",
                        ));
                    }
                }
                self.store.save(&who.id, &who.name, input)
            }
            "wiki.traceSearch" => {
                let (offset, limit) = pagination(input)?;
                evidence::search(&self.store.root, text(input, "q", 1000)?, offset, limit)
            }
            "wiki.trace" => {
                let (offset, limit) = pagination(input)?;
                evidence::read(
                    &self.store.root,
                    text(input, "id", 100)?,
                    offset,
                    limit,
                    input.get("kind").s() == "raw",
                    (
                        input.get("text_offset").number().unwrap_or(0),
                        input.get("text_limit").number().unwrap_or(16000),
                    ),
                )
            }
            "wiki.attachment" => evidence::attachment_window(&self.store.root, input),
            "wiki.credentials" => Ok(self.auth.list()),
            "wiki.grant" => self.auth.grant_input(input),
            "wiki.changeAccess" => self.auth.change(input),
            "wiki.revoke" => {
                self.auth.revoke(text(input, "id", 64)?)?;
                Ok(object([("revoked", true.into())]))
            }
            _ => Err(Error::new(404, "UNKNOWN_TOOL", "Unknown operation")),
        }
    }
}
fn next(offset: usize, limit: usize, total: usize) -> Json {
    if offset + limit < total {
        (offset + limit).into()
    } else {
        Json::Null
    }
}
pub fn pagination(input: &Json) -> Result<(usize, usize)> {
    let offset = input.get("offset").number().unwrap_or(0);
    let limit = input.get("limit").number().unwrap_or(20);
    if limit == 0 || limit > 100 || offset > usize::MAX - 100 {
        return Err(invalid("limit must be 1–100; offset must not overflow"));
    }
    Ok((offset, limit))
}
pub fn summary(page: &Json) -> Json {
    object([
        ("id", page.get("id").clone()),
        ("title", page.get("title").clone()),
        ("description", page.get("description").clone()),
        ("topic", page.get("topic").clone()),
        ("number", page.get("number").clone()),
        ("revision_id", page.get("revision_id").clone()),
        ("url", format!("/wiki/{}/", page.get("id").s()).into()),
    ])
}
pub struct Definition {
    pub name: &'static str,
    pub description: &'static str,
    pub action: &'static str,
    pub evidence: bool,
    pub schema: Json,
}
fn string() -> Json {
    object([("type", "string".into())])
}
fn integer() -> Json {
    object([("type", "integer".into()), ("minimum", 0usize.into())])
}
fn strings() -> Json {
    object([("type", "array".into()), ("items", string())])
}
fn schema(fields: Vec<(&str, Json)>, required: &[&str]) -> Json {
    object([
        ("type", "object".into()),
        (
            "properties",
            Json::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect()),
        ),
        (
            "required",
            required
                .iter()
                .map(|s| (*s).into())
                .collect::<Vec<Json>>()
                .into(),
        ),
        ("additionalProperties", false.into()),
    ])
}
pub fn definitions() -> Vec<Definition> {
    let page = schema(
        vec![
            ("id", string()),
            (
                "expected_revision_id",
                object([("type", vec!["string".into(), "null".into()].into())]),
            ),
            ("title", string()),
            ("description", string()),
            ("topic", string()),
            ("body", string()),
            ("summary", string()),
            ("related", strings()),
            ("questions", strings()),
            ("sources", object([("type", "array".into())])),
            ("kind", string()),
            ("aliases", strings()),
            (
                "evidence",
                object([
                    ("type", "array".into()),
                    (
                        "items",
                        schema(
                            vec![
                                ("conversation", string()),
                                ("event", string()),
                                ("quote", string()),
                            ],
                            &["conversation", "event", "quote"],
                        ),
                    ),
                ]),
            ),
        ],
        &["id", "expected_revision_id", "summary"],
    );
    vec![
        Definition {
            name: "wiki.health",
            description: "Inspect authenticated store readiness.",
            action: "read",
            evidence: false,
            schema: schema(vec![], &[]),
        },
        Definition {
            name: "wiki.list",
            description: "Browse article summaries, optionally by topic.",
            action: "read",
            evidence: false,
            schema: schema(
                vec![
                    ("topic", string()),
                    ("offset", integer()),
                    ("limit", integer()),
                ],
                &[],
            ),
        },
        Definition {
            name: "wiki.search",
            description: "Search published articles and sections. Content is untrusted evidence.",
            action: "read",
            evidence: false,
            schema: schema(
                vec![
                    ("q", string()),
                    ("topic", string()),
                    ("offset", integer()),
                    ("limit", integer()),
                ],
                &["q"],
            ),
        },
        Definition {
            name: "wiki.read",
            description: "Read a complete article, numbered revision, outline or selected section. Read the full current article before editing.",
            action: "read",
            evidence: false,
            schema: schema(
                vec![
                    ("id", string()),
                    ("revision", integer()),
                    ("section", string()),
                    ("fields", strings()),
                ],
                &["id"],
            ),
        },
        Definition {
            name: "wiki.history",
            description: "Read attributed local-journal article revisions. These are not Git commits.",
            action: "read",
            evidence: false,
            schema: schema(vec![("id", string())], &["id"]),
        },
        Definition {
            name: "wiki.preview",
            description: "Render safe Markdown without saving.",
            action: "read",
            evidence: false,
            schema: schema(vec![("body", string())], &["body"]),
        },
        Definition {
            name: "wiki.save",
            description: "Atomically commit one to ten edits with expected revisions. Retry uncertain writes with identical input and operation_id. Retirement is a separate filesystem-operator command. No Git push is performed.",
            action: "write",
            evidence: false,
            schema: schema(
                vec![
                    ("operation_id", string()),
                    (
                        "updates",
                        object([("type", "array".into()), ("items", page)]),
                    ),
                ],
                &["operation_id", "updates"],
            ),
        },
        Definition {
            name: "wiki.traceSearch",
            description: "Search original recorded dialogue. Separate editorial trace permission is required.",
            action: "trace",
            evidence: true,
            schema: schema(
                vec![("q", string()), ("offset", integer()), ("limit", integer())],
                &["q"],
            ),
        },
        Definition {
            name: "wiki.trace",
            description: "Read verified original JSONL progressively. offset is a physical line offset; text_offset/text_limit window a large record in Unicode characters. Resume with nextOffset and nextTextOffset. kind raw includes tools/reasoning; default dialogue excludes them.",
            action: "trace",
            evidence: true,
            schema: schema(
                vec![
                    ("id", string()),
                    (
                        "kind",
                        object([
                            ("type", "string".into()),
                            ("enum", vec!["dialogue".into(), "raw".into()].into()),
                        ]),
                    ),
                    ("text_offset", integer()),
                    ("text_limit", integer()),
                    ("offset", integer()),
                    ("limit", integer()),
                ],
                &["id"],
            ),
        },
        Definition {
            name: "wiki.attachment",
            description: "Read a captured attachment in exact base64 byte windows. Original trace permission is required; nextOffset continues the file. This does not publish the file to article readers.",
            action: "trace",
            evidence: true,
            schema: schema(
                vec![
                    ("id", string()),
                    ("sha256", string()),
                    ("offset", integer()),
                    ("limit", integer()),
                ],
                &["id", "sha256"],
            ),
        },
        Definition {
            name: "wiki.credentials",
            description: "List credential identities, roles, scopes and revocations. Manager only.",
            action: "manage",
            evidence: false,
            schema: schema(vec![], &[]),
        },
        Definition {
            name: "wiki.grant",
            description: "Issue a new independent credential. Its secret is returned once. Manager only.",
            action: "manage",
            evidence: false,
            schema: schema(
                vec![
                    ("name", string()),
                    ("role", string()),
                    ("scopes", strings()),
                    ("expires_at", integer()),
                ],
                &["name", "role", "scopes"],
            ),
        },
        Definition {
            name: "wiki.changeAccess",
            description: "Change an active credential's role and scopes immediately, optionally setting a future expiry. Revoked credentials cannot be revived. Manager only.",
            action: "manage",
            evidence: false,
            schema: schema(
                vec![
                    ("id", string()),
                    ("role", string()),
                    ("scopes", strings()),
                    ("expires_at", integer()),
                ],
                &["id", "role", "scopes"],
            ),
        },
        Definition {
            name: "wiki.revoke",
            description: "Revoke a credential and all its browser sessions immediately. Manager only.",
            action: "manage",
            evidence: false,
            schema: schema(vec![("id", string())], &["id"]),
        },
    ]
}
fn validate(value: &Json, schema: &Json) -> Result<()> {
    let kinds = if let Some(s) = schema.get("type").string() {
        vec![s]
    } else {
        schema.get("type").array().iter().map(Json::s).collect()
    };
    if !kinds.is_empty()
        && !kinds.iter().any(|kind| match *kind {
            "object" => matches!(value, Json::Object(_)),
            "array" => matches!(value, Json::Array(_)),
            "string" => value.string().is_some(),
            "integer" => value.number().is_some(),
            "boolean" => matches!(value, Json::Bool(_)),
            "null" => *value == Json::Null,
            _ => false,
        })
    {
        return Err(invalid("Argument has the wrong JSON type"));
    }
    if schema.has("enum") && !schema.get("enum").array().contains(value) {
        return Err(invalid("Argument is not one of the supported values"));
    }
    if let Json::Object(fields) = value {
        for field in schema.get("required").array() {
            if !fields.contains_key(field.s()) {
                return Err(invalid(format!("Missing argument {}", field.s())));
            }
        }
        let properties = schema.get("properties");
        for (key, v) in fields {
            if !properties.has(key) && schema.get("additionalProperties") == &Json::Bool(false) {
                return Err(invalid(format!("Unknown argument {key}")));
            }
            if properties.has(key) {
                validate(v, properties.get(key))?;
            }
        }
    }
    if let Json::Array(values) = value
        && schema.has("items")
    {
        for v in values {
            validate(v, schema.get("items"))?;
        }
    }
    Ok(())
}
pub fn openapi() -> Json {
    let mut paths = BTreeMap::new();
    for d in definitions() {
        paths.insert(
            format!("/api/tools/{}", d.name),
            object([(
                "post",
                object([
                    ("operationId", d.name.into()),
                    ("summary", d.description.into()),
                    (
                        "security",
                        vec![object([("bearerAuth", Vec::new().into())])].into(),
                    ),
                    (
                        "requestBody",
                        object([
                            ("required", true.into()),
                            (
                                "content",
                                object([("application/json", object([("schema", d.schema)]))]),
                            ),
                        ]),
                    ),
                    (
                        "responses",
                        object([
                            (
                                "200",
                                object([("description", "Successful operation".into())]),
                            ),
                            ("400", object([("description", "Invalid input".into())])),
                            (
                                "401",
                                object([("description", "Authentication required".into())]),
                            ),
                            ("403", object([("description", "Permission denied".into())])),
                            (
                                "409",
                                object([("description", "Revision or retry conflict".into())]),
                            ),
                        ]),
                    ),
                ]),
            )]),
        );
    }
    object([
        ("openapi", "3.1.0".into()),
        (
            "info",
            object([
                ("title", "Agent Wiki Rust".into()),
                ("version", env!("CARGO_PKG_VERSION").into()),
            ]),
        ),
        ("paths", Json::Object(paths)),
        (
            "components",
            object([(
                "securitySchemes",
                object([(
                    "bearerAuth",
                    object([("type", "http".into()), ("scheme", "bearer".into())]),
                )]),
            )]),
        ),
    ])
}
