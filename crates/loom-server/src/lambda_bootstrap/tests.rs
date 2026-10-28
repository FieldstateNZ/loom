use super::*;
use serde_json::{json, Value};
use std::{
    process::{Command, Stdio},
    sync::OnceLock,
};

fn environment() -> HashMap<&'static str, String> {
    HashMap::from([
        (
            "LOOM_BOOTSTRAP_PARAMETER_ARN",
            "arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/test/loom".into(),
        ),
        ("LOOM_BOOTSTRAP_PARAMETER_VERSION", "7".into()),
        ("AWS_REGION", "ap-southeast-2".into()),
        ("LOOM_LAMBDA_ACCOUNT_ID", "123456789012".into()),
        ("LOOM_REQUEST_DRIVEN", "true".into()),
        ("AWS_ACCESS_KEY_ID", "ASIASYNTHETICEXAMPLE".into()),
        ("AWS_SECRET_ACCESS_KEY", "synthetic-signing-key".into()),
        ("AWS_SESSION_TOKEN", "synthetic-session".into()),
    ])
}
fn reader(env: &HashMap<&'static str, String>, name: &'static str) -> Result<String, VarError> {
    env.get(name).cloned().ok_or(VarError::NotPresent)
}
fn ca() -> &'static str {
    static CA: OnceLock<String> = OnceLock::new();
    CA.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("loom-ca-test-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=Synthetic Loom CA",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-keyout",
            ])
            .arg(dir.join("ca.key"))
            .arg("-out")
            .arg(dir.join("ca.pem"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let pem = fs::read_to_string(dir.join("ca.pem")).unwrap();
        fs::remove_dir_all(dir).unwrap();
        pem
    })
}
fn document() -> Value {
    json!({"schema_version":1,"kind":"loom-private-lambda","database_url":"postgresql://runtime:synthetic-database-password@10.63.0.29/loom_test",
        "database_ca_pem":ca(),"database_ca_sha256":hex::encode(Sha256::digest(ca().as_bytes())),
        "root_admin_token":"a".repeat(64),"encryption_key":"02".repeat(32),"key_pepper":{"mode":"derived"}})
}
fn response() -> Value {
    json!({"Parameter":{"ARN":"arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/test/loom","Name":"/tandem/test/loom","Version":7,"Selector":":7","Type":"SecureString","Value":document().to_string()}})
}

#[test]
fn startup_reference_has_no_implicit_fallback_or_cross_account_region() {
    assert!(reference(&|_| Err(VarError::NotPresent)).unwrap().is_none());
    for (key, value) in [
        ("LOOM_BOOTSTRAP_PARAMETER_VERSION", "0"),
        ("LOOM_BOOTSTRAP_PARAMETER_VERSION", "07"),
        ("LOOM_BOOTSTRAP_PARAMETER_VERSION", "latest"),
        (
            "LOOM_BOOTSTRAP_PARAMETER_ARN",
            "https://attacker.invalid/parameter",
        ),
        ("AWS_REGION", "us-east-1"),
        ("LOOM_LAMBDA_ACCOUNT_ID", "999999999999"),
        ("LOOM_REQUEST_DRIVEN", "false"),
        ("LOOM_ROOT_ADMIN_TOKEN", ""),
        ("DATABASE_URL", ""),
        ("LOOM_ENCRYPTION_KEY", "existing"),
        ("LOOM_KEY_PEPPER", "existing"),
    ] {
        let mut e = environment();
        e.insert(key, value.into());
        assert!(reference(&|n| reader(&e, n)).is_err(), "{key}");
    }
    let mut e = environment();
    e.remove("LOOM_BOOTSTRAP_PARAMETER_ARN");
    assert!(reference(&|n| reader(&e, n)).is_err());
    assert!(reference(&|_| Err(VarError::NotUnicode("synthetic".into()))).is_err());
}

