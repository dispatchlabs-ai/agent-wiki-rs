# Agent Wiki use cases

This document defines 61 high-level intended use cases in 10 groups for Agent Wiki. Each use case has a stable ID and describes the goal of a person, agent, or integration.

## Terms

| Term                    | Meaning in this inventory                                                                            |
| ----------------------- | ---------------------------------------------------------------------------------------------------- |
| Article                 | A maintained Markdown document with revision history and links to related knowledge.                 |
| Knowledge space         | A collection of articles and associated evidence governed by shared access rules.                    |
| Conversation / evidence | The original exchange, recorded activity, and captured files that support the knowledge in articles. |
| Agent definition        | The versioned instructions and tool choices describing how a named agent uses the wiki.              |
| Connection              | An authorization linking a client or machine to a named wiki agent.                                  |

“Reader,” “editor,” “manager,” “operator,” “integrator,” and “agent owner” describe responsibilities in a use case; readers and editors may be people or authorized agents.

## 1. Establish shared knowledge

- **AW-001 — Try the complete workflow.** A prospective user tries a synthetic example, follows its sources, and previews or saves an edit to understand the workflow.
- **AW-002 — Keep an owned knowledge collection.** A knowledge owner maintains a shared knowledge space of Markdown articles and owns its revision history.
- **AW-003 — Recall prior knowledge.** A person or agent finds recorded facts, decisions, and procedures relevant to ongoing work.
- **AW-004 — Preserve useful learning.** An authorized agent preserves useful, verified knowledge so later participants can build on it.

## 2. Find and read articles

- **AW-005 — Find relevant knowledge.** A reader searches the collection to find knowledge relevant to a question.
- **AW-006 — Browse the collection.** A reader explores the article catalog and topics to discover available knowledge.
- **AW-007 — Follow related knowledge.** A reader follows article links and backlinks to understand connected subjects.
- **AW-008 — Read the relevant passage.** A reader uses an article's outline, selected passages, or metadata to find and cite the detail they need.
- **AW-009 — Understand a rich document.** A reader understands an article through its prose, tables, diagrams, mathematics, and other supported formatting.
- **AW-010 — Read an earlier account.** A reader opens a historical article revision and cites that particular version.
- **AW-011 — Open published media.** A reader opens images or PDFs published with an article as part of its explanation.

## 3. Maintain the knowledge collection

- **AW-012 — Create an article.** An editor adds a durable explanation of a subject to the knowledge collection.
- **AW-013 — Revise an article.** An editor updates an existing explanation as new information or decisions become available.
- **AW-014 — Connect related subjects.** An editor links articles so readers can follow relationships and discover supporting context.
- **AW-015 — Make knowledge discoverable.** An editor maintains useful titles, descriptions, topics, and relationships for the collection.
- **AW-016 — Inspect a draft.** An editor previews how proposed Markdown will appear before saving it.
- **AW-017 — Support a claim with evidence.** An editor connects claims to their sources and checks quoted evidence before publishing it.
- **AW-018 — Update related articles together.** An editor saves related article changes together, including new articles that link to one another.
- **AW-019 — Retire an article coherently.** An operator removes an article and repairs incoming links while preserving its history.

## 4. Review and recover article changes

- **AW-020 — Review attributed history.** A reader inspects who changed an article, why it changed, and its retained revisions.
- **AW-021 — Resolve an editing conflict.** An editor reconciles a draft with changes made since the article was read.
- **AW-022 — Recover an uncertain save.** An editor recovers the result of an uncertain save without creating a duplicate change.
- **AW-023 — Understand publication progress.** An editor can tell whether a change is committed, pushed to remote storage, and available to readers.
- **AW-024 — Read newly committed knowledge.** A reader sees committed article updates without waiting for a content build or application redeployment.

## 5. Investigate original evidence

