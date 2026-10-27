//! Real binary + PostgreSQL16, TLS and separate synthetic database roles.
//! No provider credentials, live inference or cloud resources are used.
use sqlx::{migrate::Migrate, postgres::PgPoolOptions, PgPool};
use std::{fs, path::PathBuf, process::Stdio, time::Duration};
use testcontainers::{
    core::{IntoContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
    GenericImage, ImageExt,
};
use tokio::process::{Child, Command};
use uuid::Uuid;

struct Certificates(PathBuf);
impl Certificates {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("loom-release-tls-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("openssl")
                .current_dir(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("openssl available")
                .success())
        };
        for name in ["ca", "wrong"] {
            run(&[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "2",
                "-keyout",
                &format!("{name}.key"),
                "-out",
                &format!("{name}.crt"),
                "-subj",
                &format!("/CN=synthetic-{name}"),
                "-addext",
                "basicConstraints=critical,CA:TRUE",
            ]);
        }
        run(&[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "server.key",
            "-out",
            "server.csr",
            "-subj",
            "/CN=127.0.0.1",
        ]);
        fs::write(
            dir.join("server.ext"),
            "subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n",
        )
        .unwrap();
        run(&[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca.crt",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "server.crt",
            "-days",
            "2",
            "-extfile",
            "server.ext",
        ]);
        fs::write(dir.join("pg_hba.conf"),"local all all trust\nhostssl all all 0.0.0.0/0 scram-sha-256\nhostssl all all ::/0 scram-sha-256\nhostnossl all all 0.0.0.0/0 reject\nhostnossl all all ::/0 reject\n").unwrap();
        Self(dir)
    }
    // Keep certificate failure inputs explicit at each integration-test call.
    #[allow(clippy::too_many_arguments)]
    fn url(
        &self,
        port: u16,
        db: &str,
        user: &str,
        password: &str,
        host: &str,
        mode: &str,
        ca: &str,
    ) -> String {
        let mut url =
            reqwest::Url::parse(&format!("postgres://{user}:{password}@{host}:{port}/{db}"))
                .unwrap();
        url.query_pairs_mut()
            .append_pair("sslmode", mode)
            .append_pair("sslrootcert", self.0.join(ca).to_str().unwrap());
        url.to_string()
    }
}
impl Drop for Certificates {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn binary() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_loom-server"));
    c.env_clear()
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}
fn migrator(url: &str) -> Command {
    let mut c = binary();
    c.arg("--migrate").env("DATABASE_URL", url);
    c
}
async fn output(c: &mut Command) -> std::process::Output {
    tokio::time::timeout(Duration::from_secs(20), c.output())
        .await
        .expect("bounded binary process")
        .unwrap()
}
fn safe_output(out: &std::process::Output, secrets: &[&str]) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for secret in secrets {
        assert!(
            !text.contains(secret),
            "A synthetic secret was exposed in process output"
        );
    }
    text
}
async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(3))
        .connect(url)
        .await
        .unwrap()
}
fn runtime(url: &str, addr: &str) -> Command {
    let mut c = binary();
    c.env("DATABASE_URL", url)
        .env("LOOM_RUN_MIGRATIONS", "false")
        .env("LOOM_DATABASE_MAX_CONNECTIONS", "2")
        .env("LOOM_BIND_ADDR", addr)
        .env("LOOM_ROOT_ADMIN_TOKEN", "synthetic-runtime-admin")
        .env("LOOM_ENCRYPTION_KEY", "01".repeat(32))
        .env("LOOM_KEY_PEPPER", "synthetic-release-pepper")
        .env("LOOM_BATCH_POLL_INTERVAL_SECS", "0")
        .env("RUST_LOG", "warn");
    c
}
fn free_addr() -> String {
    let s = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    s.local_addr().unwrap().to_string()
}
async fn exited(child: Child) -> std::process::Output {
    tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .expect("process must exit")
        .unwrap()
}

