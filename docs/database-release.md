# Database release procedure

Loom can run migrations as a short-lived job before starting its runtime. This
allows the runtime database role to have application DML privileges without DDL
or migration-metadata write privileges. It does not provision an AWS deployment.

## Release sequence

1. Stop admission and quiesce writers for a coordinated release or restore.
   Take and verify the deployment's required backups before changing schema.
2. Run the **new release's exact binary/image** with the migration credential:
   `loom-server --migrate`. Supply `DATABASE_URL` through the secret-injection
   mechanism, never in command arguments or committed configuration.
3. Require exit status zero. Apply/reconcile runtime grants using the migration
   owner, then start the same release with its separate runtime credential and
   `LOOM_RUN_MIGRATIONS=false`.
4. Require `/readyz` to return 200 before admitting traffic. `/healthz` alone
   proves liveness, not database or release compatibility.

Migration mode reads only `DATABASE_URL` and optional
`LOOM_MIGRATION_TIMEOUT_SECS` (default 120, integer 1–900). It uses one connection,
with a 10-second acquisition timeout and a deadline covering the whole job. It
does not initialise telemetry, HTTP, background workers, encryption/admin
configuration or provider clients. Mixed/unknown command-line flags exit 2.
SQLx serialises migration runners with PostgreSQL advisory locking. Repeated
successful runs are idempotent. A job failure or timeout exits nonzero with a
generic message; credentials and connection diagnostics are not printed.

Completed migrations can remain applied when a later migration or the job fails.
Stop the release and inspect the private job/database context. Do not delete
migration metadata, automatically roll back schema or restore only one component
of a coordinated Loom/LucidBrain backup. Retry or recovery requires an explicit
operator decision using the same pinned release.

## Role and TLS contract

Provision dedicated roles and database through the deployment's credential
manager. The migration role owns the database/schema and migrated objects; the
runtime role is a non-superuser, has no role/database creation or replication
privilege, and is not a member of the owner role. Revoke database TEMP and schema
CREATE from PUBLIC. Check inherited grants: merely revoking a direct grant does
not remove privileges inherited through PUBLIC or another role.

Example grants below assume `loom_migrator`, `loom_runtime`, database `loom`,
schema `public`, and execution as the migration owner after successful migration.
They contain no password or credential bootstrap. Adapt them to existing grants;
do not run them blindly against a shared database.

```sql
REVOKE ALL ON DATABASE loom FROM PUBLIC;
GRANT CONNECT ON DATABASE loom TO loom_runtime;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO loom_runtime;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO loom_runtime;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO loom_runtime;
REVOKE ALL ON TABLE public._sqlx_migrations FROM loom_runtime;
GRANT SELECT ON TABLE public._sqlx_migrations TO loom_runtime;
```

Reconcile grants after **each** migration job so newly created tables receive
appropriate privileges. Do not grant runtime DML by default to migration metadata.
Use separate injected DSNs for the job and runtime. Production DSNs must use
`sslmode=verify-full` and `sslrootcert` pointing at a mounted trusted CA; the host
must match the certificate. Configure the server to reject plaintext connections.
The executable selects Rustls's workspace ring provider before PostgreSQL TLS
starts, including when transitive dependencies also enable aws-lc-rs.

Runtime `LOOM_DATABASE_MAX_CONNECTIONS` defaults to 10 and accepts integers 2–64.
Size the combined deployment's pools against the database and memory budget.
Private MCP action reservations use at most half this pool. The lower bound
preserves a connection for normal gateway work. The migration job always uses
one connection independently of this setting.

## Readiness and compatibility

Runtime startup verifies the exact ordered embedded migration versions,
checksums and success markers **before** constructing provider state, starting
workers or binding HTTP. `/readyz` repeats this SELECT-only check. Missing or
unreadable metadata, missing/extra versions, failed migrations or changed
checksums fail closed. An old binary therefore refuses a newer schema even if
the change might otherwise be backwards compatible; rollback needs a deliberate
release/restore decision. Startup migrations remain enabled by default for
existing development deployments, but the separate-role deployment must disable
them explicitly.

This check validates migration metadata; it does not detect arbitrary manual DDL
changes when metadata is left unchanged. Readiness is an admission signal, not a
per-request transaction barrier: quiesce existing traffic before modifying schema.

## Verification scope

`crates/loom-server/tests/migration_release.rs` starts the real executable against
a disposable PostgreSQL 16 container with a generated CA/server certificate and
separate non-superuser roles. It covers concurrent migration runners and lock
timeout, repeated migration without runtime configuration or an available HTTP
port, runtime DML without DDL/TEMP/metadata-write privileges, successful verified
TLS, rejection of a wrong CA/hostname and plaintext, missing/incompatible metadata,
readiness versus liveness, and graceful SIGTERM. Certificates are synthetic and
removed with the fixture. OpenSSL and Docker are required.

This is local database/process qualification. Linux ARM64 production-image
qualification, the full LucidBrain/pgvector migration chain, combined host capacity,
real Cognito provisioning, AWS deployment and physical iPhone/live voice testing
remain separate release gates.
