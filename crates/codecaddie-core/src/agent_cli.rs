//! One-shot `codecaddie-core agent <verb>` facade for headless coding bots.
//!
//! Every invocation prints exactly one JSON object on stdout — `{"ok":true,…}`
//! on success or `{"ok":false,"error":…,"message":…,"remediation":[…]}` on
//! failure — and exits 0 or 1 accordingly; stderr is reserved for logs. File
//! transfer happens only through the owner-only agent-exchange directories
//! under the channel data root (`agent-exchange/inbox` for payloads arriving
//! from the bot, `agent-exchange/outbox` for exports leaving the Mac); the
//! process working directory is never consulted. Reports submitted through
//! this facade always carry the hard-coded provider slug — by design there is
//! no flag that can set or override it.

use crate::{
    agent_gateway::AgentGateway,
    local_state::{LocalWorkspaceStore, ReadyActionRequest},
    repository::LocalRepository,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// The immutable provider slug recorded on every report submitted through
/// this facade. Hard-coded on the Mac side as a provenance guarantee: no CLI
/// flag exists that can set or override it.
pub(crate) const AGENT_CHANNEL_PROVIDER: &str = "agent:grok-bot";

/// Matches the MCP message cap and desktop IPC frame limit.
const MAX_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;

/// Outbox exports the bot never collected are removed after a week.
const OUTBOX_RETENTION: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

const USAGE: &str = "usage: codecaddie-core agent <status|goals|backlog|begin-analysis|submit-analysis|map|submit-map|note-action|export> [--flag value ...]";

/// Entry point for `codecaddie-core agent …`. Prints the single JSON result
/// object on stdout and returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let outcome = LocalWorkspaceStore::from_environment()
        .map_err(CliError::internal)
        .and_then(|store| execute(store, args));
    match outcome {
        Ok(result) => {
            println!("{}", ok_object(result));
            0
        }
        Err(error) => {
            println!("{}", error.to_json());
            1
        }
    }
}

#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) remediation: Vec<String>,
}

impl CliError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            remediation: vec![],
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    fn internal(error: anyhow::Error) -> Self {
        Self::new("operation_failed", format!("{error:#}"))
    }

    fn remediation(mut self, steps: Vec<String>) -> Self {
        self.remediation = steps;
        self
    }

    fn to_json(&self) -> Value {
        json!({
            "ok": false,
            "error": self.code,
            "message": self.message,
            "remediation": self.remediation,
        })
    }
}

impl From<anyhow::Error> for CliError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(error)
    }
}

fn ok_object(result: Value) -> Value {
    let mut object = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    object.insert("ok".into(), Value::Bool(true));
    Value::Object(object)
}

pub(crate) fn execute(store: LocalWorkspaceStore, args: &[String]) -> Result<Value, CliError> {
    let exchange = ExchangeDirs::prepare(store.data_root())?;
    let mut tokens = args.iter().map(String::as_str);
    let Some(verb) = tokens.next() else {
        return Err(CliError::invalid(USAGE));
    };
    let mut flags = parse_flags(tokens)?;
    match verb {
        "status" => {
            let workspace = take(&mut flags, "workspace");
            finish(flags)?;
            let gateway = AgentGateway::new(store, workspace);
            let mut status = gateway.workspace_status()?;
            status["exchange"] = json!({
                "inbox": exchange.inbox.display().to_string(),
                "outbox": exchange.outbox.display().to_string(),
            });
            Ok(status)
        }
        "goals" => {
            let workspace = take(&mut flags, "workspace");
            finish(flags)?;
            AgentGateway::new(store, workspace)
                .approved_goals()
                .map_err(Into::into)
        }
        "backlog" => {
            let workspace = take(&mut flags, "workspace");
            finish(flags)?;
            AgentGateway::new(store, workspace)
                .action_backlog()
                .map_err(Into::into)
        }
        "begin-analysis" => begin_analysis(store, flags),
        "submit-analysis" => submit_analysis(store, flags, &exchange),
        "map" => get_codebase_map(store, flags),
        "submit-map" => submit_codebase_map(store, flags, &exchange),
        "note-action" => note_action(store, flags, &exchange),
        "export" => export(store, flags, &exchange),
        other => Err(CliError::invalid(format!(
            "unknown verb {other:?}; {USAGE}"
        ))),
    }
}