#[tokio::test]
async fn migration_job_and_runtime_are_separated_with_verified_tls_and_schema_gates() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = Certificates::new();
    let pg=GenericImage::new("postgres","16").with_entrypoint("bash").with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stderr("database system is ready to accept connections"))
        .with_wait_for(WaitFor::message_on_stdout("database system is ready to accept connections"))
        .with_mount(Mount::bind_mount(certs.0.to_str().unwrap(),"/certs"))
        .with_env_var("POSTGRES_PASSWORD","postgres")
        .with_cmd(["-c","mkdir -p /tmp/loom-tls && cp /certs/server.crt /certs/server.key /certs/pg_hba.conf /tmp/loom-tls/ && chmod 600 /tmp/loom-tls/server.key && chown -R postgres:postgres /tmp/loom-tls && exec /usr/local/bin/docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/loom-tls/server.crt -c ssl_key_file=/tmp/loom-tls/server.key -c hba_file=/tmp/loom-tls/pg_hba.conf"])
        .start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let admin_url = certs.url(
        port,
        "postgres",
        "postgres",
        "postgres",
        "127.0.0.1",
        "verify-full",
        "ca.crt",
    );
    // Only fixture startup is retried: no migration or application write has run.
    let admin = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(p) = PgPoolOptions::new()
                .max_connections(2)
                .connect(&admin_url)
                .await
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("TLS database ready");
    let suffix = Uuid::new_v4().simple().to_string();
    let db = format!("release_{suffix}");
    let owner = format!("migrate_{suffix}");
    let actor = format!("runtime_{suffix}");
    let owner_password = Uuid::new_v4().simple().to_string();
    let actor_password = Uuid::new_v4().simple().to_string();
    for (role, password) in [(&owner, &owner_password), (&actor, &actor_password)] {
        sqlx::query(&format!("CREATE ROLE {role} LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION")).execute(&admin).await.unwrap();
    }
    sqlx::query(&format!("CREATE DATABASE {db} OWNER {owner}"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(&format!("REVOKE ALL ON DATABASE {db} FROM PUBLIC"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(&format!("GRANT CONNECT ON DATABASE {db} TO {actor}"))
        .execute(&admin)
        .await
        .unwrap();
    let owner_url = certs.url(
        port,
        &db,
        &owner,
        &owner_password,
        "127.0.0.1",
        "verify-full",
        "ca.crt",
    );
    let actor_url = certs.url(
        port,
        &db,
        &actor,
        &actor_password,
        "127.0.0.1",
        "verify-full",
        "ca.crt",
    );
    let owner_pool = connect(&owner_url).await;
    sqlx::query("REVOKE ALL ON SCHEMA public FROM PUBLIC")
        .execute(&owner_pool)
        .await
        .unwrap();
    sqlx::query(&format!("GRANT USAGE ON SCHEMA public TO {actor}"))
        .execute(&owner_pool)
        .await
        .unwrap();
    let actor_pool = connect(&actor_url).await;
    let secrets = [owner_password.as_str(), actor_password.as_str()];
    let tls: bool = sqlx::query_scalar("SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()")
        .fetch_one(&actor_pool)
        .await
        .unwrap();
    assert!(tls);
    let can_create: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user,'public','CREATE')")
            .fetch_one(&actor_pool)
            .await
            .unwrap();
    assert!(!can_create);
    let no_permission = output(&mut migrator(&actor_url)).await;
    assert!(!no_permission.status.success());
    safe_output(&no_permission, &secrets);
    let no_schema = output(&mut runtime(&actor_url, &free_addr())).await;
    assert!(!no_schema.status.success());
    assert!(safe_output(&no_schema, &secrets).contains("failed to start"));

    // The SQLx migration lock blocks both jobs. A timed-out job must not later
    // apply anything, and concurrent explicit jobs safely serialize afterward.
    let mut lock = owner_pool.acquire().await.unwrap();
    lock.lock().await.unwrap();
    let timed_out = output(migrator(&owner_url).env("LOOM_MIGRATION_TIMEOUT_SECS", "1")).await;
    assert!(!timed_out.status.success());
    safe_output(&timed_out, &secrets);
    let mut a = migrator(&owner_url).spawn().unwrap();
    let mut b = migrator(&owner_url).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(a.try_wait().unwrap().is_none());
    assert!(b.try_wait().unwrap().is_none());
    let absent: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations')::text")
            .fetch_one(&actor_pool)
            .await
            .unwrap();
    assert!(absent.is_none());
    lock.unlock().await.unwrap();
    drop(lock);
    for child in [a, b] {
        let result = exited(child).await;
        let text = safe_output(&result, &secrets);
        assert!(result.status.success(), "{text}");
    }
    loom_store::verify_migrations(&owner_pool).await.unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let repeated = output(
        migrator(&owner_url)
            .env("LOOM_BIND_ADDR", reserved.local_addr().unwrap().to_string())
            .env("LOOM_ENCRYPTION_KEY", "invalid-runtime-key")
            .env("LOOM_DATABASE_MAX_CONNECTIONS", "invalid-runtime-pool"),
    )
    .await;
    assert!(
        repeated.status.success(),
        "{}",
        safe_output(&repeated, &secrets)
    );
    // Normal runtime does not receive owner/DDL permissions. Metadata is SELECT-only.
    for sql in [
        format!("GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO {actor}"),
        format!("GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO {actor}"),
        format!("REVOKE ALL ON _sqlx_migrations FROM {actor}"),
        format!("GRANT SELECT ON _sqlx_migrations TO {actor}"),
    ] {
        sqlx::query(&sql).execute(&owner_pool).await.unwrap();
    }
    for sql in [
        "CREATE TABLE forbidden_runtime_ddl(id integer)",
        "CREATE TEMP TABLE forbidden_temp_ddl(id integer)",
        "UPDATE _sqlx_migrations SET success=false",
    ] {
        let error = sqlx::query(sql).execute(&actor_pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    loom_store::verify_migrations(&actor_pool).await.unwrap();
    let wrong_ca = output(&mut migrator(&certs.url(
        port,
        &db,
        &owner,
        &owner_password,
        "127.0.0.1",
        "verify-full",
        "wrong.crt",
    )))
    .await;
    assert!(!wrong_ca.status.success());
    safe_output(&wrong_ca, &secrets);
    // Establish reachability independently, then check the TLS error itself.
    // Do not depend on a weaker SSL mode's backend-specific semantics.
    let reachable = tokio::net::TcpStream::connect(("localhost", port))
        .await
        .unwrap();
    drop(reachable);
    let invalid_name = PgPoolOptions::new()
        .max_connections(1)
        .connect(&certs.url(
            port,
            &db,
            &owner,
            &owner_password,
            "localhost",
            "verify-full",
            "ca.crt",
        ))
        .await
        .unwrap_err();
    assert!(
        format!("{invalid_name:?}").contains("NotValidForName"),
        "TLS hostname error: {invalid_name:?}"
    );
    let wrong_host = output(&mut migrator(&certs.url(
        port,
        &db,
        &owner,
        &owner_password,
        "localhost",
        "verify-full",
        "ca.crt",
    )))
    .await;
    assert!(!wrong_host.status.success());
    safe_output(&wrong_host, &secrets);
    let plaintext = output(&mut migrator(&certs.url(
        port,
        &db,
        &owner,
        &owner_password,
        "127.0.0.1",
        "disable",
        "ca.crt",
    )))
    .await;
    assert!(!plaintext.status.success());
    safe_output(&plaintext, &secrets);

    let addr = free_addr();
    let mut server = runtime(&actor_url, &addr).spawn().unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://{addr}");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(
                server.try_wait().unwrap().is_none(),
                "Runtime exited before readiness"
            );
            if client
                .get(format!("{base}/readyz"))
                .send()
                .await
                .is_ok_and(|r| r.status() == 200)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let created = client
        .post(format!("{base}/admin/tenants"))
        .bearer_auth("synthetic-runtime-admin")
        .json(&serde_json::json!({"slug":"release-runtime-write","name":"Synthetic release"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenants WHERE slug='release-runtime-write'")
            .fetch_one(&actor_pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let (version, checksum): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations ORDER BY version DESC LIMIT 1",
    )
    .fetch_one(&owner_pool)
    .await
    .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=$2")
        .bind(vec![0u8])
        .bind(version)
        .execute(&owner_pool)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/readyz"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mismatch = output(&mut runtime(&actor_url, &free_addr())).await;
    assert!(!mismatch.status.success());
    safe_output(&mismatch, &secrets);
    sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=$2")
        .bind(checksum)
        .bind(version)
        .execute(&owner_pool)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET success=false WHERE version=$1")
        .bind(version)
        .execute(&owner_pool)
        .await
        .unwrap();
    assert!(loom_store::verify_migrations(&actor_pool).await.is_err());
    sqlx::query("UPDATE _sqlx_migrations SET success=true WHERE version=$1")
        .bind(version)
        .execute(&owner_pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,checksum,success,execution_time) VALUES(20990101000000,'unknown release',$1,true,0)").bind(vec![1u8]).execute(&owner_pool).await.unwrap();
    assert!(loom_store::verify_migrations(&actor_pool).await.is_err());
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=20990101000000")
        .execute(&owner_pool)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/readyz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    #[cfg(unix)]
    {
        assert!(std::process::Command::new("kill")
            .args(["-TERM", &server.id().unwrap().to_string()])
            .status()
            .unwrap()
            .success());
        let stopped = exited(server).await;
        assert!(stopped.status.success());
        safe_output(&stopped, &secrets);
    }
    #[cfg(not(unix))]
    {
        server.kill().await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tenants WHERE slug='release-runtime-write'"
        )
        .fetch_one(&actor_pool)
        .await
        .unwrap(),
        1
    );
    actor_pool.close().await;
    owner_pool.close().await;
    admin.close().await;
}
