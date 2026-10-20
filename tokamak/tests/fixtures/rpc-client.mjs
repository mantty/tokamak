import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import net from 'node:net';
import path from 'node:path';
import tls from 'node:tls';
import { createClient } from '../../../plugins/core/src/rpc/client.mjs';

const require = createRequire(new URL('../../../tokamak-cli/npm/package.json', import.meta.url));
const WebSocket = require('ws');
const [port, state] = process.argv.slice(2);
const host = 'services.tokamak.local';
const connections = new Set();
const timer = setTimeout(() => { console.error('RPC test timed out'); process.exit(1); }, 10_000);

function connect(_options, connected) {
  const socket = net.connect({ host: '127.0.0.1', port: Number(port) });
  connections.add(socket);
  socket.once('close', () => connections.delete(socket));
  socket.once('error', connected);
  socket.once('connect', () => socket.write(`CONNECT ${host}:443 HTTP/1.1\r\nHost: ${host}:443\r\n\r\n`));
  let header = Buffer.alloc(0);
  function receive(chunk) {
    header = Buffer.concat([header, chunk]);
    const end = header.indexOf('\r\n\r\n');
    if (end < 0) return;
    socket.off('data', receive);
    socket.off('error', connected);
    if (!header.toString().startsWith('HTTP/1.1 200')) {
      socket.destroy();
      connected(new Error(header.toString()));
      return;
    }
    socket.pause();
    if (header.length > end + 4) socket.unshift(header.subarray(end + 4));
    const secure = tls.connect({
      socket, servername: host,
      ca: fs.readFileSync(path.join(state, 'ca.cert.pem')),
      cert: fs.readFileSync(path.join(state, 'client.cert.pem')),
      key: fs.readFileSync(path.join(state, 'client.key.pem')),
    });
    connected(null, secure);
  }
  socket.on('data', receive);
}

globalThis.WebSocket = class extends WebSocket {
  constructor(url) {
    super(url, { createConnection: connect, handshakeTimeout: 5000, origin: `https://${host}` });
  }
};

try {
  const client = createClient(['echo', 'stage', 'fail', 'invalid', 'bytes', 'ignore'], `https://${host}/_tokamak/app`);
  assert.equal(await client.stage(), 'foreground');
  const value = { count: 42n, bytes: new Uint8Array([8, 9]), map: new Map() };
  value.map.set(value, value);
  assert.deepEqual(await client.echo(value), value);
  await assert.rejects(client.fail(), { message: 'Denied', code: 'DENIED' });
  await assert.rejects(client.invalid(), { name: 'DataCloneError' });
  assert.deepEqual(new Uint8Array(await new Response(await client.bytes()).arrayBuffer()), new Uint8Array([1, 2, 3]));
  const cancelled = Promise.withResolvers();
  let pulls = 0;
  const source = new ReadableStream({
    type: 'bytes',
    pull(controller) { pulls++; controller.enqueue(new Uint8Array(100_000).fill(7)); },
    cancel(reason) { cancelled.resolve(reason); },
  });
  const stream = await client.echo(source);
  assert.equal(pulls, 0);
  const reader = stream.getReader();
  assert.equal((await reader.read()).value.byteLength, 64 * 1024);
  assert.equal(pulls, 1);
  await reader.cancel('finished');
  assert.equal(await cancelled.promise, 'finished');
  const ignored = Promise.withResolvers();
  assert.equal(await client.ignore(new ReadableStream({ type: 'bytes', cancel: ignored.resolve })), 'finished');
  assert.equal((await ignored.promise).name, 'AbortError');
} finally {
  clearTimeout(timer);
  for (const socket of connections) socket.destroy();
}