fn begin_analysis(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
) -> Result<Value, CliError> {
    let repo = require(&mut flags, "repo")?;
    let fetch_ref = take(&mut flags, "fetch-ref");
    let workspace = take(&mut flags, "workspace");
    finish(flags)?;
    let (repository_id, pinned) = match repo.split_once('@') {
        Some((id, sha)) => (id.to_string(), Some(sha.to_string())),
        None => (repo, None),
    };
    if repository_id.is_empty() {
        return Err(CliError::invalid(
            "--repo requires <repositoryId> or <repositoryId>@<commitSha>",
        ));
    }
    if let Some(sha) = &pinned
        && (sha.len() < 4 || sha.len() > 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(CliError::invalid(
            "--repo pins commits as <repositoryId>@<hexCommitSha>",
        ));
    }
    if fetch_ref.is_some() {
        return Err(CliError::new(
            "read_only_repository",
            "begin-analysis cannot fetch into the registered checkout",
        )
        .remediation(vec![
            "fetch the commit outside CodeCaddie, then retry with its commit SHA".into(),
        ]));
    }
    let gateway = AgentGateway::new(store, workspace);
    let workspace_id = gateway.resolve_workspace()?;
    let repository = resolve_registered_repository(&gateway, &workspace_id, &repository_id)?;
    let commit = match pinned {
        None => repository
            .resolve_commit("HEAD")
            .map_err(CliError::internal)?,
        Some(sha) => resolve_pinned_commit(&repository, &sha)?,
    };
    gateway
        .open_session(workspace_id, vec![(repository, commit)])
        .map_err(Into::into)
}

fn submit_analysis(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
    exchange: &ExchangeDirs,
) -> Result<Value, CliError> {
    let session = require(&mut flags, "session")?;
    let file = require(&mut flags, "file")?;
    finish(flags)?;
    let payload_path = exchange.confine_inbox(&file)?;
    let gateway = AgentGateway::new(store, None);
    // An unknown or expired session leaves the payload in place: the bot can
    // begin a fresh session and resubmit without re-transferring the file.
    gateway.verify_session(&session).map_err(|error| {
        CliError::new("unknown_session", format!("{error:#}")).remediation(vec![
            "call begin-analysis to open a fresh session, then resubmit".into(),
        ])
    })?;
    let outcome = read_payload(&payload_path).and_then(|analysis| {
        gateway
            .submit_analysis(&session, analysis, AGENT_CHANNEL_PROVIDER.to_string())
            .map_err(|error| CliError::new("analysis_rejected", format!("{error:#}")))
    });
    // Processed payloads never linger in the inbox, whether the submission
    // was recorded or rejected by validation.
    let _ = fs::remove_file(&payload_path);
    outcome
}

fn get_codebase_map(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
) -> Result<Value, CliError> {
    let session = take(&mut flags, "session");
    let workspace = take(&mut flags, "workspace");
    finish(flags)?;
    AgentGateway::new(store, workspace)
        .get_codebase_map(session.as_deref())
        .map_err(Into::into)
}

fn submit_codebase_map(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
    exchange: &ExchangeDirs,
) -> Result<Value, CliError> {
    let session = require(&mut flags, "session")?;
    let file = require(&mut flags, "file")?;
    finish(flags)?;
    let payload_path = exchange.confine_inbox(&file)?;
    let gateway = AgentGateway::new(store, None);
    gateway.verify_session(&session).map_err(|error| {
        CliError::new("unknown_session", format!("{error:#}")).remediation(vec![
            "call begin-analysis to open a fresh session, then resubmit".into(),
        ])
    })?;
    let outcome = read_payload(&payload_path).and_then(|map| {
        gateway
            .submit_codebase_map(&session, map, AGENT_CHANNEL_PROVIDER.to_string())
            .map_err(|error| CliError::new("map_rejected", format!("{error:#}")))
    });
    let _ = fs::remove_file(&payload_path);
    outcome
}

