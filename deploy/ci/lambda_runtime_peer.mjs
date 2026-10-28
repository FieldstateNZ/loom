// Local Runtime API peer only. No provider, database or AWS service is used.
import http from 'node:http';
import net from 'node:net';
import assert from 'node:assert/strict';

let delivered = 0, databaseConnections = 0;
const statuses = [], paths = ['/healthz', '/admin/tenants'];
const account = '123456789012';
const fail = () => { console.log(JSON.stringify({passed:false})); process.exit(1); };
const watchdog = setTimeout(fail, 30000);
net.createServer(socket => { databaseConnections++; socket.destroy(); fail(); })
  .listen(5432, '0.0.0.0');
http.createServer(async (req, res) => {
  try {
    if (req.method === 'GET' && req.url === '/2018-06-01/runtime/invocation/next') {
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
      console.log(JSON.stringify({passed:true,responses:statuses.length,statuses,database_connections:databaseConnections}));
    }
  } catch { fail(); }
}).listen(8080, '0.0.0.0', () => console.log('ready'));
