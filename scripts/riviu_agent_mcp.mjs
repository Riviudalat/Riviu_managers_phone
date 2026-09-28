#!/usr/bin/env node
// Every tool uses authenticated Riviu admission and the existing manual owner.
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
const require = createRequire(resolve(fileURLToPath(new URL('../apps/desktop/package.json', import.meta.url))));
const { Server } = require('@modelcontextprotocol/sdk/server/index.js');
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');
const { ListToolsRequestSchema, CallToolRequestSchema } = require('@modelcontextprotocol/sdk/types.js');
const token = process.env.RIVIU_API_TOKEN;
if (!token) throw Error('Set RIVIU_API_TOKEN from Riviu API settings');
const url = new URL(process.env.RIVIU_API_URL ?? 'http://127.0.0.1:22222');
if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
    || url.username || url.password || url.search || url.hash || url.pathname !== '/') {
  throw Error('Riviu agent endpoint must be a local HTTP origin');
}
const server = new Server({ name: 'riviu-agent', version: '2.0.0' }, { capabilities: { tools: {} } });
const string = { type: 'string' };
const object = (properties, required = []) => ({ type: 'object', properties, required, additionalProperties: false });
const locator = object({ role: string, name: string, text: string, id: string, exact: { type: 'boolean', default: true } });
const scope = object({ package: string, root: locator });
const fields = object(Object.fromEntries(['identity', 'semantics', 'states', 'bounds', 'rawAttributes'].map(k => [k, { type: 'boolean', default: true }])));
const expected = { oneOf: [
  object({ kind: { enum: ['exists', 'absent', 'visible', 'hidden', 'enabled', 'disabled'] } }, ['kind']),
  object({ kind: { enum: ['selected', 'checked', 'focused'] }, value: { type: 'boolean' } }, ['kind', 'value']),
  object({ kind: { enum: ['value', 'text'] }, value: string, exact: { type: 'boolean', default: true } }, ['kind', 'value']),
] };
const ancestor = object({ maxDepth: { type: 'integer', minimum: 1 }, resourceId: string, resourceIdSuffix: string, className: string }, ['maxDepth']);
const legacySelector = object({ package: string, text: string, description: string, resourceId: string, className: string,
  schemaVersion: { const: 2 }, textPrefix: string, descriptionPrefix: string, scope: ancestor,
  actionTarget: object({ kind: { const: 'clickableAncestor' }, ancestor }, ['kind', 'ancestor']),
}, ['package']);
const operations = {
  begin: ['Begin caller-owned manual session; reject busy devices. Idle release and ref expiry: 60s.', [], {}],
  end: ['Gracefully end this session and invalidate refs.', [], {}],
  observe: ['Read semantic nodes, unknown states and opaque refs. Each observation replaces old refs.', [], { query: locator, scope, fields }],
  tap: ['Tap unique fresh ref only when existing navigation engine approves navigationTarget. Post/Send require existing engines; refs never grant public-effect authority.', ['ref', 'navigationTarget'], { ref: string, navigationTarget: string }],
  type: ['Replace text in already focused unique non-password textbox; exact value/focus readback. No implicit focus, newline or submit.', ['ref', 'text'], { ref: string, text: string }],
  swipe: ['Swipe within freshly measured bounds of a unique scrollable ref. No raw coordinates.', ['ref', 'direction'], { ref: string, direction: { enum: ['up', 'down', 'left', 'right'] } }],
  press: ['Back or Home through existing session; invalidates refs. ACK is not verification.', ['key'], { key: { enum: ['back', 'home'] } }],
  wait_for: ['Core read-only wait. Status: satisfied/deadlineExceeded/cancelled/unsupported. Verdict: satisfied/notSatisfied/unknown/ambiguous.', ['expected'], { query: locator, scope, fields, expected }],
  expect: ['One fresh read; unknown or ambiguous never satisfies an expectation.', ['expected'], { query: locator, scope, fields, expected }],
  screenshot: ['Semantic observation then screenshot sequentially, explicitly not atomic; invalidates refs.', [], {}],
};
const tools = [
  { name: 'riviu_devices', description: 'List devices managed by Riviu', inputSchema: object({}) },
  ...['observe', 'tap', 'record', 'recording'].map(operation => ({
    name: 'riviu_' + operation, description: 'Legacy Inspector ' + operation + '; use riviu_v2_* for semantic sessions.',
    inputSchema: object({ udid: string,
      ...(operation === 'tap' ? { selector: legacySelector } : {}),
      ...(operation === 'record' ? { active: { type: 'boolean' }, name: string } : {}),
    }, ['udid', ...(operation === 'tap' ? ['selector'] : operation === 'record' ? ['active'] : [])]),
  })),
  ...Object.entries(operations).map(([operation, [description, required, properties]]) => ({
    name: 'riviu_v2_' + operation,
    description: description + ' Errors preserve code/status; never automatically retry uncertain actions.',
    inputSchema: object({ udid: string, ...(operation === 'begin' ? {} : { sessionToken: string }),
      timeoutMs: { type: 'integer', minimum: 1, maximum: 120000, default: 10000 }, ...properties,
    }, ['udid', ...(operation === 'begin' ? [] : ['sessionToken']), ...required]),
  })),
];
server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools }));
server.setRequestHandler(CallToolRequestSchema, async ({ params }) => {
  if (!tools.some(t => t.name === params.name)) throw Error('Unknown tool');
  const args = params.arguments ?? {};
  const semantic = params.name.startsWith('riviu_v2_');
  const operation = params.name.slice(semantic ? 9 : 6);
  const devices = operation === 'devices';
  const timeoutMs = semantic ? (args.timeoutMs ?? 10000) : 10000;
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120000) throw Error('timeoutMs must be 1..120000');
  try {
    const response = await fetch(url.origin + (devices ? '/v1/devices' : semantic ? '/v2/inspector' : '/v1/inspector/' + operation), {
      method: devices ? 'GET' : 'POST', redirect: 'error',
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      ...(devices ? {} : { body: JSON.stringify(semantic ? { ...args, operation } : args) }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const result = await response.json();
    const value = result.result ?? result;
    const content = [];
    if (value.pngBase64) {
      content.push({ type: 'image', data: value.pngBase64, mimeType: 'image/png' });
      delete value.pngBase64;
    }
    content.push({ type: 'text', text: JSON.stringify({ httpStatus: response.status, result: value }) });
    return { content, isError: !response.ok || ['deadlineExceeded', 'cancelled', 'unsupported'].includes(value.status)
      || (value.verdict !== undefined && value.verdict !== 'satisfied') };
  } catch {
    // Bodies and exception strings may contain credentials or typed text.
    return { content: [{ type: 'text', text: JSON.stringify({ code: 'InspectorTransportUncertain',
      message: 'No usable result; do not automatically retry a mutation.' }) }], isError: true };
  }
});
await server.connect(new StdioServerTransport());