#[test]
fn valid_settings_keep_existing_crypto_and_explicit_verified_tls_then_clean_owned_ca() {
    use std::os::unix::fs::PermissionsExt;
    let e = environment();
    let read = |n| reader(&e, n);
    let r = reference(&read).unwrap().unwrap();
    let loaded = materialise(&r, &serde_json::to_vec(&response()).unwrap(), &read).unwrap();
    let config = loaded.config();
    assert!(config.request_driven);
    assert!(!config.run_migrations);
    assert!(config.batch_poll_interval.is_zero());
    assert_eq!(config.database_max_connections, 2);
    assert_eq!(config.encryption_key(), [2; 32]);
    assert_eq!(config.key_pepper(), crate::keys::derive_pepper(&[2; 32]));
    let url = Url::parse(&config.database_url).unwrap();
    let query = url.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(query.get("sslmode").unwrap(), "verify-full");
    let path = PathBuf::from(query.get("sslrootcert").unwrap().as_ref());
    assert_eq!(fs::read_to_string(&path).unwrap(), ca());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let diagnostic = format!("{config:?}");
    for secret in [
        "synthetic-database-password",
        &"a".repeat(64),
        &"02".repeat(32),
    ] {
        assert!(!diagnostic.contains(secret));
    }
    drop(loaded);
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
    let mut p = response();
    let mut d = document();
    d["key_pepper"] = json!({"mode":"explicit","value":"existing-explicit-pepper"});
    p["Parameter"]["Value"] = json!(d.to_string());
    let loaded = materialise(&r, &serde_json::to_vec(&p).unwrap(), &read).unwrap();
    assert_eq!(loaded.config().key_pepper(), b"existing-explicit-pepper");
}

#[test]
fn substituted_or_malformed_parameters_are_redacted_and_never_materialised() {
    let e = environment();
    let read = |n| reader(&e, n);
    let r = reference(&read).unwrap().unwrap();
    let mut bad = Vec::new();
    for (key, value) in [
        ("ARN", json!("other")),
        ("Name", json!("/other")),
        ("Type", json!("String")),
        ("Version", json!(8)),
        ("Selector", json!(":8")),
        ("Value", json!("not-json")),
    ] {
        let mut p = response();
        p["Parameter"][key] = value;
        bad.push(p);
    }
    for (key, value) in [
        ("database_url", json!("https://attacker.invalid/database")),
        (
            "database_url",
            json!(format!(
                "{}?sslmode=disable",
                document()["database_url"].as_str().unwrap()
            )),
        ),
        ("root_admin_token", json!("short")),
        ("encryption_key", json!("00")),
        ("database_ca_sha256", json!("0".repeat(64))),
        ("key_pepper", json!({"mode":"explicit","value":""})),
        ("key_pepper", json!({"mode":"unknown"})),
        ("unexpected", json!(true)),
    ] {
        let mut d = document();
        d[key] = value;
        let mut p = response();
        p["Parameter"]["Value"] = json!(d.to_string());
        bad.push(p);
    }
    let mut d = document();
    d.as_object_mut().unwrap().remove("key_pepper");
    let mut p = response();
    p["Parameter"]["Value"] = json!(d.to_string());
    bad.push(p);
    let mut p = response();
    p["Parameter"]["Value"] = json!(" ".repeat(4097));
    bad.push(p);
    for p in bad {
        match materialise(&r, &serde_json::to_vec(&p).unwrap(), &read) {
            Ok(_) => panic!("Invalid synthetic settings accepted"),
            Err(error) => {
                assert_eq!(error.to_string(), "private Lambda bootstrap unavailable");
                assert_eq!(format!("{error:?}"), "BootstrapError");
            }
        }
    }
    assert!(materialise(&r, &vec![b' '; LIMIT + 1], &read).is_err());
}

#[test]
fn exact_ssm_request_uses_shared_official_signer_and_sensitive_role_headers() {
    let mut e = environment();
    e.insert("AWS_ENDPOINT_URL_SSM", "https://attacker.invalid".into());
    let read = |n| reader(&e, n);
    let r = reference(&read).unwrap().unwrap();
    let (endpoint, headers, body) = request(&r, &read).unwrap();
    assert_eq!(endpoint.as_str(), "https://ssm.ap-southeast-2.api.aws/");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"Name":format!("{}:7",r.arn),"WithDecryption":true})
    );
    assert_eq!(headers["x-amz-target"], "AmazonSSM.GetParameter");
    assert_eq!(
        headers["x-amz-content-sha256"],
        hex::encode(Sha256::digest(&body))
    );
    assert!(headers["authorization"]
        .to_str()
        .unwrap()
        .contains("/ap-southeast-2/ssm/aws4_request"));
    assert!(headers["authorization"].is_sensitive());
    assert!(headers["x-amz-security-token"].is_sensitive());
    e.remove("AWS_SESSION_TOKEN");
    assert!(request(&r, &|n| reader(&e, n)).is_err());
}
