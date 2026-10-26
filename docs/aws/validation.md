# Reproducible validation and evidence plan

A release is not “AWS verified” because containers compile, an API test passes or a cost model balances. Record immutable source/images/IaC, input secret-version references (no values), DB engine/extension versions and observed results for each gate. All steps below are **required future validation**, except the explicitly recorded local checks.

## 1. Existing evidence, narrowly scoped

- [Tandem task API record](tandem-api-evidence.json): 19 combined checks through real HTTP/SQLite, Loom/PostgreSQL and actual LucidBrain MCP/PostgreSQL, three synthetic tasks, only Responses simulated. Report includes source SHA-256s, restarts, duplicate confirmation, lost reply, cancellation, credential replacement and deletion recovery. It does not test production schemas, actual Cognito/AWS routes or native controls.
- Source artifact also records 48 backend/infrastructure tests, TypeScript, Lambda bundle/CDK synthesis, and 14 private-memory joint checks. Existing draft CI is not a merged release or a fresh current-cloud inspection.
- Cost reproduction: `python3 docs/aws/cost-model.py --check`. Read rate sources/effective dates and quantity/allowance assumptions; no account login is used. This check proves arithmetic/provenance structure, not actual spend or platform availability.

## 2. Local production-image and database gate, no live AI

1. Export clean reviewed source and build exact ARM64 Linux images in CI. Verify image digest, architecture, non-root user, health command, CA bundle and absence of `.env`/test credentials/build-context leakage. Pin proxy and pgvector as well as app images.
2. Start PostgreSQL16 + pgvector from an empty persistent volume with production TLS and separate databases/roles. Run the exact Loom and LucidBrain production migration paths, not `tests/helpers/db.ts`. Exercise current-to-next upgrade, repeated migration, failed migration and competing migration lock. Verify runtime roles cannot alter schema, read the other DB or bootstrap extensions.
3. Verify SQLx and Node clients reject wrong CA and wrong hostname. Check effective pool limits, role grants, statement/idle timeouts, data directory ownership and encrypted-volume assumptions. Test two simultaneous user turns plus readiness/status/reconciliation without starvation.
4. Start actual production app entrypoints with unused managed-agent startup reconciliation disabled, no model/embedding key and no optional scheduled/batch worker. Check readiness and graceful SIGTERM/kill behavior. Measure combined RSS/cgroup peaks, disk/image size and CPU under realistic two-user/mock-provider load before claiming the 4 GB budget works.
5. Use a synthetic Responses endpoint with real MCP and production schema to repeat memory/task flow. This needs an explicitly test-only provider override and test deployment; never enable a loopback or arbitrary-provider override in production mobile configuration.

## 3. Read-only AWS prerequisites

Use a fresh `tandem-assistant-os` session. Record STS account/region and inspect existing stack/resources/tags before changes. Check EC2 ARM availability/quota, exact AMI/ECS agent support, EIP quota, Lambda quota, OIDC role boundary and DNS authority. Public regional prices show offerings, not account quota, chosen DB minor/extension support or spare host capacity. The expired session on 27 October was not a cloud-state verification.

Review a concrete costed IaC plan and resource diff with the selected availability window before activation. Assert retained data/EIP policies, exact secret ARNs/roles, no broad PassRole, public method/path/port allowlists and image/secret references. Design preparation does not authorise new spend by itself.

## 4. Real AWS synthetic smoke, no live AI

Deploy only after the infrastructure decision is reviewed. Record CloudFormation outputs, actual image digests and readiness; keep all probe identities/data synthetic. A test-only, private simulated inference endpoint may verify transport without using a live OpenAI key. It must never be reachable through an app-supplied URL.

- Unauthenticated/ID-token/wrong-scope requests fail; Cognito subject, not body fields, selects private tenant and workspace. Two users with identical session/request UUIDs cannot share history, actions, memory or keys.
- Approved user starts a conversation, recalls a permitted retained memory, sees a proposal, confirms exact details, receives one committed task receipt, and retrieves retained data in a fresh session. Attempt a disallowed workspace/tool/category and forged digest/identity/payload; reject before any write or inappropriate inference.
- Prove public 443 cannot reach `/admin`, `/mcp`, `/internal` or batches, including encoded-path/normalisation variants. Public 8080/8081/8082/8443/5432 and SSH remain denied. Test TLS/SNI and certificate renewal; test private MCP expected headers through the proxy. Inspect effective SG/IPv6/Docker firewall paths rather than trusting a diagram.
- Deliberately drop the response after a real capture commit, restart Loom/LucidBrain/Tandem, and reconcile the same original operation to its actual task ID. Explicit retry cannot duplicate it. Cancel before dispatch: zero tasks. Cancel after dispatch: actual committed/unknown result remains visible; do not falsely report no-effect.
- Interrupt during provider/tool work and after app/backend durable commits. Verify late output is fenced, unknown work does not automatically buy another inference, and conversation deletion refusal restores access to recovery. Include key refresh, replacement with read-only receipt permission, revocation and workspace reassignment.
- Test planned stop/start and unclean host kill using retained EBS, key/pepper and TLS material. No ASG termination or replacement task may silently wipe the ledger. Quiesce both application writers for one coordinated logical backup set, then trigger an isolated restore, compare memories/history/task/operation IDs and reconcile unknown operations before promotion. An independently timed pair of live database dumps is not that proof.
- Verify task drain/timeout, readiness, snapshot/backup retention and rollback. Check logs/traces contain no prompts, tool results, credentials or upstream error bodies. Test alarms and tagged infrastructure spend, and distinguish model charges. Alerts are not a hard cap.

## 5. Live provider and phone release gate, explicitly deferred

When Brett authorises private live credentials, run the same path against the **intended OpenAI provider**, without mock assertions standing in for actual behavior. Record selected available model, response/tool quality, time-to-first-visible-result, end-to-end latency, reported/billed usage differences, voice recognition/playback/interrupt and rendering behavior on the actual iPhone. Enable embeddings only as a separate deliberate test with verified vector migration/model dimensions.

Exercise the signed release candidate on cellular and after app/phone restart, token refresh and account switching. Show full proposal title/detail/category/expiry; only a committed receipt allows spoken success. Reconnect must preserve event order and original task identity, and a new conversation must not silently lose unresolved actions. Complete the user's five first-integration targets: conversation, permitted memory, one confirmed task with visible streamed result, fresh-session retention and server-side denial, plus tested failure/cancel/retry behavior.

Do not claim full OASP conformance until the selected real provider passes the pinned session/version/resources/vault/event/migration contract and the existing conformance/release work. Do not claim production character quality from the dressed preview or replace Riley with a generated stand-in. Those are separate original-goal requirements.