fn note_action(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
    exchange: &ExchangeDirs,
) -> Result<Value, CliError> {
    let file = require(&mut flags, "file")?;
    finish(flags)?;
    let payload_path = exchange.confine_inbox(&file)?;
    let outcome = read_payload(&payload_path)
        .and_then(|payload| {
            serde_json::from_value::<ReadyActionRequest>(payload).map_err(|error| {
                CliError::new(
                    "payload_invalid",
                    format!("payload must be {{recommendationId, title, note}}: {error}"),
                )
            })
        })
        .and_then(|request| {
            AgentGateway::new(store, None)
                .record_action_note(request)
                .map_err(|error| CliError::new("action_rejected", format!("{error:#}")))
        });
    let _ = fs::remove_file(&payload_path);
    outcome
}

fn export(
    store: LocalWorkspaceStore,
    mut flags: BTreeMap<String, String>,
    exchange: &ExchangeDirs,
) -> Result<Value, CliError> {
    let kind = require(&mut flags, "kind")?;
    let out = require(&mut flags, "out")?;
    finish(flags)?;
    if kind != "word" {
        return Err(CliError::invalid(format!(
            "unknown export kind {kind:?}; expected word"
        )));
    }
    let destination = exchange.confine_outbox(&out)?;
    let recent = store
        .recent_workspace()
        .map_err(CliError::internal)?
        .ok_or_else(|| {
            CliError::new(
                "workspace_unavailable",
                "no CodeCaddie workspace exists on this device",
            )
            .remediation(vec![
                "create a workspace in the CodeCaddie app first".into(),
            ])
        })?;
    store
        .export_word_report(&recent.workspace_id, &destination)
        .map_err(|error| CliError::new("export_failed", format!("{error:#}")))?;
    Ok(json!({
        "kind": kind,
        "path": destination.display().to_string(),
    }))
}

fn resolve_registered_repository(
    gateway: &AgentGateway,
    workspace_id: &str,
    repository_id: &str,
) -> Result<LocalRepository, CliError> {
    let (access, projection) = gateway
        .store()
        .workspace_parts(workspace_id)
        .map_err(CliError::internal)?;
    if !projection.repositories.contains_key(repository_id) {
        let registered: Vec<&str> = projection.repositories.keys().map(String::as_str).collect();
        return Err(CliError::new(
            "unknown_repository",
            format!(
                "repositoryId {repository_id:?} is not registered in this workspace; registered: {registered:?}"
            ),
        ));
    }
    let checkout = access.repository_path.trim().to_string();
    if checkout.is_empty() {
        return Err(CliError::new(
            "repository_unavailable",
            "this device records no local checkout for the workspace repository",
        )
        .remediation(vec![
            "set the repository path in the CodeCaddie app on this device".into(),
        ]));
    }
    LocalRepository::attach(repository_id, &checkout).map_err(|error| {
        CliError::new(
            "repository_unavailable",
            format!("the registered checkout at {checkout:?} is not usable: {error:#}"),
        )
    })
}

fn resolve_pinned_commit(repository: &LocalRepository, sha: &str) -> Result<String, CliError> {
    if let Ok(commit) = repository.resolve_commit(sha) {
        return Ok(commit);
    }
    Err(CliError::new(
        "commit_not_found",
        format!("commit {sha} was not found in the registered checkout"),
    )
    .remediation(vec![
        "fetch the commit outside CodeCaddie, then retry with its commit SHA".into(),
        "or attach a separate checkout that already contains the commit".into(),
    ]))
}

