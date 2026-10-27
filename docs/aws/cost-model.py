#!/usr/bin/env python3
"""Reproduce the design estimate from public rate snapshots; never calls AWS."""
import argparse
from decimal import Decimal, ROUND_HALF_UP
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
D = Decimal


def estimate():
    inputs = json.loads((ROOT / 'pricing-inputs.json').read_text())
    rates = inputs['rates']
    def rate(key):
        value = D(rates[key]['usd'])
        assert value >= 0
        return value
    def line(label, key, quantity, basis=None):
        amount = rate(key) * D(str(quantity))
        return {'item': label, 'rate': key, 'quantity': str(quantity),
                'usd': str(amount), **({'basis': basis} if basis else {})}
    def allowance(label, amount, basis):
        return {'item': label, 'rate': None, 'usd': str(D(amount)), 'basis': basis}
    def scenario(name, lines, **extra):
        total = sum((D(x['usd']) for x in lines), D(0))
        return {'name': name, 'items': lines, 'total_usd_unrounded': str(total),
                'total_usd': str(total.quantize(D('.01'), rounding=ROUND_HALF_UP)), **extra}
    common = [
        line('Encrypted gp3: 30 GB root + 16 GB data', 'gp3_gb_month', 46),
        line('One retained IPv4, including stopped hours', 'ipv4_hour', 730),
        line('One public DNS hosted zone', 'hosted_zone_month', 1),
        line('Private image retention', 'ecr_gb_month', 2),
        line('Retained compressed logical backups', 's3_gb_month', 2),
        line('Stored EBS snapshot data', 'snapshot_gb_month', 2),
        line('Standard log ingestion', 'logs_ingest_gb', '.1'),
        line('Log storage', 'logs_storage_gb_month', '.1'),
        line('Three standard alarm metrics', 'standard_alarm_month', 3),
        line('One custom backup/health metric', 'custom_metric_month', 1),
        allowance('Existing serverless entry/storage/identity', '.25',
                  'Planning allowance: 100 logical chat turns and <=5,000 total HTTP requests/month; small DynamoDB/PITR data. Not a quoted AWS rate or a cap.'),
        allowance('Control, DNS, KMS and backup requests', '.10', 'Small-pilot request allowance; no customer-managed KMS key monthly fee assumed.'),
        allowance('Provider/API internet transfer', '.12', '1 GB planning allowance; retained separate from OpenAI inference. Not an AWS transfer quote.'),
    ]
    scenarios = [scenario('scheduled_ecs_ec2_pilot', [line('4 GB t4g.medium Linux', 'ec2_t4g_medium_hour', 40)] + common,
                          availability='40 total running hours/month, including startup, backups and maintenance; unavailable between test windows'),
                 scenario('same_ecs_ec2_pilot_always_on', [line('4 GB t4g.medium Linux', 'ec2_t4g_medium_hour', 730)] + common,
                          availability='730 running hours/month; single-host, single-AZ, still not a highly available production tier')]
    managed = [
        line('Fargate ARM: Loom .25 + LucidBrain .5 + Tandem relay .25 vCPU', 'fargate_arm_vcpu_hour', 730),
        line('Fargate ARM: total 2 GB requested memory', 'fargate_arm_gb_hour', 2 * 730),
        line('RDS PostgreSQL db.t4g.small Multi-AZ', 'rds_multi_hour', 730),
        line('RDS Multi-AZ gp3 provisioned storage', 'rds_gp3_multi_gb_month', 20),
        line('Public Loom ALB + private MCP ALB', 'alb_hour', 2 * 730),
        line('Assumed average .1 LCU per ALB', 'alb_lcu_hour', D('.2') * 730),
        line('Two NAT gateways for private tasks, one per AZ', 'nat_hour', 2 * 730),
        line('NAT processing allowance: 10 GB', 'nat_gb', 10),
        line('Two public ALB addresses + two NAT addresses', 'ipv4_hour', 4 * 730),
        line('Public + private hosted zones', 'hosted_zone_month', 2),
        line('Private image retention', 'ecr_gb_month', 2),
        line('Additional logical backup retention', 's3_gb_month', 2),
        line('Standard log ingestion', 'logs_ingest_gb', 1),
        line('Log storage', 'logs_storage_gb_month', 1),
        line('Six standard alarm metrics', 'standard_alarm_month', 6),
        line('Three custom metrics', 'custom_metric_month', 3),
        allowance('Serverless entry/storage/identity', '.25', 'Same low-traffic workload as pilot; no model charges.'),
        allowance('Control/DNS/KMS/backup requests', '.10', 'No paid container insights, managed tracing, custom KMS key or private CA.'),
        allowance('Internet transfer', '.12', 'Same 1 GB planning allowance; extra cross-AZ/transfer charges remain variable.'),
    ]
    scenarios.append(scenario('managed_private_network_reference', managed,
        availability='Managed hosting reference, not an approved budget or production sizing proof. One application replica each; distributed controls gate horizontal scaling.'))
    return {'pricing_date': inputs['pricing_date'], 'currency': 'USD', 'region': inputs['region'],
            'month_hours': 730, 'scenarios': scenarios,
            'excluded': ['OpenAI inference and embeddings', 'taxes', 'new domain registration', 'character production/rendering/downloads',
                         'staff/operations time', 'unexpected data growth/usage above assumptions', 'optional paid security/telemetry services'],
            'notes': ['No free credits or commitment discounts applied.', 'This is an estimate, not a spend cap.',
                      'The scheduled host retains storage and IPv4 while stopped.', 'Resource/ARM/migration/TLS/restore qualification is a release gate, not established by this calculation.']}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    result = estimate()
    text = json.dumps(result, indent=2) + '\n'
    path = ROOT / 'cost-estimate.json'
    if args.write:
        path.write_text(text)
    if args.check:
        assert path.read_text() == text, 'Estimate drift: run cost-model.py --write and review assumptions.'
        values = result['scenarios']
        delta = D(values[1]['total_usd_unrounded']) - D(values[0]['total_usd_unrounded'])
        rate = D(json.loads((ROOT/'pricing-inputs.json').read_text())['rates']['ec2_t4g_medium_hour']['usd'])
        assert delta == D(690) * rate
        assert all(D(x['total_usd_unrounded']) == sum((D(i['usd']) for i in x['items']), D(0)) for x in values)
    for item in result['scenarios']:
        print(f"{item['name']}: USD {item['total_usd']}/month")


if __name__ == '__main__':
    main()
