//! Root-bound job references: the consumer half of the reference contract.
//!
//! A successful launch response already names both halves of a job's identity —
//! its ID and the absolute jobs root that owns it. This module turns such a
//! response, saved verbatim, back into a selector that binds to exactly one job
//! directory regardless of the consumer's cwd, `AGENT_EXEC_ROOT`, or platform
//! defaults.
//!
//! Scope: this is a same-host, same-filesystem handoff, not a sandbox. The root
//! contents and symlink mappings a reference names are assumed trusted; nothing
//! here provides a cross-host or malicious-filesystem identity guarantee.
//! Fields other than the ones validated below are ignored and never executed.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

/// Maximum accepted reference document size.
///
/// A reference is a single launch response, so 16 MiB is orders of magnitude
/// above any legitimate input while keeping an unbounded pipe from being read
/// into memory.
pub const MAX_REFERENCE_BYTES: usize = 16 * 1024 * 1024;

/// Literal spelling of the stdin selector accepted by `--from-json`.
pub const STDIN_SELECTOR: &str = "-";

/// Sentinel error type for a rejected job reference.
///
/// Rendered as `invalid_argument` / `retryable=false`. The message never echoes
/// the supplied document: a reference can carry arbitrary caller content and
/// stdout is a machine-readable channel.
#[derive(Debug)]
pub struct InvalidJobReference(pub String);

impl std::fmt::Display for InvalidJobReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid job reference: {}", self.0)
    }
}

impl std::error::Error for InvalidJobReference {}

fn invalid(reason: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(InvalidJobReference(reason.into()))
}

/// A validated root-bound job reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobReference {
    /// Exact job directory name; never a prefix.
    pub job_id: String,
    /// Absolute jobs root that owns `job_id`, exactly as the producer reported it.
    pub root: PathBuf,
}

/// Read and validate a reference from `selector`, which is either `-` for stdin
/// or a filesystem path.
pub fn load(selector: &str) -> Result<JobReference> {
    let bytes = if selector == STDIN_SELECTOR {
        read_bounded(&mut std::io::stdin().lock(), "standard input")?
    } else {
        let mut file = std::fs::File::open(selector)
            .map_err(|e| invalid(format!("cannot read reference file: {e}")))?;
        read_bounded(&mut file, "reference file")?
    };
    parse(&bytes)
}

/// Read at most [`MAX_REFERENCE_BYTES`] plus one sentinel byte, so an oversized
/// document is detected without buffering it.
fn read_bounded(source: &mut impl std::io::Read, what: &str) -> Result<Vec<u8>> {
    use std::io::Read;

    let mut buffer = Vec::new();
    let read = source
        .take(MAX_REFERENCE_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|e| invalid(format!("cannot read {what}: {e}")))?;
    if read > MAX_REFERENCE_BYTES {
        return Err(invalid(format!(
            "reference exceeds the {MAX_REFERENCE_BYTES}-byte limit"
        )));
    }
    Ok(buffer)
}