fn read_payload(path: &Path) -> Result<Value, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::new(
            "file_unavailable",
            format!("could not read {}: {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_PAYLOAD_BYTES {
        return Err(CliError::new(
            "payload_too_large",
            format!("payload is {} bytes; the limit is 16 MiB", metadata.len()),
        )
        .remediation(vec![
            "trim rationales and other free text, then transfer a smaller payload".into(),
        ]));
    }
    let text = fs::read_to_string(path).map_err(|error| {
        CliError::new(
            "file_unavailable",
            format!("could not read {}: {error}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|error| {
        CliError::new(
            "payload_invalid",
            format!("payload is not valid JSON: {error}"),
        )
    })
}

/// The owner-only file-exchange directories. Both sides are created on
/// demand; the canonicalized paths are the confinement boundary every
/// `--file` and `--out` argument is checked against.
struct ExchangeDirs {
    inbox: PathBuf,
    outbox: PathBuf,
}

impl ExchangeDirs {
    fn prepare(data_root: &Path) -> Result<Self, CliError> {
        let root = data_root.join("agent-exchange");
        let inbox = root.join("inbox");
        let outbox = root.join("outbox");
        for directory in [&root, &inbox, &outbox] {
            fs::create_dir_all(directory).map_err(|error| {
                CliError::new(
                    "exchange_unavailable",
                    format!("could not create {}: {error}", directory.display()),
                )
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
            }
        }
        let canonicalized = |path: &Path| {
            path.canonicalize().map_err(|error| {
                CliError::new(
                    "exchange_unavailable",
                    format!("could not resolve {}: {error}", path.display()),
                )
            })
        };
        let inbox = canonicalized(&inbox)?;
        let outbox = canonicalized(&outbox)?;
        Self::collect_outbox_garbage(&outbox);
        Ok(Self { inbox, outbox })
    }

    /// Removes outbox files the bot never collected within the retention
    /// window. Runs on every invocation.
    fn collect_outbox_garbage(outbox: &Path) {
        let Ok(entries) = fs::read_dir(outbox) else {
            return;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .map(|age| age > OUTBOX_RETENTION)
                .unwrap_or(false);
            if stale {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// A `--file` argument must be absolute and canonicalize to a regular
    /// file inside the inbox; symlinks pointing elsewhere resolve outside it
    /// and are rejected.
    fn confine_inbox(&self, raw: &str) -> Result<PathBuf, CliError> {
        let requested = Path::new(raw);
        if !requested.is_absolute() {
            return Err(CliError::invalid(
                "--file must be an absolute path inside the agent inbox",
            ));
        }
        let canonical = requested.canonicalize().map_err(|error| {
            CliError::new(
                "file_unavailable",
                format!("could not open {raw:?}: {error}"),
            )
        })?;
        if canonical == self.inbox || !canonical.starts_with(&self.inbox) {
            return Err(CliError::new(
                "file_not_in_inbox",
                format!("{raw:?} resolves outside the agent inbox"),
            )
            .remediation(vec![format!(
                "transfer the file into {} and retry",
                self.inbox.display()
            )]));
        }
        if !canonical.is_file() {
            return Err(CliError::new(
                "file_unavailable",
                format!("{raw:?} is not a regular file"),
            ));
        }
        Ok(canonical)
    }

    /// An `--out` argument must be absolute, its canonicalized parent must
    /// sit inside the outbox, and the target itself may not be a symlink.
    fn confine_outbox(&self, raw: &str) -> Result<PathBuf, CliError> {
        let requested = Path::new(raw);
        if !requested.is_absolute() {
            return Err(CliError::invalid(
                "--out must be an absolute path inside the agent outbox",
            ));
        }
        let Some(file_name) = requested.file_name() else {
            return Err(CliError::invalid("--out must name a file, not a directory"));
        };
        let parent = requested
            .parent()
            .ok_or_else(|| CliError::invalid("--out must name a file inside the agent outbox"))?;
        let parent = parent.canonicalize().map_err(|error| {
            CliError::new(
                "output_not_in_outbox",
                format!("could not resolve the parent of {raw:?}: {error}"),
            )
            .remediation(vec![format!(
                "write exports under {}",
                self.outbox.display()
            )])
        })?;
        if !parent.starts_with(&self.outbox) {
            return Err(CliError::new(
                "output_not_in_outbox",
                format!("{raw:?} resolves outside the agent outbox"),
            )
            .remediation(vec![format!(
                "write exports under {}",
                self.outbox.display()
            )]));
        }
        let destination = parent.join(file_name);
        if let Ok(metadata) = fs::symlink_metadata(&destination)
            && metadata.file_type().is_symlink()
        {
            return Err(CliError::new(
                "output_not_in_outbox",
                format!("{raw:?} is a symlink; refusing to write through it"),
            ));
        }
        Ok(destination)
    }
}

fn parse_flags<'a>(
    mut tokens: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, String>, CliError> {
    let mut flags = BTreeMap::new();
    while let Some(token) = tokens.next() {
        let Some(name) = token.strip_prefix("--") else {
            return Err(CliError::invalid(format!("unexpected argument {token:?}")));
        };
        let Some(value) = tokens.next() else {
            return Err(CliError::invalid(format!("--{name} requires a value")));
        };
        if flags.insert(name.to_string(), value.to_string()).is_some() {
            return Err(CliError::invalid(format!("--{name} was given twice")));
        }
    }
    Ok(flags)
}

fn take(flags: &mut BTreeMap<String, String>, name: &str) -> Option<String> {
    flags.remove(name)
}

fn require(flags: &mut BTreeMap<String, String>, name: &str) -> Result<String, CliError> {
    flags
        .remove(name)
        .ok_or_else(|| CliError::invalid(format!("--{name} is required")))
}

fn finish(flags: BTreeMap<String, String>) -> Result<(), CliError> {
    match flags.into_keys().next() {
        None => Ok(()),
        Some(name) => Err(CliError::invalid(format!(
            "--{name} is not a flag of this verb"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_state::{ApproveGoalRequest, CreateWorkspaceRequest};
    use codecaddie_domain::{GoalVersion, ReportOrigin};
    use std::process::Command;
    use tempfile::TempDir;

    fn git_fixture() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        for arguments in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@codecaddie.local"],
            vec!["config", "user.name", "CodeCaddie Test"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(&arguments)
                .status()
                .unwrap();
        }
        fs::write(
            directory.path().join("tenant.rs"),
            "fn invoice(tenant: Id) {\n    scoped(tenant);\n}\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["commit", "--quiet", "-m", "fixture"])
            .status()
            .unwrap();
        directory
    }

    fn store_at(root: &Path) -> LocalWorkspaceStore {
        LocalWorkspaceStore::new(root.to_path_buf()).unwrap()
    }

    fn workspace_with_goal(data: &Path, repository: &Path) -> (String, GoalVersion) {
        let store = store_at(data);
        let workspace = store
            .create_workspace(CreateWorkspaceRequest {
                name: "Acme".into(),
                repository_display_name: "acme-app".into(),
                repository_path: repository.display().to_string(),
                product_brief: "Acme promises tenant-isolated invoicing.".into(),
                context: Default::default(),
            })
            .unwrap();
        let goal = store
            .approve_goal(
                &workspace.workspace_id,
                ApproveGoalRequest {
                    goal_id: "primary-goal".into(),
                    title: "Tenant isolation".into(),
                    business_outcome: "Customers cannot cross boundaries".into(),
                    criteria: vec!["Every invoice read is tenant scoped".into()],
                    priority: 5,
                    position: 1,
                    rubric_dimensions: vec!["security".into()],
                },
            )
            .unwrap();
        (workspace.workspace_id, goal)
    }

    /// One CLI invocation over a fresh one-shot store handle, exactly as the
    /// `agent` process mode runs.
    fn run_cli(data: &Path, args: &[&str]) -> Result<Value, CliError> {
        let args: Vec<String> = args.iter().map(|value| value.to_string()).collect();
        execute(store_at(data), &args)
    }

    fn analysis_payload(goal: &GoalVersion, rationale: &str) -> Value {
        json!({
            "providerVersion": "grok-bot 1.0",
            "assessments": [{
                "goalVersionId": goal.id,
                "summary": "Tenant isolation holds at the frozen commit.",
                "criteria": [{
                    "criterionId": goal.criteria[0].id,
                    "verdict": "supported",
                    "rationale": rationale,
                    "confidence": 0.9,
                    "evidence": [{
                        "repositoryId": "attached-repository",
                        "path": "tenant.rs",
                        "startLine": 1,
                        "endLine": 2,
                        "kind": "implementation"
                    }]
                }]
            }],
            "architecture": [],
            "recommendations": []
        })
    }

    #[test]
    fn agent_cli_end_to_end_submits_records_and_prevents_replay() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        let (workspace_id, goal) = workspace_with_goal(data.path(), repository.path());

        // status reports attested mode and advertises the exchange dirs.
        let status = run_cli(data.path(), &["status"]).unwrap();
        assert_eq!(status["ok"], Value::Null); // execute returns the bare payload
        assert_eq!(status["attestedModeAvailable"], true);
        let inbox = PathBuf::from(status["exchange"]["inbox"].as_str().unwrap());
        let outbox = PathBuf::from(status["exchange"]["outbox"].as_str().unwrap());
        assert!(inbox.is_absolute() && inbox.is_dir());
        assert!(outbox.is_absolute() && outbox.is_dir());

        let goals = run_cli(data.path(), &["goals"]).unwrap();
        assert_eq!(goals["goals"][0]["id"], json!(goal.id));

        // Pin an explicit sha that exists.
        let head = LocalRepository::attach("attached-repository", repository.path())
            .unwrap()
            .head()
            .unwrap();
        let begun = run_cli(
            data.path(),
            &[
                "begin-analysis",
                "--repo",
                &format!("attached-repository@{head}"),
                "--workspace",
                &workspace_id,
            ],
        )
        .unwrap();
        let session = begun["analysisSessionId"].as_str().unwrap().to_string();
        assert_eq!(begun["workspaceId"], json!(workspace_id));
        assert_eq!(begun["repositories"][0]["commitSha"], json!(head));
        assert_eq!(begun["goalSetHash"].as_str().unwrap().len(), 64);
        let session_path = data
            .path()
            .join("agent-sessions")
            .join(format!("{session}.json"));
        assert!(session_path.exists());

        // Submit a valid payload placed in the real inbox.
        let payload_path = inbox.join("analysis.json");
        fs::write(
            &payload_path,
            analysis_payload(&goal, "Invoice reads are tenant scoped.").to_string(),
        )
        .unwrap();
        let recorded = run_cli(
            data.path(),
            &[
                "submit-analysis",
                "--session",
                &session,
                "--file",
                payload_path.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(recorded["recorded"], true);
        assert_eq!(recorded["coverage"], 1.0);
        assert_eq!(recorded["goalVerdicts"][0]["verdict"], "supported");

        // The report is in the ledger with agent provenance and the
        // hard-coded provider slug.
        let report = store_at(data.path())
            .recent_workspace()
            .unwrap()
            .unwrap()
            .latest_report
            .unwrap();
        assert_eq!(report.origin, ReportOrigin::AgentSession);
        assert_eq!(report.provider, AGENT_CHANNEL_PROVIDER);

        // Session and payload files were consumed.
        assert!(!session_path.exists());
        assert!(!payload_path.exists());

        // Replaying the consumed session fails and leaves the new payload
        // for a future session.
        fs::write(
            &payload_path,
            analysis_payload(&goal, "Second try.").to_string(),
        )
        .unwrap();
        let replay = run_cli(
            data.path(),
            &[
                "submit-analysis",
                "--session",
                &session,
                "--file",
                payload_path.to_str().unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(replay.code, "unknown_session");
        assert!(
            replay
                .message
                .contains("unknown or expired analysis session")
        );
        assert!(payload_path.exists());

        // note-action consumes its payload and readies the action.
        let note_path = inbox.join("note.json");
        fs::write(
            &note_path,
            json!({
                "recommendationId": "rec-1",
                "title": "Harden invoices",
                "note": "Scoped all reads."
            })
            .to_string(),
        )
        .unwrap();
        let noted = run_cli(
            data.path(),
            &["note-action", "--file", note_path.to_str().unwrap()],
        )
        .unwrap();
        assert_eq!(noted["action"]["status"], "ready_for_verification");
        assert!(!note_path.exists());
    }

    #[test]
    fn begin_analysis_reports_missing_commits_with_a_machine_code() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        workspace_with_goal(data.path(), repository.path());
        let missing = "deadbeef".repeat(5);
        let error = run_cli(
            data.path(),
            &[
                "begin-analysis",
                "--repo",
                &format!("attached-repository@{missing}"),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "commit_not_found");
        assert_eq!(error.remediation.len(), 2);
        // No session file was left behind.
        let sessions = data.path().join("agent-sessions");
        assert_eq!(
            fs::read_dir(&sessions).map(|dir| dir.count()).unwrap_or(0),
            0
        );

        // A non-hex pin is rejected before touching git.
        let error = run_cli(
            data.path(),
            &["begin-analysis", "--repo", "attached-repository@--exec=x"],
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
    }

    #[test]
    fn agent_git_inputs_never_modify_the_registered_checkout() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        workspace_with_goal(data.path(), repository.path());
        let before = std::process::Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["show-ref", "--head"])
            .output()
            .unwrap()
            .stdout;

        let error = run_cli(
            data.path(),
            &[
                "begin-analysis",
                "--repo",
                "attached-repository@deadbeef",
                "--fetch-ref",
                "agent-branch",
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "read_only_repository");

        // The removed import-bundle verb stays unknown and leaves inputs alone.
        let status = run_cli(data.path(), &["status"]).unwrap();
        let inbox = PathBuf::from(status["exchange"]["inbox"].as_str().unwrap());
        let bundle = inbox.join("work.bundle");
        fs::write(&bundle, vec![0_u8; 1024]).unwrap();
        let error = run_cli(
            data.path(),
            &[
                "import-bundle",
                "--repo",
                "attached-repository",
                "--file",
                bundle.to_str().unwrap(),
                "--ref",
                "agent-branch",
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
        assert!(bundle.exists());

        let after = std::process::Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["show-ref", "--head"])
            .output()
            .unwrap()
            .stdout;
        assert_eq!(after, before);
    }

    #[test]
    fn payload_paths_are_confined_to_the_exchange_directories() {
        let data = tempfile::tempdir().unwrap();

        // An absolute path outside the inbox is rejected and never deleted.
        let outside = data.path().join("outside.json");
        fs::write(&outside, "{}").unwrap();
        let error = run_cli(
            data.path(),
            &[
                "submit-analysis",
                "--session",
                "analysis-x",
                "--file",
                outside.to_str().unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "file_not_in_inbox");
        assert!(outside.exists());

        // Relative paths are rejected outright.
        let error = run_cli(
            data.path(),
            &["note-action", "--file", "inbox/payload.json"],
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_arguments");

        // A symlink inside the inbox pointing outside is rejected.
        #[cfg(unix)]
        {
            let inbox = data.path().join("agent-exchange").join("inbox");
            let link = inbox.join("sneaky.json");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let error = run_cli(
                data.path(),
                &["note-action", "--file", link.to_str().unwrap()],
            )
            .unwrap_err();
            assert_eq!(error.code, "file_not_in_inbox");
            assert!(outside.exists());
        }

        // Exports must land inside the outbox: a foreign directory and a
        // dot-dot escape are both rejected before any workspace is touched.
        let stray = data.path().join("packet.docx");
        let error = run_cli(
            data.path(),
            &["export", "--kind", "word", "--out", stray.to_str().unwrap()],
        )
        .unwrap_err();
        assert_eq!(error.code, "output_not_in_outbox");
        let escape = data
            .path()
            .join("agent-exchange")
            .join("outbox")
            .join("..")
            .join("escape.docx");
        let error = run_cli(
            data.path(),
            &[
                "export",
                "--kind",
                "word",
                "--out",
                escape.to_str().unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "output_not_in_outbox");
    }

    #[test]
    fn expired_sessions_are_rejected_and_collected() {
        let data = tempfile::tempdir().unwrap();
        let repository = git_fixture();
        let (_, goal) = workspace_with_goal(data.path(), repository.path());

        // Without @sha the CLI pins HEAD exactly like the MCP tool.
        let begun = run_cli(
            data.path(),
            &["begin-analysis", "--repo", "attached-repository"],
        )
        .unwrap();
        let head = LocalRepository::attach("attached-repository", repository.path())
            .unwrap()
            .head()
            .unwrap();
        assert_eq!(begun["repositories"][0]["commitSha"], json!(head));
        let session = begun["analysisSessionId"].as_str().unwrap().to_string();
        let session_path = data
            .path()
            .join("agent-sessions")
            .join(format!("{session}.json"));

        // Age the session past its TTL.
        let encrypted_session = fs::read(&session_path).unwrap();
        assert!(!String::from_utf8_lossy(&encrypted_session).contains(&head));
        let decoded = crate::at_rest::ContentCipher::for_tests()
            .open_or_plain("agent-session-v1", &encrypted_session)
            .unwrap();
        assert!(decoded.encrypted);
        let mut stored: Value = serde_json::from_slice(&decoded.plaintext).unwrap();
        stored["expiresAt"] = json!("2000-01-01T00:00:00Z");
        fs::write(&session_path, stored.to_string()).unwrap();

        let inbox = data.path().join("agent-exchange").join("inbox");
        let payload_path = inbox.join("late.json");
        fs::write(
            &payload_path,
            analysis_payload(&goal, "Too late.").to_string(),
        )
        .unwrap();
        let error = run_cli(
            data.path(),
            &[
                "submit-analysis",
                "--session",
                &session,
                "--file",
                payload_path.to_str().unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "unknown_session");
        assert!(
            error
                .message
                .contains("unknown or expired analysis session")
        );
        // The expired session file was garbage collected.
        assert!(!session_path.exists());
    }
}
