use std::{ffi::OsString, fs, path::Path};

use codecaddie_core::{
    protocol::{CoreRequest, PROTOCOL_VERSION},
    service,
    update::{self, UpdaterResultCode},
};

const REPOSITORY_SOURCE_FIXTURE: &str = include_str!("fixtures/adversarial/repository_payload.rs");
const ATTACHMENT_SOURCE_FIXTURE: &str = include_str!("fixtures/adversarial/attachment_payload.md");
const SOURCE_CANARY: &str = "REPOSITORY_PRIVATE_SENTINEL_7DB9562A";
const ATTACHMENT_CANARY: &str = "ATTACHMENT_PRIVATE_SENTINEL_4F128CDE";
const SECRET_CANARY: &str = "SECRET_PRIVATE_SENTINEL_SK_LOCAL_8C31A6E2";

struct DataRootOverride(Option<OsString>);

impl DataRootOverride {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("CODECADDIE_DATA_DIR");
        // This integration test runs in its own process and restores the prior
        // value on drop, so no parallel in-process caller can observe it.
        unsafe { std::env::set_var("CODECADDIE_DATA_DIR", path) };
        Self(previous)
    }
}

impl Drop for DataRootOverride {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            unsafe { std::env::set_var("CODECADDIE_DATA_DIR", previous) };
        } else {
            unsafe { std::env::remove_var("CODECADDIE_DATA_DIR") };
        }
    }
}

fn ping(id: &str, params: serde_json::Value) -> CoreRequest {
    CoreRequest {
        id: id.into(),
        protocol_version: PROTOCOL_VERSION,
        workspace_id: None,
        method: "system.ping".into(),
        params: params
            .as_object()
            .cloned()
            .expect("ping params are an object"),
    }
}

async fn assert_malicious_mailbox_is_redacted(mailbox: &Path, id: &str, payload: &str) {
    fs::write(mailbox, payload).unwrap();
    let response = service::handle(ping(
        id,
        serde_json::json!({ "consumeUpdaterResult": true }),
    ))
    .await;
    assert!(response.ok);
    assert_eq!(
        response.result.as_ref().unwrap()["updaterResult"],
        serde_json::json!({
            "schemaVersion": 1,
            "status": "failed",
            "code": "resultUnreadable"
        })
    );
    let wire = serde_json::to_string(&response).unwrap();
    assert!(!wire.contains(SOURCE_CANARY));
    assert!(!wire.contains(ATTACHMENT_CANARY));
    assert!(!wire.contains(SECRET_CANARY));
    assert!(
        !mailbox.exists(),
        "an invalid mailbox must be consumed once"
    );
}

#[tokio::test]
async fn privacy_adversarial_updater_mailbox_and_startup_ipc_are_content_free() {
    assert!(REPOSITORY_SOURCE_FIXTURE.contains(SOURCE_CANARY));
    assert!(ATTACHMENT_SOURCE_FIXTURE.contains(ATTACHMENT_CANARY));
    let directory = tempfile::tempdir().unwrap();
    let _override = DataRootOverride::set(directory.path());
    let mailbox = directory
        .path()
        .join("updates")
        .join("last-updater-result-v1.json");
    fs::create_dir_all(mailbox.parent().unwrap()).unwrap();

    // The escaped key becomes the unknown `message` field after JSON parsing.
    // Both it and the secret alias must be rejected before either value can
    // cross the updater-result IPC boundary.
    let source = serde_json::to_string(&format!(
        "{REPOSITORY_SOURCE_FIXTURE}\n{ATTACHMENT_SOURCE_FIXTURE}"
    ))
    .unwrap();
    let secret = serde_json::to_string(SECRET_CANARY).unwrap();
    let combined = serde_json::to_string(&format!("{SOURCE_CANARY}:{SECRET_CANARY}")).unwrap();
    let malicious_payloads = [
        format!(
            r#"{{"schemaVersion":1,"status":"failed","code":"installFailed","\u006dessage":{source},"token":{secret}}}"#,
        ),
        format!(r#"{{"schemaVersion":1,"status":"failed","code":{combined}}}"#),
        format!(
            r#"{{"schemaVersion":1,"status":{{"source":{source},"secret":{secret}}},"code":"installFailed"}}"#,
        ),
        format!(
            r#"{{"schemaVersion":1,"status":"failed","code":"installFailed","code":{combined}}}"#,
        ),
        format!(
            r#"{{"schemaVersion":1,"schema_version":{combined},"status":"failed","code":"installFailed"}}"#,
        ),
    ];
    for (index, malicious) in malicious_payloads.iter().enumerate() {
        assert_malicious_mailbox_is_redacted(
            &mailbox,
            &format!("adversarial-startup-{index}"),
            malicious,
        )
        .await;
    }

    // A valid fixed result remains available across ordinary and false/string
    // opt-out pings, then appears once only on the exact boolean startup opt-in.
    update::record_updater_result(UpdaterResultCode::InstallFailed).unwrap();
    for (id, params) in [
        ("ordinary-ping", serde_json::json!({})),
        (
            "false-opt-out",
            serde_json::json!({ "consumeUpdaterResult": false }),
        ),
        (
            "string-opt-out",
            serde_json::json!({ "consumeUpdaterResult": "true" }),
        ),
    ] {
        let response = service::handle(ping(id, params)).await;
        assert!(response.ok);
        let result = response.result.unwrap();
        let mut fields: Vec<&str> = result
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["build", "protocolVersion", "service"]);
        assert!(
            result.get("updaterResult").is_none(),
            "non-startup pings retain the historical response shape"
        );
        assert!(
            mailbox.is_file(),
            "an opt-out ping must not consume the mailbox"
        );
    }

    let consumed = service::handle(ping(
        "valid-startup",
        serde_json::json!({ "consumeUpdaterResult": true }),
    ))
    .await;
    assert_eq!(
        consumed.result.unwrap()["updaterResult"],
        serde_json::json!({
            "schemaVersion": 1,
            "status": "failed",
            "code": "installFailed"
        })
    );
    assert!(!mailbox.exists());

    let empty = service::handle(ping(
        "empty-startup",
        serde_json::json!({ "consumeUpdaterResult": true }),
    ))
    .await;
    assert!(empty.result.unwrap()["updaterResult"].is_null());
}
