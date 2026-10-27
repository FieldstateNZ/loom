# Tandem on AWS: serverless first, lowest practical idle cost

Brett clarified the requirement on **28 October 2026**: evaluate serverless application hosting first; retain PostgreSQL where needed and choose its cheapest practical host. A small VM for the database is acceptable. Reusing a continuously running application stack is not a fixed requirement.

**This supersedes the ECS-first recommendation.** The [old ECS design](ecs-pilot-reference.md), its USD 11.76/41.02 model and Tandem's later USD 7.46/25.54 personal variant are historical alternatives, not the default deployment or a current spending request. The prepared 24-addition change set must not be executed under the old hosting-choice question. No new AWS resource or spending approval is implied.

Read the [serverless application and database comparison](serverless-first.md), [reproducible estimates](serverless-cost-estimate.json), [rate evidence](serverless-pricing-inputs.json) and [next implementation work](backlog.md).

## Direction

- Keep Tandem's existing Cognito / API Gateway / Lambda / DynamoDB entry point and installed iPhone contract.
- Move request/response Loom, LucidBrain MCP and the bounded communication worker toward on-demand Lambda execution. Preserve their existing PostgreSQL state, user/workspace identities, explicit memory opt-ins and separately confirmed tasks.
- Compare **one shared PostgreSQL host with separate databases/roles**: small Lightsail/EC2 versus auto-pausing Aurora PostgreSQL. Database hosting is a separate decision from application compute.
- Cost networking, storage, backups and billed model-waiting time. Avoid fixed NAT gateways, load balancers, provisioned concurrency and keep-alive traffic in the baseline.
- Do not select a managed product just because its name says serverless, and do not assume compatibility from a container starting locally. Qualify request lifecycle, pools, private identity headers, interruption/recovery and actual idle behaviour.

This is a researched design revision, not an implemented or deployed Lambda runtime. Current local live conversation/memory proofs remain valid for the existing service flow. They do not qualify Lambda cold starts, freeze/thaw, IAM transport or a smaller database host. Review goes to IngeniousRiley; Brett tests the app experience and approves a concrete final cost proposal.

Reproduce the new arithmetic with `python3 docs/aws/serverless-cost-model.py --check`. `python3 docs/aws/validate-design.py` also preserves checks for the dated ECS evidence, without promoting it back to the selected architecture.
