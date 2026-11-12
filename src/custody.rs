use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::acquisition::now_ms;
use crate::source::{invalid, open_image};
use crate::verify_file;

const MAX_EVENTS: u32 = 4096;
const ROOT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const DOMAIN: &[u8] = b"OmniRecovery custody event v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum CustodyAction {
    Acquired,
    Verified,
    Exported,
    Reviewed,
    RequestedWipe,
    ApprovedWipe,
    RequestedFirmware,
    ApprovedFirmware,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Event {
    sequence: u32,
    previous: String,
    time_unix_ms: u64,
    actor: String,
    case: String,
    action: CustodyAction,
    artifact_sha256: String,
    artifact_bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SignedEvent {
    event: Event,
    signature: String,
}

#[derive(Debug, Serialize)]
pub struct CustodyHead {
    pub events: u32,
    pub sha256: String,
}

fn read_limited(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_image(path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(invalid("custody file exceeds size limit"));
    }
    Ok(bytes)
}

fn check_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 80
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'@'))
}

fn serialize_event(event: &Event) -> io::Result<Vec<u8>> {
    let mut data = DOMAIN.to_vec();
    serde_json::to_writer(&mut data, event)?;
    Ok(data)
}

fn hash_event(event: &Event, signature: &Signature) -> io::Result<String> {
    let mut digest = Sha256::new();
    digest.update(serialize_event(event)?);
    digest.update(signature.to_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

fn event_path(directory: &Path, sequence: u32) -> std::path::PathBuf {
    directory.join(format!("{sequence:08}.json"))
}

fn trusted_keys(path: &Path) -> io::Result<BTreeMap<String, VerifyingKey>> {
    let configured: BTreeMap<String, String> =
        serde_json::from_slice(&read_limited(path, 64 * 1024)?)?;
    let mut fingerprints = std::collections::BTreeSet::new();
    let mut keys = BTreeMap::new();
    for (actor, public) in configured {
        let mut bytes = [0; 32];
        if !check_label(&actor)
            || hex::decode_to_slice(&public, &mut bytes).is_err()
            || !fingerprints.insert(bytes)
        {
            return Err(invalid(
                "invalid actor label, key or duplicate identity key",
            ));
        }
        let key =
            VerifyingKey::from_bytes(&bytes).map_err(|_| invalid("invalid Ed25519 public key"))?;
        if key.is_weak() {
            return Err(invalid("weak Ed25519 public key"));
        }
        keys.insert(actor, key);
    }
    if keys.is_empty() {
        return Err(invalid("custody requires a configured trust list"));
    }
    Ok(keys)
}

fn walk(
    directory: &Path,
    keys: &BTreeMap<String, VerifyingKey>,
    mut observe: impl FnMut(&Event),
) -> io::Result<CustodyHead> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(invalid("custody ledger must be a real directory"));
    }
    let mut indexes = Vec::new();
    for entry in fs::read_dir(directory)? {
        let name = entry?.file_name();
        if let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".json")) {
            let index = stem
                .parse::<u32>()
                .ok()
                .filter(|n| format!("{n:08}") == stem && (1..=MAX_EVENTS).contains(n))
                .ok_or_else(|| invalid("unexpected JSON file in custody ledger"))?;
            indexes.push(index);
        }
    }
    indexes.sort_unstable();
    let mut head = CustodyHead {
        events: 0,
        sha256: ROOT.into(),
    };
    for index in indexes {
        let signed: SignedEvent =
            serde_json::from_slice(&read_limited(&event_path(directory, index), 8192)?)?;
        let event = signed.event;
        if index != head.events + 1 || event.sequence != index || event.previous != head.sha256 {
            return Err(invalid("custody sequence or previous digest mismatch"));
        }
        let key = keys
            .get(&event.actor)
            .ok_or_else(|| invalid("custody event used an untrusted identity"))?;
        let mut sig = [0_u8; 64];
        hex::decode_to_slice(signed.signature, &mut sig)
            .map_err(|_| invalid("invalid custody signature encoding"))?;
        let signature = Signature::from_bytes(&sig);
        key.verify_strict(&serialize_event(&event)?, &signature)
            .map_err(|_| invalid("custody signature mismatch"))?;
        observe(&event);
        head.events = index;
        head.sha256 = hash_event(&event, &signature)?;
    }
    Ok(head)
}

