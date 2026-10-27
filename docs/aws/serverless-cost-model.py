#!/usr/bin/env python3
"""Reproduce provisional serverless-first costs; no AWS calls or credentials."""
import argparse
from decimal import Decimal as D, ROUND_HALF_UP
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parent

def estimate():
 rates=json.loads((ROOT/'serverless-pricing-inputs.json').read_text())['rates']
 rate=lambda key:D(rates[key]['usd'])
 money=lambda x:str(x.quantize(D('.01'),rounding=ROUND_HALF_UP))
 result=[]
 for turns in [0,600,2400]:
  # Aggregated across all nested functions, including waiting for providers,
  # audio, cold starts and controls. This is a budget assumption, not a benchmark.
  gb_seconds=D(turns)*64
  invocations=D(turns)*12
  components={'lambda_duration':gb_seconds*rate('lambda_arm_gb_second'),
   'lambda_requests':invocations*rate('lambda_request'),
   'http_api_requests':invocations*rate('http_api_request'),
   'other_application_allowance':D('.50')}
  common=sum(components.values())
  databases=[('lightsail_512m_candidate',rate('lightsail_512m_month')+8*rate('lightsail_disk_gib_month'),None),
   ('lightsail_1g_candidate',rate('lightsail_1g_month')+8*rate('lightsail_disk_gib_month'),None),
   ('ec2_t4g_nano_candidate',730*rate('ec2_t4g_nano_hour')+16*rate('gp3_gib_month'),None),
   ('ec2_t4g_micro_candidate',730*rate('ec2_t4g_micro_hour')+16*rate('gp3_gib_month'),None)]
  for hours in ([0] if turns==0 else [20,40,100,730]):
   databases.append(('aurora_pausing_candidate',10*rate('aurora_pg_gib_month')+D('.5')*D(hours)*rate('aurora_pg_acu_hour'),hours))
  for name,base,hours in databases:
   total=common+base+D('.25')
   result.append({'candidate':name,'monthly_turns':turns,'assumed_total_lambda_gb_seconds':str(gb_seconds),'assumed_total_function_invocations':str(invocations),
    'aurora_awake_hours':hours,'database_base_usd':money(base),'application_components_usd':{k:money(v) for k,v in components.items()},
    'backup_and_database_io_allowance_usd':'.25','total_usd':money(total),'unrounded_total_usd':str(total)})
 return {'currency':'USD','region':'ap-southeast-2','decision':'provisional comparison; none deployed/qualified/approved','scenarios':result,
  'assumptions':['730 hours for EC2 calculations; Lightsail published monthly plan maximum.','One shared PostgreSQL instance, separate Loom/LucidBrain databases and roles.','Aurora Standard PostgreSQL ServerlessV2, 10 GiB, 0.5 ACU while awake, 0 when paused; 0.20 per ACU-hour.','Aurora awake hours include idle-tail, wake and maintenance time, not just model/request duration.','64 GB-seconds per user turn aggregated across all nested/waiting application functions; 12 invokes and 12 API calls per turn, not measured.','0.50 monthly application allowance: ECR .10, logical backups .05, logs .05, one alarm .10, DynamoDB/control .10, transfer .10.','0.25 monthly DB backup/I/O allowance is a planning quantity, not an AWS price guarantee.','No NAT gateway, load balancer, public IPv4, RDS Proxy, provisioned concurrency or periodic warmers. Networking/AMI/encryption compatibility remains unqualified.','Lightsail includes an additional 8 GiB encrypted independently retained data disk at 0.10/GiB-month; bundled disk remains the OS volume.','EC2 assumes a compatible 8 GiB non-ECS root + 8 GiB retained encrypted data; no CPU surplus-credit charges (standard mode).','Application idle Lambda compute is zero; retained storage/operational allowances persist.','No free tier, trial, credit, commitments or discounts included.'],
  'excluded':['Claude/OpenAI usage','tax','Labour and operations','Usage/storage/backup/I/O above the stated quantities','Additional networking or encrypted disk required by qualification'],
  'not_a_cap':True,'account_eligibility_verified':False,'runtime_qualified':False}

def main():
 p=argparse.ArgumentParser();p.add_argument('--write',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args()
 text=json.dumps(estimate(),indent=2)+'\n';path=ROOT/'serverless-cost-estimate.json'
 if a.write:path.write_text(text)
 if a.check:assert path.read_text()==text,'Serverless estimate drift; regenerate and review inputs.'
 if not a.write and not a.check:print(text,end='')
 if a.check:print('Serverless-first estimate reproduced; assumptions remain explicit.')
if __name__=='__main__':main()