/// Validate a reference document.
///
/// Accepts a successful launch response body: either the MCP `structuredContent`
/// object or the byte-identical text JSON body. Protocol wrappers (JSON-RPC
/// envelopes, `tools/call` results, `content` arrays) are not references and are
/// rejected, because they do not carry `ok`/`job_id`/`root` at the top level.
pub fn parse(bytes: &[u8]) -> Result<JobReference> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("reference is not valid UTF-8 JSON"))?;
    // Duplicate keys resolve last-wins, matching `serde_json`'s object model.
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| invalid(format!("reference is not JSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("reference must be a JSON object"))?;

    // `ok` is required, so a bare two-field `{job_id, root}` document — which
    // asserts nothing about the launch having succeeded — is not a reference.
    match object.get("ok") {
        Some(serde_json::Value::Bool(true)) => {}
        Some(serde_json::Value::Bool(false)) => {
            return Err(invalid("reference reports ok=false"));
        }
        Some(_) => return Err(invalid("reference field ok must be a boolean")),
        None => return Err(invalid("reference is missing the ok field")),
    }

    if let Some(version) = object.get("schema_version") {
        let version = version
            .as_str()
            .ok_or_else(|| invalid("reference field schema_version must be a string"))?;
        validate_schema_version(version)?;
    }

    let root = required_string(object, "root")?;
    let job_id = required_string(object, "job_id")?;

    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(invalid("reference field root must be an absolute path"));
    }
    validate_job_id(&job_id)?;

    Ok(JobReference { job_id, root })
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String> {
    let value = object
        .get(field)
        .ok_or_else(|| invalid(format!("reference is missing the {field} field")))?
        .as_str()
        .ok_or_else(|| invalid(format!("reference field {field} must be a string")))?;
    if value.is_empty() {
        return Err(invalid(format!(
            "reference field {field} must not be empty"
        )));
    }
    if value.contains('\0') {
        return Err(invalid(format!(
            "reference field {field} must not contain NUL"
        )));
    }
    Ok(value.to_string())
}

/// Accept only a decimal `MAJOR.MINOR` version inside the current major series.
fn validate_schema_version(version: &str) -> Result<()> {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    let well_formed = parts.next().is_none()
        && !major.is_empty()
        && !minor.is_empty()
        && major.bytes().all(|b| b.is_ascii_digit())
        && minor.bytes().all(|b| b.is_ascii_digit());
    if !well_formed {
        return Err(invalid(
            "reference field schema_version must be decimal MAJOR.MINOR",
        ));
    }
    if major.parse::<u64>().ok() != Some(0) {
        return Err(invalid(format!(
            "reference schema_version {version} is not compatible with major version 0"
        )));
    }
    Ok(())
}

/// A job ID must name one directory entry inside the referenced root and nothing
/// else, so it can never escape the root or select it.
///
/// `.` and `..` are rejected because they are not `Component::Normal`. Path
/// separators and `:` are rejected on every platform, not only where they are
/// native, so a reference validated on one host stays rejected on another. Both
/// legacy ULID IDs and current fixed-length hex IDs satisfy this.
fn validate_job_id(job_id: &str) -> Result<()> {
    if job_id.contains('/') || job_id.contains('\\') || job_id.contains(':') {
        return Err(invalid(
            "reference field job_id must not contain a path separator or colon",
        ));
    }
    let path = Path::new(job_id);
    let mut components = path.components();
    let only = match (components.next(), components.next()) {
        (Some(Component::Normal(first)), None) => first,
        _ => {
            return Err(invalid(
                "reference field job_id must be a single path component",
            ));
        }
    };
    if only != std::ffi::OsStr::new(job_id) {
        return Err(invalid(
            "reference field job_id must be a single path component",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(extra: &str) -> String {
        format!(
            r#"{{"schema_version":"0.4","ok":true,"type":"run","job_id":"abc123","root":"/jobs/root"{extra}}}"#
        )
    }

    fn reason(bytes: &[u8]) -> String {
        let err = parse(bytes).expect_err("reference must be rejected");
        err.downcast_ref::<InvalidJobReference>()
            .expect("rejection must be typed as InvalidJobReference")
            .0
            .clone()
    }

    #[test]
    fn root_reference_parses_a_launch_response_body() {
        let parsed = parse(body("").as_bytes()).expect("launch body is a reference");
        assert_eq!(parsed.job_id, "abc123");
        assert_eq!(parsed.root, PathBuf::from("/jobs/root"));
    }

    #[test]
    fn root_reference_ignores_unrelated_fields_without_executing_them() {
        let parsed = parse(body(r#","command":["rm","-rf","/"],"stdout":"x""#).as_bytes())
            .expect("unknown fields are ignored");
        assert_eq!(parsed.job_id, "abc123");
    }

    #[test]
    fn root_reference_accepts_a_document_without_schema_version() {
        let parsed = parse(br#"{"ok":true,"job_id":"abc","root":"/r"}"#)
            .expect("schema_version is optional");
        assert_eq!(parsed.root, PathBuf::from("/r"));
    }

    #[test]
    fn root_reference_takes_the_last_duplicate_key() {
        let parsed = parse(br#"{"ok":true,"job_id":"first","job_id":"second","root":"/r"}"#)
            .expect("duplicate keys are last-wins");
        assert_eq!(parsed.job_id, "second");
    }

    #[test]
    fn root_reference_rejects_documents_that_are_not_objects() {
        assert!(reason(b"[]").contains("must be a JSON object"));
        assert!(reason(b"\"abc\"").contains("must be a JSON object"));
    }

    #[test]
    fn root_reference_rejects_malformed_and_non_utf8_input() {
        assert!(reason(b"{").contains("not JSON"));
        assert!(reason(&[0xff, 0xfe]).contains("not valid UTF-8"));
    }

    #[test]
    fn root_reference_rejects_protocol_wrappers() {
        let structured = format!(r#"{{"structuredContent":{}}}"#, body(""));
        assert!(reason(structured.as_bytes()).contains("missing the ok field"));
        let rpc = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{}}}"#, body(""));
        assert!(reason(rpc.as_bytes()).contains("missing the ok field"));
        let content = format!(
            r#"{{"content":[{{"type":"text","text":{}}}]}}"#,
            serde_json::to_string(&body("")).expect("nested text")
        );
        assert!(reason(content.as_bytes()).contains("missing the ok field"));
    }

    #[test]
    fn root_reference_requires_a_successful_launch() {
        assert!(reason(br#"{"ok":false,"job_id":"a","root":"/r"}"#).contains("ok=false"));
        assert!(reason(br#"{"job_id":"a","root":"/r"}"#).contains("missing the ok field"));
        assert!(reason(br#"{"ok":"true","job_id":"a","root":"/r"}"#).contains("must be a boolean"));
    }

    #[test]
    fn root_reference_requires_an_absolute_nonempty_root() {
        assert!(reason(br#"{"ok":true,"job_id":"a"}"#).contains("missing the root field"));
        assert!(reason(br#"{"ok":true,"job_id":"a","root":""}"#).contains("must not be empty"));
        assert!(
            reason(br#"{"ok":true,"job_id":"a","root":"relative/jobs"}"#)
                .contains("must be an absolute path")
        );
        assert!(reason(br#"{"ok":true,"job_id":"a","root":5}"#).contains("must be a string"));
        assert!(
            reason("{\"ok\":true,\"job_id\":\"a\",\"root\":\"/r\\u0000x\"}".as_bytes())
                .contains("must not contain NUL")
        );
    }

    #[test]
    fn root_reference_rejects_unsafe_job_ids() {
        for id in ["", ".", "..", "a/b", "a\\b", "C:x", "/abs", "a:b"] {
            let document = format!(
                r#"{{"ok":true,"job_id":{},"root":"/r"}}"#,
                serde_json::to_string(id).expect("encode id")
            );
            parse(document.as_bytes()).expect_err(&format!("job_id {id:?} must be rejected"));
        }
        assert!(
            reason("{\"ok\":true,\"job_id\":\"a\\u0000b\",\"root\":\"/r\"}".as_bytes())
                .contains("must not contain NUL")
        );
    }

    #[test]
    fn root_reference_accepts_legacy_ulid_and_current_hex_ids() {
        for id in [
            "01HQ8Z0000000000000000000A",
            "0123456789abcdef0123456789abcdef",
        ] {
            let document = format!(r#"{{"ok":true,"job_id":"{id}","root":"/r"}}"#);
            let parsed = parse(document.as_bytes()).expect("supported ID shape");
            assert_eq!(parsed.job_id, id);
        }
    }

    #[test]
    fn root_reference_rejects_incompatible_schema_versions() {
        for version in ["1.0", "0", "0.4.1", "x.y", "0.", ".4", "00.4.0"] {
            let document =
                format!(r#"{{"ok":true,"schema_version":"{version}","job_id":"a","root":"/r"}}"#);
            parse(document.as_bytes())
                .expect_err(&format!("schema_version {version:?} must be rejected"));
        }
        assert!(
            reason(br#"{"ok":true,"schema_version":0.4,"job_id":"a","root":"/r"}"#)
                .contains("must be a string")
        );
        parse(br#"{"ok":true,"schema_version":"0.3","job_id":"a","root":"/r"}"#)
            .expect("earlier minor versions in major 0 stay compatible");
    }

    #[test]
    fn root_reference_rejects_oversized_documents() {
        let mut oversized = Vec::with_capacity(MAX_REFERENCE_BYTES + 8);
        oversized.extend_from_slice(br#"{"ok":true,"job_id":"a","root":"/r","pad":""#);
        oversized.resize(MAX_REFERENCE_BYTES + 1, b'x');
        oversized.extend_from_slice(br#""}"#);
        let err = read_bounded(&mut oversized.as_slice(), "reference file")
            .expect_err("oversized input must be rejected");
        assert!(
            err.downcast_ref::<InvalidJobReference>()
                .expect("typed rejection")
                .0
                .contains("exceeds"),
            "unexpected reason: {err}"
        );
    }

    #[test]
    fn root_reference_reads_a_document_at_the_size_limit() {
        let mut at_limit = Vec::new();
        at_limit.extend_from_slice(br#"{"ok":true,"job_id":"a","root":"/r","pad":""#);
        at_limit.resize(MAX_REFERENCE_BYTES - 2, b'x');
        at_limit.extend_from_slice(br#""}"#);
        assert_eq!(at_limit.len(), MAX_REFERENCE_BYTES);
        let bytes = read_bounded(&mut at_limit.as_slice(), "reference file")
            .expect("a document exactly at the limit is accepted");
        assert_eq!(parse(&bytes).expect("valid reference").job_id, "a");
    }

    #[test]
    fn root_reference_rejection_never_echoes_the_document() {
        let secret = r#"{"ok":true,"job_id":"a","root":"relative","secret":"TOPSECRET"}"#;
        let message = parse(secret.as_bytes())
            .expect_err("relative root is rejected")
            .to_string();
        assert!(
            !message.contains("TOPSECRET"),
            "rejection must not echo input: {message}"
        );
    }
}
