/**
 * CloudFront Function source for HTTP Basic Auth gating, backed by a
 * CloudFront KeyValueStore. The KVS holds a single key `auth-token`
 * containing the pre-encoded `base64(user:password)` value. Credentials
 * are populated out-of-band (`aws cloudfront-keyvaluestore put-key`)
 * and never live in this code or in the CDK template.
 *
 * Runtime: cloudfront-js-2.0 — supports `import` and top-level `await`,
 * required for `cloudfront.kvs()` access.
 *
 * Closed-by-default: if the KVS lookup fails (typically because the key
 * has not been populated yet, e.g. immediately after first deploy), the
 * function returns 503 rather than allowing requests through.
 */
export function basicAuthFunctionCode(kvsId: string): string {
  return `
import cf from 'cloudfront';
const kvs = cf.kvs('${kvsId}');

async function handler(event) {
  var request = event.request;
  var headers = request.headers;
  var expected;
  try {
    var token = await kvs.get('auth-token');
    expected = 'Basic ' + token;
  } catch (e) {
    return {
      statusCode: 503,
      statusDescription: 'Service Unavailable',
      headers: { 'cache-control': { value: 'no-store' } },
      body: 'Auth not configured'
    };
  }
  if (!headers.authorization || headers.authorization.value !== expected) {
    return {
      statusCode: 401,
      statusDescription: 'Unauthorized',
      headers: { 'www-authenticate': { value: 'Basic realm="Staging"' } }
    };
  }
  return request;
}
`.trim();
}