- **AW-025 — Find a relevant conversation.** An editorial reader searches recorded dialogue to locate the source behind a question or claim.
- **AW-026 — Read a source in context.** An editorial reader examines the original conversation around the evidence they need.
- **AW-027 — Inspect the recorded work.** An editorial reader examines recorded tool activity, reasoning, and other source material to understand what happened.
- **AW-028 — Understand source provenance.** An editorial reader follows session, snapshot, branch, and source relationships to understand where evidence came from.
- **AW-029 — Cite precise evidence.** An editorial reader links to the original event or source line supporting a claim.
- **AW-030 — Inspect a captured attachment.** An editorial reader previews or retrieves a captured file associated with a conversation.
- **AW-031 — Read a large source progressively.** An editorial reader explores a large source in manageable portions while retaining its original context.

## 6. Sign in and protect a personal account

- **AW-032 — Sign in as the intended person.** A person signs in with a local or federated identity and receives that identity's wiki access.
- **AW-033 — Maintain a local password.** A person changes their local password and ends other sessions when securing their account.
- **AW-034 — Protect sign-in with a second factor.** A person adds or manages passkeys, an authenticator app, or both to protect their sign-in.
- **AW-035 — Recover access after losing a factor.** A person regains access through a recovery code or a trusted operator after losing a sign-in factor.
- **AW-036 — End a browser session.** A person signs out of the current browser session when finished using the wiki.

## 7. Manage access to knowledge

- **AW-037 — Invite a local user.** A manager invites someone to create a local account and separately assigns their wiki access.
- **AW-038 — Assign wiki responsibilities.** A manager assigns or changes the wiki role of a person or agent.
- **AW-039 — Enroll company readers deliberately.** An operator gives new verified members of a chosen Google Workspace domain automatic reader access.
- **AW-040 — Inspect people and agent access.** A manager reviews who has wiki access and who created, owns, or can use each agent.
- **AW-041 — Withdraw wiki access.** A manager or operator removes an identity's access to protected wiki knowledge.

## 8. Define and connect agents

- **AW-042 — Define a named agent.** An agent owner creates a named agent with instructions and tools, then obtains its wiki access separately.
- **AW-043 — Revise an agent deliberately.** An agent owner or authorized editor changes its definition while existing connections retain the version they approved.
- **AW-044 — Share control of an agent.** An agent owner or access manager gives another person permission to use the agent, edit its definition, or manage its access.
- **AW-045 — Authorize a remote client.** A person connects a client through a chosen agent with approved permissions and clear attribution of both identities.
- **AW-046 — Keep a trusted machine connected.** An operator keeps a trusted machine connected under its own agent credential until expiry or revocation.
- **AW-047 — Delegate bounded authority.** An operator gives an agent limited authority to act for a named person within that person's current rights.
- **AW-048 — Disconnect an agent client.** A person or operator disconnects a selected agent client independently of browser sign-out.

## 9. Use the wiki through other tools

- **AW-049 — Use a preferred client.** A person or agent reads and maintains knowledge through a preferred supported client with consistent permissions.
- **AW-050 — Automate a content workflow.** An integrator automates article workflows through documented operations and structured results.
- **AW-051 — Retain identity across clients.** A terminal user or integration explicitly chooses a human or agent identity and retains its permissions and attribution.
- **AW-052 — Connect an existing evidence archive.** An operator connects an existing evidence service so authorized readers can inspect its conversations and files through the wiki.

## 10. Operate and recover a wiki

- **AW-053 — Host a shared wiki.** An operator hosts an authenticated wiki for an individual or trusted team with controlled HTTPS access.
- **AW-054 — Establish installation ownership.** An operator initializes a managed installation or brings existing content and identities under its care.
- **AW-055 — Choose article-only operation.** An operator turns conversation access off or on while preserving the archive, article citations, and published media.
- **AW-056 — Retain original conversations.** An operator imports supported conversations in full while preserving their original records.
- **AW-057 — Publish selected article assets.** An operator publishes selected images or PDFs with their source provenance for article readers.
- **AW-058 — Diagnose service readiness.** An operator identifies problems affecting article access, evidence availability, or search.
- **AW-059 — Keep derived data recoverable.** An operator rebuilds or relocates search indexes while preserving articles, original evidence, and citations.
- **AW-060 — Recover durable knowledge and access.** An operator backs up and restores the knowledge collection, its history, and the identities and permissions needed to use it.
- **AW-061 — Upgrade or repair an installation.** An operator upgrades the wiki or repairs interrupted maintenance while preserving its data and a recovery path.