/// Generate a named-operator signing seed and return the public key for a separate trust list.
pub fn generate_custody_key(path: &Path) -> io::Result<String> {
    let mut seed = Zeroizing::new([0_u8; 32]);
    getrandom::fill(&mut *seed)
        .map_err(|_| io::Error::other("could not generate an Ed25519 seed"))?;
    let key = SigningKey::from_bytes(&seed);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&*seed)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(hex::encode(key.verifying_key().as_bytes()))
}

/// Sign and append one event if the actor's configured public key matches the supplied seed.
pub fn append_custody(
    directory: &Path,
    trust: &Path,
    private: &Path,
    actor: &str,
    case: &str,
    action: CustodyAction,
    artifact: &Path,
) -> io::Result<CustodyHead> {
    if !check_label(actor) || !check_label(case) {
        return Err(invalid("invalid custody identity or case label"));
    }
    let keys = trusted_keys(trust)?;
    let mut head = walk(directory, &keys, |_| {})?;
    if head.events >= MAX_EVENTS {
        return Err(invalid("custody ledger capacity exceeded"));
    }
    let mut seed_file = open_image(private)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if seed_file.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(invalid(
                "custody private seed must be owner-accessible only",
            ));
        }
    }
    let mut seed = Zeroizing::new([0_u8; 32]);
    seed_file.read_exact(&mut *seed)?;
    if seed_file.read(&mut [0_u8; 1])? != 0 {
        return Err(invalid("invalid custody private seed"));
    }
    let key = SigningKey::from_bytes(&seed);
    if keys.get(actor) != Some(&key.verifying_key()) {
        return Err(invalid(
            "custody private seed does not match the trusted actor",
        ));
    }
    let record = verify_file(artifact)?;
    let event = Event {
        sequence: head.events + 1,
        previous: head.sha256,
        time_unix_ms: now_ms(),
        actor: actor.into(),
        case: case.into(),
        action,
        artifact_sha256: record.sha256,
        artifact_bytes: record.length,
    };
    let signature = key.sign(&serialize_event(&event)?);
    head.events = event.sequence;
    head.sha256 = hash_event(&event, &signature)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(
        &mut file,
        &SignedEvent {
            event,
            signature: hex::encode(signature.to_bytes()),
        },
    )?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(event_path(directory, head.events))
        .map_err(|error| error.error)?;
    Ok(head)
}

