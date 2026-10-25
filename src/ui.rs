use crate::app::App;
use crate::auth::Principal;
use crate::json::{Json, object};
use crate::markdown::{escape, render};
use crate::store::{Error, Result};

pub fn shell(title: &str, body: &str, who: Option<&Principal>, conversations: bool) -> String {
    let nav=who.map(|p|format!("<nav><a href=\"/\">Articles</a>{}{}{}</nav><div class=\"identity\"><span>{}</span><form method=\"post\" action=\"/logout\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><button class=\"quiet\">Sign out</button></form></div>",
        if conversations&&p.can("trace"){"<a href=\"/conversations/\">Conversations</a>"}else{""},
        if p.can("write"){"<a href=\"/edit/\">New article</a>"}else{""},
        if p.can("manage"){"<a href=\"/access/\">Access</a>"}else{""},escape(&p.name),escape(&p.csrf))).unwrap_or_default();
    let csrf = who.map(|p| p.csrf.as_str()).unwrap_or("");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"csrf-token\" content=\"{}\"><title>{} · Agent Wiki</title><link rel=\"stylesheet\" href=\"/assets/style.css\"><script defer src=\"/assets/browser.js\"></script></head><body><a class=\"skip\" href=\"#main\">Skip to content</a><header><a class=\"brand\" href=\"/\"><span class=\"brand-mark\">a</span> Agent Wiki</a>{nav}</header><main id=\"main\">{body}</main><footer>Agent Wiki · Rust alpha · Local revision journal</footer></body></html>",
        escape(csrf),
        escape(title)
    )
}
pub fn login() -> String {
    shell(
        "Sign in",
        "<section class=\"login\"><p class=\"eyebrow\">YOUR KNOWLEDGE, CONNECTED</p><h1>Welcome back.</h1><p>Sign in with a credential issued by your wiki operator.</p><form method=\"post\" action=\"/login\"><label>Access credential<input name=\"token\" type=\"password\" autocomplete=\"off\" required minlength=\"64\" maxlength=\"64\" spellcheck=\"false\"></label><button>Sign in</button></form><p class=\"muted\">This alpha uses independent bearer credentials. Existing Wiki passwords, passkeys and Google accounts cannot be used here.</p></section>",
        None,
        false,
    )
}
pub fn catalog(app: &mut App, who: &Principal, q: &str, offset: usize) -> Result<String> {
    let result = if q.is_empty() {
        app.call(
            who,
            "wiki.list",
            &object([("offset", offset.into()), ("limit", 40usize.into())]),
        )?
    } else {
        app.call(
            who,
            "wiki.search",
            &object([
                ("q", q.into()),
                ("offset", offset.into()),
                ("limit", 40usize.into()),
            ]),
        )?
    };
    let mut body = format!(
        "<section class=\"page-heading\"><p class=\"eyebrow\">THE KNOWLEDGE COLLECTION</p><h1>{}</h1><p>Useful explanations. Connected ideas. A record of what changed.</p></section><form class=\"search\" method=\"get\" action=\"/\"><label class=\"sr-only\" for=\"q\">Search articles</label><input id=\"q\" name=\"q\" placeholder=\"Search your knowledge…\" value=\"{}\"><button>Search</button></form><div class=\"section-label\">{} articles</div><section class=\"cards\">",
        if q.is_empty() {
            "Your wiki".into()
        } else {
            format!("Results for “{}”", escape(q))
        },
        escape(q),
        result.get("total").s_number()
    );
    for p in result.get("articles").array() {
        body.push_str(&format!("<article class=\"card\"><p class=\"eyebrow\">{}</p><h2><a href=\"{}\">{}</a></h2><p>{}</p><span class=\"muted\">Revision {}</span></article>",escape(p.get("topic").s()),escape(p.get("url").s()),escape(p.get("title").s()),escape(p.get("description").s()),p.get("number").s_number()));
    }
    if result.get("articles").array().is_empty() {
        body.push_str("<div class=\"empty\"><h2>Nothing here yet.</h2><p>Create an article or try a different search.</p></div>");
    }
    body.push_str("</section>");
    if let Some(n) = result.get("nextOffset").number() {
        body.push_str(&format!(
            "<a class=\"button\" href=\"/?offset={n}&amp;q={}\">Next page →</a>",
            crate::http::encode(q)
        ));
    }
    Ok(shell("Articles", &body, Some(who), app.conversations))
}
pub fn article(
    app: &mut App,
    who: &Principal,
    key: &str,
    revision: Option<usize>,
) -> Result<String> {
    let mut args = object([("id", key.into())]);
    if let Some(n) = revision {
        args.insert("revision", n.into());
    }
    let p = app.call(who, "wiki.read", &args)?;
    let mut body = "<div class=\"article-layout\"><aside class=\"toc\"><a href=\"/\">← All articles</a><strong>On this page</strong>".to_string();
    for section in p.get("sections").array() {
        body.push_str(&format!(
            "<a href=\"#{}\">{}</a>",
            escape(section.get("anchor").s()),
            escape(section.get("heading").s())
        ));
    }
    body.push_str(&format!("</aside><article><div class=\"article-heading\"><p class=\"eyebrow\">{}</p><h1>{}</h1><p class=\"lead\">{}</p><div class=\"article-actions\"><span>Revision {} · {}</span><a href=\"/history/{key}/\">History</a>{}</div></div><div class=\"prose\">{}</div>",escape(p.get("topic").s()),escape(p.get("title").s()),escape(p.get("description").s()),p.get("number").s_number(),escape(p.get("author").s()),if who.can("write")&&app.writes{format!("<a href=\"/edit/{key}/\">Edit article</a>")}else{String::new()},render(p.get("body").s())));
    if !p.get("backlinks").array().is_empty() {
        body.push_str("<section class=\"backlinks\"><h2>Linked from</h2><ul>");
        for link in p.get("backlinks").array() {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                escape(link.get("url").s()),
                escape(link.get("title").s())
            ));
        }
        body.push_str("</ul></section>");
    }
    body.push_str("</article></div>");
    Ok(shell(
        p.get("title").s(),
        &body,
        Some(who),
        app.conversations,
    ))
}
pub fn editor(app: &App, who: &Principal, key: &str) -> Result<String> {
    who.require("write")?;
    if !app.writes {
        return Err(Error::new(
            403,
            "WRITES_DISABLED",
            "Article writes are disabled",
        ));
    }
    let p = if key.is_empty() {
        object([])
    } else {
        app.store.page(key, None)?
    };
    let mut form = String::from(
        "<section class=\"page-heading\"><p class=\"eyebrow\">BUILD SHARED UNDERSTANDING</p><h1>Edit an article</h1></section><form id=\"editor\" class=\"editor\"><div class=\"field-grid\">",
    );
    for (name, label, value) in [
        ("id", "Article ID", key),
        ("title", "Title", p.get("title").s()),
        ("topic", "Topic", p.get("topic").s()),
        ("description", "Description", p.get("description").s()),
    ] {
        form.push_str(&format!(
            "<label>{label}<input name=\"{name}\" value=\"{}\" {} {}></label>",
            escape(value),
            if name == "id" && !key.is_empty() {
                "readonly"
            } else {
                ""
            },
            if name == "id" || name == "title" {
                "required"
            } else {
                ""
            }
        ));
    }
    form.push_str(&format!("</div><input name=\"revision\" type=\"hidden\" value=\"{}\"><label>Markdown<textarea name=\"body\" rows=\"24\" required spellcheck=\"true\">{}</textarea></label><label>Change summary<input name=\"summary\" required maxlength=\"1000\" placeholder=\"What changed, and why?\"></label><div class=\"actions\"><button type=\"submit\">Save article</button><button id=\"preview-button\" type=\"button\" class=\"secondary\">Preview</button><a href=\"/\">Cancel</a></div><p id=\"status\" role=\"status\"></p></form><section id=\"preview\" class=\"prose\" hidden></section>",escape(p.get("revision_id").s()),escape(p.get("body").s())));
    Ok(shell("Edit article", &form, Some(who), app.conversations))
}
pub fn history(app: &mut App, who: &Principal, key: &str) -> Result<String> {
    let h = app.call(who, "wiki.history", &object([("id", key.into())]))?;
    let mut body = format!(
        "<section class=\"page-heading\"><p class=\"eyebrow\">REVISION HISTORY</p><h1>{}</h1><a href=\"/wiki/{key}/\">Current article →</a></section><ol class=\"history\">",
        escape(key)
    );
    for p in h.get("revisions").array() {
        body.push_str(&format!("<li><a href=\"{}\">Revision {}</a><strong>{}</strong><span class=\"muted\">{} {}</span></li>",escape(p.get("url").s()),p.get("number").s_number(),escape(p.get("summary").s()),escape(p.get("author").s()),if p.get("deleted").boolean(){"· Retired"}else{""}));
    }
    body.push_str("</ol>");
    Ok(shell("History", &body, Some(who), app.conversations))
}
pub fn access(app: &App, who: &Principal) -> Result<String> {
    who.require("manage")?;
    let mut body = String::from(
        "<section class=\"page-heading\"><p class=\"eyebrow\">ACCESS DIRECTORY</p><h1>People and agents</h1><p>Independent credentials, explicit roles, immediate revocation.</p></section><div class=\"table-wrap\"><table><thead><tr><th>Name</th><th>Role</th><th>Scopes</th><th>Status</th><th></th></tr></thead><tbody>",
    );
    for c in app.auth.list().array() {
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(c.get("name").s()),
            escape(c.get("role").s()),
            escape(
                &c.get("scopes")
                    .array()
                    .iter()
                    .map(Json::s)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            if c.get("revoked").boolean() {
                "Revoked"
            } else {
                "Active"
            },
            if c.get("revoked").boolean() {
                String::new()
            } else {
                format!(
                    "<button class=\"quiet revoke\" data-id=\"{}\">Revoke</button>",
                    escape(c.get("id").s())
                )
            }
        ));
    }
    body.push_str("</tbody></table></div><section class=\"grant\"><h2>Issue a credential</h2><form id=\"grant\"><div class=\"field-grid\"><label>Name<input name=\"name\" required maxlength=\"200\"></label><label>Role<select name=\"role\"><option>reader</option><option>editor</option><option>manager</option></select></label></div><button>Create credential</button><p id=\"status\" role=\"status\"></p><label id=\"secret-label\" hidden>Save this credential now. It is shown only once.<textarea id=\"new-secret\" readonly rows=\"3\" spellcheck=\"false\"></textarea></label></form></section>");
    Ok(shell("Access", &body, Some(who), app.conversations))
}
pub fn conversations(
    app: &mut App,
    who: &Principal,
    key: &str,
    q: &str,
    offset: usize,
    raw: bool,
) -> Result<String> {
    let mut body;
    if key.is_empty() {
        let r = app.call(
            who,
            "wiki.traceSearch",
            &object([
                ("q", q.into()),
                ("offset", offset.into()),
                ("limit", 40usize.into()),
            ]),
        )?;
        body = format!(
            "<section class=\"page-heading\"><p class=\"eyebrow\">ORIGINAL EVIDENCE · EDITORIAL ACCESS</p><h1>Conversations</h1></section><form class=\"search\"><input name=\"q\" aria-label=\"Search dialogue\" value=\"{}\" placeholder=\"Search recorded dialogue\"><button>Search</button></form><section class=\"cards\">",
            escape(q)
        );
        for t in r.get("results").array() {
            body.push_str(&format!(
                "<article class=\"card\"><h2><a href=\"{}\">{}</a></h2><p>{}</p></article>",
                escape(t.get("url").s()),
                escape(t.get("title").s()),
                escape(t.get("snippet").s())
            ));
        }
        body.push_str("</section>");
        if let Some(n) = r.get("nextOffset").number() {
            body.push_str(&format!(
                "<a href=\"?q={}&amp;offset={n}\">Next page →</a>",
                crate::http::encode(q)
            ));
        }
    } else {
        let r = app.call(
            who,
            "wiki.trace",
            &object([
                ("id", key.into()),
                ("offset", offset.into()),
                ("limit", 40usize.into()),
                ("kind", if raw { "raw" } else { "dialogue" }.into()),
            ]),
        )?;
        body = format!(
            "<section class=\"page-heading\"><p class=\"eyebrow\">IMMUTABLE ORIGINAL SOURCE</p><h1>Conversation</h1><p class=\"mono\">{}</p><a href=\"?kind={}\">{}</a></section>",
            escape(key),
            if raw { "dialogue" } else { "raw" },
            if raw {
                "Show dialogue"
            } else {
                "Show raw records, including tools and reasoning"
            }
        );
        for m in r.get("messages").array() {
            let number = m.get("line").s_number();
            body.push_str(&format!("<section class=\"message\" id=\"line-{number}\"><a href=\"{}\">Line {number}</a><strong>{}</strong><pre>{}</pre></section>",escape(m.get("url").s()),escape(m.get("role").s()),escape(&if raw{m.get("record").encode()}else{m.get("text").s().to_string()})));
        }
        if let Some(n) = r.get("nextOffset").number() {
            body.push_str(&format!(
                "<a href=\"?offset={n}&amp;kind={}\">Continue →</a>",
                if raw { "raw" } else { "dialogue" }
            ));
        }
    }
    Ok(shell("Conversations", &body, Some(who), app.conversations))
}
