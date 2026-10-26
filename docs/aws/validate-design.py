#!/usr/bin/env python3
"""Offline checks for the AWS design bundle; no cloud or provider credentials."""
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit, unquote

root = Path(__file__).resolve().parent
subprocess.run([sys.executable, str(root / 'cost-model.py'), '--check'], check=True)
for path in root.glob('*.md'):
    for target in re.findall(r'\]\(([^)]+)\)', path.read_text()):
        url = urlsplit(target)
        if url.scheme or url.netloc:
            assert url.scheme == 'https', (path, target)
            continue
        assert (path.parent / unquote(url.path)).resolve().exists(), (path, target)
inputs = json.loads((root / 'pricing-inputs.json').read_text())
assert inputs['currency'] == 'USD' and inputs['region'] == 'ap-southeast-2'
for key, value in inputs['rates'].items():
    assert urlsplit(value['source_url']).hostname in {'pricing.us-east-1.amazonaws.com', 'aws.amazon.com'}, key
    assert value.get('sku') or key == 'ipv4_hour', key
    assert value.get('effective_date') or value.get('retrieved_date'), key
api = json.loads((root / 'tandem-api-evidence.json').read_text())
assert api['tandem_http_api'] and len(api['checks']) == 19
assert all(re.fullmatch('[a-f0-9]{64}', digest) for digest in api['tandem_source_sha256'].values())
source = json.loads((root / 'source-evidence.json').read_text())
for item in source['sources'] + source['additional_sources']:
    assert re.fullmatch('[a-f0-9]{40}', item['revision']), item
print('AWS design: relative links, source refs, rate provenance, evidence shape and estimate passed.')