/// Verify all signatures and the hash chain against an independently retained checkpoint.
pub fn verify_custody(
    directory: &Path,
    trust: &Path,
    expected_head: &str,
) -> io::Result<CustodyHead> {
    let head = walk(directory, &trusted_keys(trust)?, |_| {})?;
    if !head.sha256.eq_ignore_ascii_case(expected_head) {
        return Err(invalid(
            "custody head does not match the retained checkpoint",
        ));
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_reject_tampering_and_checkpoint_rejects_truncation() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let key = tmp.path().join("owner.key");
        let trust = tmp.path().join("trust.json");
        let ledger = tmp.path().join("ledger");
        let artifact = tmp.path().join("manifest.json");
        fs::create_dir(&ledger)?;
        fs::write(&artifact, b"synthetic evidence")?;
        let public = generate_custody_key(&key)?;
        fs::write(
            &trust,
            serde_json::to_vec(&serde_json::json!({"examiner": public}))?,
        )?;
        let first = append_custody(
            &ledger,
            &trust,
            &key,
            "examiner",
            "case01",
            CustodyAction::Acquired,
            &artifact,
        )?;
        let second = append_custody(
            &ledger,
            &trust,
            &key,
            "examiner",
            "case01",
            CustodyAction::Verified,
            &artifact,
        )?;
        assert_eq!(verify_custody(&ledger, &trust, &second.sha256)?.events, 2);
        let event = event_path(&ledger, 2);
        let mut bytes: serde_json::Value = serde_json::from_slice(&fs::read(&event)?)?;
        bytes["event"]["case"] = serde_json::json!("case02");
        fs::write(&event, serde_json::to_vec(&bytes)?)?;
        assert!(verify_custody(&ledger, &trust, &second.sha256).is_err());
        fs::remove_file(event)?;
        assert!(verify_custody(&ledger, &trust, &second.sha256).is_err());
        assert_eq!(verify_custody(&ledger, &trust, &first.sha256)?.events, 1);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveAction {
    Wipe,
    Firmware,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionPlan {
    action: SensitiveAction,
    case: String,
    target_model: String,
    target_id: String,
    image_sha256: Option<String>,
    created_unix_ms: u64,
    expires_unix_ms: u64,
    warning: String,
}

#[derive(Debug, Serialize)]
pub struct ApprovedAction {
    pub action: SensitiveAction,
    pub plan_sha256: String,
    pub requester: String,
    pub approver: String,
    pub checkpoint: CustodyHead,
}

const APPROVAL_MS: u64 = 24 * 60 * 60 * 1000;

/// Write a reviewable operation plan. Firmware input requires a trusted vendor digest.
pub fn make_action_plan(
    action: SensitiveAction,
    case: &str,
    model: &str,
    target_id: &str,
    firmware: Option<(&Path, &str)>,
    destination: &Path,
) -> io::Result<()> {
    if ![case, model, target_id].into_iter().all(check_label) {
        return Err(invalid("invalid case, model or target identifier"));
    }
    let image_sha256 = match (action, firmware) {
        (SensitiveAction::Wipe, None) => None,
        (SensitiveAction::Firmware, Some((image, trusted))) => {
            let digest = verify_file(image)?.sha256;
            if !digest.eq_ignore_ascii_case(trusted) {
                return Err(invalid(
                    "firmware SHA-256 did not match the trusted release digest",
                ));
            }
            Some(digest)
        }
        _ => {
            return Err(invalid(
                "firmware plans require an image and trusted digest",
            ));
        }
    };
    let created_unix_ms = now_ms();
    let plan = ActionPlan {
        action, case: case.into(), target_model: model.into(), target_id: target_id.into(),
        image_sha256, created_unix_ms, expires_unix_ms: created_unix_ms + APPROVAL_MS,
        warning: "This operation can permanently destroy data. Verify the exact physical target, backup, vendor method and approval before execution.".into(),
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, &plan)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

/// Verify a request and approval signed by distinct trusted operators for the exact plan bytes.
pub fn verify_action_approval(
    directory: &Path,
    trust: &Path,
    expected_head: &str,
    plan: &Path,
) -> io::Result<ApprovedAction> {
    let bytes = read_limited(plan, 8192)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let plan: ActionPlan = serde_json::from_slice(&bytes)?;
    let now = now_ms();
    if !(plan.created_unix_ms..=plan.expires_unix_ms).contains(&now)
        || plan.expires_unix_ms.saturating_sub(plan.created_unix_ms) > APPROVAL_MS
        || ![&plan.case, &plan.target_model, &plan.target_id]
            .into_iter()
            .all(|label| check_label(label))
        || (plan.action == SensitiveAction::Firmware)
            != plan.image_sha256.as_ref().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        return Err(invalid(
            "operation plan expired or contains invalid target details",
        ));
    }
    let (request, approval) = match plan.action {
        SensitiveAction::Wipe => (CustodyAction::RequestedWipe, CustodyAction::ApprovedWipe),
        SensitiveAction::Firmware => (
            CustodyAction::RequestedFirmware,
            CustodyAction::ApprovedFirmware,
        ),
    };
    let mut requester = None;
    let mut pair = None;
    let checkpoint = walk(directory, &trusted_keys(trust)?, |event| {
        if event.case == plan.case
            && event.artifact_sha256 == sha256
            && (plan.created_unix_ms..=now).contains(&event.time_unix_ms)
        {
            if event.action == request {
                requester = Some(event.actor.clone());
                pair = None;
            } else if event.action == approval
                && let Some(actor) = requester.as_ref().filter(|actor| **actor != event.actor)
            {
                pair = Some((actor.clone(), event.actor.clone()));
            }
        }
    })?;
    if !checkpoint.sha256.eq_ignore_ascii_case(expected_head) {
        return Err(invalid("custody checkpoint did not match"));
    }
    let (requester, approver) = pair.ok_or_else(|| {
        invalid("operation requires a request and approval from two different trusted identities")
    })?;
    Ok(ApprovedAction {
        action: plan.action,
        plan_sha256: sha256,
        requester,
        approver,
        checkpoint,
    })
}
