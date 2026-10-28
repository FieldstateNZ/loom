// Local Runtime API peer only. No provider, database or AWS service is used.
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash,createHmac,timingSafeEqual} from 'node:crypto';

const bootstrap=process.argv[2]==='--bootstrap'||process.argv[2]==='--bootstrap-reject',reject=process.argv[2]==='--bootstrap-reject';
let delivered = 0, databaseConnections = 0, reads=0;
const statuses = [], paths = ['/healthz', '/admin/tenants'];
const account = '123456789012';
const fail = () => { console.log(JSON.stringify({passed:false})); process.exit(1); };
const watchdog = setTimeout(fail, 30000);
net.createServer(socket => { databaseConnections++; socket.destroy(); fail(); })
  .listen(5432, '0.0.0.0');
if(bootstrap)https.createServer({key:readFileSync('/fixture-data/ssm.key'),cert:readFileSync('/fixture-data/ssm.crt')},async(req,res)=>{
 try{
  assert.equal(req.method,'POST');assert.equal(req.url,'/');assert.equal(++reads,1);
  const parts=[];let bytes=0;for await(const part of req){bytes+=part.length;assert.ok(bytes<2048);parts.push(part);}
  const body=Buffer.concat(parts),hash=x=>createHash('sha256').update(x).digest('hex'),hmac=(key,x)=>createHmac('sha256',key).update(x).digest();
  assert.deepEqual(JSON.parse(body.toString()),{Name:'arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/fixture/loom:7',WithDecryption:true});
  assert.equal(req.headers['x-amz-target'],'AmazonSSM.GetParameter');assert.equal(req.headers['x-amz-security-token'],'synthetic-bootstrap-session');
  const auth=/^AWS4-HMAC-SHA256 Credential=(ASIASYNTHETICEXAMPLE)\/(\d{8})\/ap-southeast-2\/ssm\/aws4_request, SignedHeaders=([a-z0-9;-]+), Signature=([a-f0-9]{64})$/.exec(req.headers.authorization??'');assert.ok(auth);
  const names=auth[3].split(';');assert.deepEqual(names,[...names].sort());assert.ok(names.includes('host')&&names.includes('x-amz-security-token'));
  const headers=names.map(name=>`${name}:${String(req.headers[name]).trim().replace(/\s+/g,' ')}\n`).join('');
  const canonical=['POST','/','',headers,auth[3],hash(body)].join('\n'),scope=`${auth[2]}/ap-southeast-2/ssm/aws4_request`;
  let key=hmac('AWS4synthetic-bootstrap-signing-key',auth[2]);for(const part of ['ap-southeast-2','ssm','aws4_request'])key=hmac(key,part);
  const signature=hmac(key,['AWS4-HMAC-SHA256',req.headers['x-amz-date'],scope,hash(canonical)].join('\n'));
  assert.ok(timingSafeEqual(signature,Buffer.from(auth[4],'hex')));
  res.writeHead(200,{'content-type':'application/x-amz-json-1.1'}).end(readFileSync('/fixture-data/parameter.json'));
  if(reject){clearTimeout(watchdog);console.log(JSON.stringify({passed:true,responses:0,statuses:[],database_connections:0,bootstrap_reads:reads,startup_rejected:true}));}
 }catch{fail();}
}).listen(443,'0.0.0.0');
http.createServer(async (req, res) => {
  try {
    if (req.method === 'GET' && req.url === '/2018-06-01/runtime/invocation/next') {
      assert.ok(!reject);if(bootstrap)assert.equal(reads,1);
      if (delivered >= paths.length) return; // Remain idle for the next event.
      const index = delivered++, path = paths[index];
      const event = {
        version:'2.0', routeKey:'$default', rawPath:path, rawQueryString:'',
        headers:{host:'synthetic.lambda-url.ap-southeast-2.on.aws'},
        requestContext:{accountId:account, domainName:'synthetic.lambda-url.ap-southeast-2.on.aws',
          authorizer:{iam:{accountId:account,userArn:`arn:aws:sts::${account}:assumed-role/tandem-test/session`}},
          http:{method:'GET',path,sourceIp:'127.0.0.1',protocol:'HTTP/1.1'}},
        isBase64Encoded:false,
      };
      res.writeHead(200, {
        'content-type':'application/json',
        'lambda-runtime-aws-request-id':`synthetic-${index}`,
        'lambda-runtime-deadline-ms':String(Date.now()+30000),
        'lambda-runtime-invoked-function-arn':`arn:aws:lambda:ap-southeast-2:${account}:function:synthetic`,
      }).end(JSON.stringify(event));
      return;
    }
    assert.equal(req.method, 'POST');
    assert.equal(req.url, `/2018-06-01/runtime/invocation/synthetic-${statuses.length}/response`);
    const parts = []; let bytes = 0;
    for await (const part of req) { bytes += part.length; assert.ok(bytes <= 262144); parts.push(part); }
    const result = JSON.parse(Buffer.concat(parts).toString());
    assert.equal(result.statusCode, statuses.length === 0 ? 200 : 404);
    if (statuses.length === 0) {
      const body = Buffer.from(result.body, result.isBase64Encoded ? 'base64' : 'utf8');
      assert.equal(body.toString(), 'ok');
    }
    statuses.push(result.statusCode);
    assert.equal(databaseConnections, 0);
    res.writeHead(202).end();
    if (statuses.length === paths.length) {
      clearTimeout(watchdog);
      console.log(JSON.stringify({passed:true,responses:statuses.length,statuses,database_connections:databaseConnections,bootstrap_reads:reads}));
    }
  } catch { fail(); }
}).listen(8080, '0.0.0.0', () => console.log('ready'));
