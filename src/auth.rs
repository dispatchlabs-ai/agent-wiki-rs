use crate::digest::{random, sha256};
use crate::json::{Json, object};
use crate::store::{Error, Result, atomic, hash_id, invalid, load_json, now, text};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct Principal {
    pub id: String,
    pub name: String,
    pub role: String,
    pub scopes: Vec<String>,
    pub csrf: String,
    pub cookie: bool,
}
impl Principal {
    pub fn can(&self, action: &str) -> bool {
        let role = match action {
            "read" => ["reader", "editor", "manager"].contains(&self.role.as_str()),
            "write" | "trace" => ["editor", "manager"].contains(&self.role.as_str()),
            "manage" => self.role == "manager",
            _ => false,
        };
        role && self.scopes.iter().any(|s| s == action)
    }
    pub fn require(&self, action: &str) -> Result<()> {
        if self.can(action) {
            Ok(())
        } else {
            Err(Error::new(
                403,
                "FORBIDDEN",
                format!("{action} permission required"),
            ))
        }
    }
}
#[derive(Clone)]
pub struct Auth {
    path: PathBuf,
    data: Arc<Json>,
    fenced: bool,
}
impl Auth {
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("auth.json");
        let data = load_json(&path)?;
        if !matches!(data.get("credentials"), Json::Array(_))
            || !matches!(data.get("sessions"), Json::Array(_))
        {
            return Err(invalid("Corrupt credential store"));
        }
        for c in data.get("credentials").array() {
            if !hash_id(c.get("id").s())
                || !["reader", "editor", "manager"].contains(&c.get("role").s())
                || !matches!(c.get("revoked"), Json::Bool(_))
                || !matches!(c.get("scopes"), Json::Array(_))
                || (c.has("expires_at") && c.get("expires_at").number().is_none())
                || c.get("scopes")
                    .array()
                    .iter()
                    .any(|s| !["read", "write", "trace", "manage"].contains(&s.s()))
            {
                return Err(invalid("Corrupt credential record"));
            }
        }
        for session in data.get("sessions").array() {
            if !hash_id(session.get("hash").s())
                || !hash_id(session.get("csrf").s())
                || session.get("expires").number().is_none()
                || !data
                    .get("credentials")
                    .array()
                    .iter()
                    .any(|c| c.get("id") == session.get("credential"))
            {
                return Err(invalid("Corrupt session record"));
            }
        }
        Ok(Self {
            path,
            data: Arc::new(data),
            fenced: false,
        })
    }
    fn persist(&mut self, next: Json) -> Result<()> {
        if self.fenced {
            return Err(Error::new(
                503,
                "AUTH_UNCERTAIN",
                "Restart to recover an uncertain credential write",
            ));
        }
        self.fenced = true;
        atomic(&self.path, next.encode().as_bytes())?;
        self.data = Arc::new(next);
        self.fenced = false;
        Ok(())
    }
    pub fn grant(&mut self, name: &str, role: &str, scopes: &[String]) -> Result<Json> {
        self.issue(name, role, scopes, None)
    }
    fn issue(
        &mut self,
        name: &str,
        role: &str,
        scopes: &[String],
        expiry: Option<usize>,
    ) -> Result<Json> {
        if name.trim().is_empty()
            || name.len() > 200
            || !["reader", "editor", "manager"].contains(&role)
        {
            return Err(invalid("Provide a name and reader, editor or manager role"));
        }
        if scopes
            .iter()
            .any(|s| !["read", "write", "trace", "manage"].contains(&s.as_str()))
            || !scopes.iter().any(|s| s == "read")
        {
            return Err(invalid(
                "Scopes must include read and may include write, trace, manage",
            ));
        }
        if role == "reader" && scopes.iter().any(|s| s != "read") {
            return Err(invalid(
                "Reader credentials cannot carry write or trace scopes",
            ));
        }
        if role != "manager" && scopes.iter().any(|s| s == "manage") {
            return Err(invalid("Only manager credentials can carry manage scope"));
        }
        let token = random()?;
        let key = sha256(token.as_bytes());
        let mut credential = object([
            ("id", key.clone().into()),
            ("name", name.into()),
            ("role", role.into()),
            (
                "scopes",
                scopes
                    .iter()
                    .map(|s| s.clone().into())
                    .collect::<Vec<Json>>()
                    .into(),
            ),
            ("created_at", now().into()),
            ("revoked", false.into()),
        ]);
        if let Some(expiry) = expiry {
            credential.insert("expires_at", expiry.into());
        }
        let mut next = (*self.data).clone();
        let mut credentials = next.get("credentials").array().to_vec();
        credentials.push(credential.clone());
        next.insert("credentials", credentials.into());
        self.persist(next)?;
        let mut result = credential;
        result.insert("token", token.into());
        Ok(result)
    }
    pub fn authorize(&self, secret: &str, cookie: bool) -> Result<Principal> {
        if self.fenced {
            return Err(Error::new(
                503,
                "AUTH_UNCERTAIN",
                "Restart to recover an uncertain credential write",
            ));
        }
        if !hash_id(secret) {
            return Err(Error::new(
                401,
                "UNAUTHORIZED",
                "Sign in or provide a valid bearer credential",
            ));
        }
        let key = sha256(secret.as_bytes());
        let mut csrf = String::new();
        let key = if cookie {
            let session = self
                .data
                .get("sessions")
                .array()
                .iter()
                .find(|s| {
                    s.get("hash").s() == key
                        && s.get("expires").number().unwrap_or(0) as u64 > now()
                })
                .ok_or_else(|| Error::new(401, "UNAUTHORIZED", "Session expired or revoked"))?;
            csrf = session.get("csrf").s().into();
            session.get("credential").s().to_string()
        } else {
            key
        };
        let credential = self
            .data
            .get("credentials")
            .array()
            .iter()
            .find(|c| {
                c.get("id").s() == key
                    && !c.get("revoked").boolean()
                    && c.get("expires_at")
                        .number()
                        .is_none_or(|expiry| expiry as u64 > now())
            })
            .ok_or_else(|| Error::new(401, "UNAUTHORIZED", "Credential expired or revoked"))?;
        Ok(Principal {
            id: key,
            name: credential.get("name").s().into(),
            role: credential.get("role").s().into(),
            scopes: credential
                .get("scopes")
                .array()
                .iter()
                .map(|s| s.s().into())
                .collect(),
            csrf,
            cookie,
        })
    }
    pub fn login(&mut self, token: &str) -> Result<String> {
        let who = self.authorize(token, false)?;
        let secret = random()?;
        let csrf = random()?;
        let mut next = (*self.data).clone();
        let mut sessions = next
            .get("sessions")
            .array()
            .iter()
            .filter(|s| s.get("expires").number().unwrap_or(0) as u64 > now())
            .cloned()
            .collect::<Vec<_>>();
        sessions.push(object([
            ("hash", sha256(secret.as_bytes()).into()),
            ("credential", who.id.into()),
            ("csrf", csrf.into()),
            ("expires", (now() + 43200).into()),
        ]));
        next.insert("sessions", sessions.into());
        self.persist(next)?;
        Ok(secret)
    }
    pub fn logout(&mut self, secret: &str) -> Result<()> {
        let hash = sha256(secret.as_bytes());
        let mut next = (*self.data).clone();
        next.insert(
            "sessions",
            next.get("sessions")
                .array()
                .iter()
                .filter(|s| s.get("hash").s() != hash)
                .cloned()
                .collect::<Vec<_>>()
                .into(),
        );
        self.persist(next)
    }
    pub fn list(&self) -> Json {
        self.data.get("credentials").clone()
    }
    pub fn revoke(&mut self, key: &str) -> Result<()> {
        let mut next = (*self.data).clone();
        let mut found = false;
        let credentials = next
            .get("credentials")
            .array()
            .iter()
            .map(|c| {
                let mut c = c.clone();
                if c.get("id").s() == key {
                    c.insert("revoked", true.into());
                    found = true;
                }
                c
            })
            .collect::<Vec<_>>();
        if !found {
            return Err(Error::new(404, "NOT_FOUND", "Credential not found"));
        }
        next.insert("credentials", credentials.into());
        self.persist(next)
    }
    pub fn grant_input(&mut self, input: &Json) -> Result<Json> {
        if input.has("expires_at")
            && input
                .get("expires_at")
                .number()
                .is_none_or(|n| n as u64 <= now())
        {
            return Err(invalid("expires_at must be a future Unix timestamp"));
        }
        let scopes = input
            .get("scopes")
            .array()
            .iter()
            .map(|s| s.s().into())
            .collect::<Vec<_>>();
        self.issue(
            text(input, "name", 200)?,
            text(input, "role", 20)?,
            &scopes,
            input.get("expires_at").number(),
        )
    }

    pub fn change(&mut self, input: &Json) -> Result<Json> {
        let key = text(input, "id", 64)?;
        let role = text(input, "role", 20)?;
        let scopes = input.get("scopes").array();
        if !["reader", "editor", "manager"].contains(&role)
            || !matches!(input.get("scopes"), Json::Array(_))
            || !scopes.iter().any(|s| s.s() == "read")
            || scopes.iter().any(|s| {
                !["read", "write", "trace", "manage"].contains(&s.s())
                    || (role == "reader" && s.s() != "read")
                    || (role != "manager" && s.s() == "manage")
            })
        {
            return Err(invalid("Invalid role and scopes"));
        }
        if input.has("expires_at")
            && input
                .get("expires_at")
                .number()
                .is_none_or(|n| n as u64 <= now())
        {
            return Err(invalid("expires_at must be a future Unix timestamp"));
        }
        let mut next = (*self.data).clone();
        let mut records = next.get("credentials").array().to_vec();
        let credential = records
            .iter_mut()
            .find(|c| c.get("id").s() == key && !c.get("revoked").boolean())
            .ok_or_else(|| {
                Error::new(
                    404,
                    "NOT_FOUND",
                    "Active credential not found; revoked credentials cannot be revived",
                )
            })?;
        credential.insert("role", role.into());
        credential.insert("scopes", input.get("scopes").clone());
        if input.has("expires_at") {
            credential.insert("expires_at", input.get("expires_at").clone());
        }
        let result = credential.clone();
        next.insert("credentials", records.into());
        self.persist(next)?;
        Ok(result)
    }
    pub fn reset_after_restore(&mut self, revoke_credentials: bool) -> Result<()> {
        let mut next = (*self.data).clone();
        next.insert("sessions", Vec::new().into());
        next.insert(
            "credentials",
            next.get("credentials")
                .array()
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    if revoke_credentials {
                        c.insert("revoked", true.into());
                    }
                    c
                })
                .collect::<Vec<_>>()
                .into(),
        );
        self.persist(next)
    }
}
